//! Rewriting import address tables.
//!
//! Every module that calls `CreateFileW` does so through a slot in its own
//! import table that the loader filled with the real address. Pointing that
//! slot at a hook redirects the module's calls and nothing else, which is
//! why the hook DLL's own calls — made through its own, unpatched table —
//! reach the real functions without any re-entrancy guard.
//!
//! The PE structures are read by offset: the layout has been fixed since
//! PE32+ was introduced, and reading it directly keeps this module free of
//! anything that could differ between bindings versions.

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows_sys::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleBaseNameW};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// One function to redirect: its export name and the hook's address.
pub struct Patch {
    pub name: &'static [u8],
    pub hook: usize,
}

/// One module a pass rewrote, for the log.
pub struct Patched {
    pub base: usize,
    pub name: String,
    pub slots: usize,
}

/// Modules already rewritten, by base address, so a module is never patched
/// twice and a later load only costs the new modules.
static PATCHED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

/// Import table entries that can lead to the file API. `kernel32` is what
/// older code links against; the C runtime and newer code import the same
/// functions through the `api-ms-win-core-*` contract sets.
fn is_file_api_module(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("kernel32")
        || name.starts_with("kernelbase")
        || name.starts_with("api-ms-win-core-file")
        || name.starts_with("api-ms-win-core-libraryloader")
}

/// Every module loaded in this process.
pub fn modules() -> Vec<HMODULE> {
    let mut modules: Vec<HMODULE> = vec![std::ptr::null_mut(); 1024];
    let mut needed = 0u32;
    loop {
        let bytes = (modules.len() * std::mem::size_of::<HMODULE>()) as u32;
        // SAFETY: the buffer is `bytes` long and owned by us.
        let ok = unsafe {
            K32EnumProcessModules(
                GetCurrentProcess(),
                modules.as_mut_ptr(),
                bytes,
                &mut needed,
            )
        };
        if ok == 0 {
            return Vec::new();
        }
        let count = needed as usize / std::mem::size_of::<HMODULE>();
        if count <= modules.len() {
            modules.truncate(count);
            return modules;
        }
        modules.resize(count, std::ptr::null_mut());
    }
}

/// Patch every loaded module except those in `skip`, once each. Returns
/// the modules looked at for the first time, with how many slots each had.
pub fn patch_all(patches: &[Patch], skip: &[HMODULE]) -> Vec<Patched> {
    let Ok(mut guard) = PATCHED.lock() else {
        return Vec::new();
    };
    let patched = guard.get_or_insert_with(HashSet::new);
    let reals = real_addresses(patches);
    let mut out = Vec::new();
    for module in modules() {
        let base = module as usize;
        if skip.contains(&module) || !patched.insert(base) {
            continue;
        }
        // SAFETY: `module` is the base of an image the loader mapped and keeps
        // mapped while it is listed; it was listed a moment ago.
        let slots = unsafe { patch_module(base, patches, &reals) };
        out.push(Patched {
            base,
            name: module_name(module),
            slots,
        });
    }
    out
}

/// Where each patched function really is, as `(patch index, address)`:
/// `kernel32`'s export and `kernelbase`'s, since an import slot holds
/// whichever the loader resolved it to.
fn real_addresses(patches: &[Patch]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for dll in ["kernel32.dll", "kernelbase.dll"] {
        let wide: Vec<u16> = dll.encode_utf16().chain(Some(0)).collect();
        // SAFETY: plain lookups of modules every process has loaded.
        let module = unsafe { GetModuleHandleW(wide.as_ptr()) };
        if module.is_null() {
            continue;
        }
        for (index, patch) in patches.iter().enumerate() {
            let mut name = patch.name.to_vec();
            name.push(0);
            if let Some(address) = unsafe { GetProcAddress(module, name.as_ptr()) } {
                out.push((index, address as usize));
            }
        }
    }
    out
}

fn module_name(module: HMODULE) -> String {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer's length is passed with it.
    let len = unsafe {
        K32GetModuleBaseNameW(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            module,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
}

/// Rewrite one module's imports. Returns how many slots were changed.
///
/// Slots are matched by the imported name where the module still carries
/// its name table, and otherwise by the address the loader put in them: an
/// image whose name table was stripped, or that imports a function by
/// ordinal or through a contract set not listed here, calls the very same
/// address, and that address is what gives it away.
///
/// # Safety
/// `base` must be the base address of a mapped PE32+ image.
unsafe fn patch_module(base: usize, patches: &[Patch], reals: &[(usize, usize)]) -> usize {
    let read_u16 = |at: usize| unsafe { std::ptr::read_unaligned(at as *const u16) };
    let read_u32 = |at: usize| unsafe { std::ptr::read_unaligned(at as *const u32) };

    if read_u16(base) != 0x5A4D {
        return 0; // "MZ"
    }
    let nt = base + read_u32(base + 0x3C) as usize;
    if read_u32(nt) != 0x0000_4550 {
        return 0; // "PE\0\0"
    }
    let optional = nt + 24;
    if read_u16(optional) != 0x20B {
        return 0; // not PE32+
    }
    // DataDirectory[1] (imports) sits 120 bytes into the PE32+ optional header.
    let imports_rva = read_u32(optional + 120) as usize;
    let imports_size = read_u32(optional + 124) as usize;
    if imports_rva == 0 || imports_size == 0 {
        return 0;
    }

    let mut changed = 0;
    let mut descriptor = base + imports_rva;
    loop {
        let original_first_thunk = read_u32(descriptor) as usize;
        let name_rva = read_u32(descriptor + 12) as usize;
        let first_thunk = read_u32(descriptor + 16) as usize;
        if name_rva == 0 && first_thunk == 0 {
            break;
        }
        descriptor += 20;
        if first_thunk == 0 {
            continue;
        }
        let names_known =
            original_first_thunk != 0 && is_file_api_module(&c_string(base + name_rva));

        // The names come from the original thunks; the addresses the module
        // actually calls through are in the first thunks, at the same index.
        let mut index = 0;
        loop {
            let slot = (base + first_thunk + index * 8) as *mut usize;
            let current = std::ptr::read_volatile(slot);
            let lookup = if original_first_thunk != 0 {
                std::ptr::read_unaligned((base + original_first_thunk + index * 8) as *const u64)
            } else {
                current as u64
            };
            if lookup == 0 {
                break;
            }
            index += 1;

            let by_name = (names_known && lookup & (1 << 63) == 0)
                .then(|| {
                    let name_at = base + (lookup as usize & 0x7FFF_FFFF) + 2; // skip the hint
                    let name = c_bytes(name_at);
                    patches.iter().find(|p| p.name == name)
                })
                .flatten();
            let patch = by_name.or_else(|| {
                reals
                    .iter()
                    .find(|(_, address)| *address == current)
                    .map(|(at, _)| &patches[*at])
            });
            let Some(patch) = patch else {
                continue;
            };
            if current != patch.hook && write_slot(slot, patch.hook) {
                changed += 1;
            }
        }
    }
    changed
}

/// Write one import slot, lifting the page protection around it.
///
/// # Safety
/// `slot` must point into a loaded module's import address table.
unsafe fn write_slot(slot: *mut usize, value: usize) -> bool {
    let mut previous: PAGE_PROTECTION_FLAGS = 0;
    let size = std::mem::size_of::<usize>();
    if VirtualProtect(slot as *const c_void, size, PAGE_READWRITE, &mut previous) == 0 {
        return false;
    }
    std::ptr::write_volatile(slot, value);
    let mut ignored: PAGE_PROTECTION_FLAGS = 0;
    VirtualProtect(slot as *const c_void, size, previous, &mut ignored);
    true
}

/// A NUL-terminated byte string inside an image, without the terminator.
///
/// # Safety
/// `at` must point to a NUL-terminated string in mapped memory.
unsafe fn c_bytes<'a>(at: usize) -> &'a [u8] {
    let start = at as *const u8;
    let mut len = 0;
    // Import names are short; the cap only stops a damaged image from
    // walking the whole address space.
    while len < 512 && *start.add(len) != 0 {
        len += 1;
    }
    std::slice::from_raw_parts(start, len)
}

unsafe fn c_string(at: usize) -> String {
    String::from_utf8_lossy(c_bytes(at)).into_owned()
}
