//! The hooked file functions.
//!
//! Each hook does the same three things: turn the path it was given into a
//! full path, ask the overlay where that path really is, and call the real
//! function with the answer. Anything outside the game folder, and anything
//! the overlay does not know, goes through untouched — the cost for the
//! game's own files is one lookup.
//!
//! The rules themselves live in `sbmm_vfs::Vfs::redirect` and are tested
//! there; this module is only the translation to and from Win32.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};

use sbmm_vfs::{pattern, Access, Mapping, Redirect, VPath, Vfs};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{
    SetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, HMODULE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFullPathNameW, CREATE_ALWAYS, CREATE_NEW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES,
    OPEN_ALWAYS, WIN32_FILE_ATTRIBUTE_DATA, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetModuleHandleW, GetProcAddress, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

use crate::iat::{self, Patch};

type CreateFileW =
    unsafe extern "system" fn(*const u16, u32, u32, *const c_void, u32, u32, HANDLE) -> HANDLE;
type GetFileAttributesW = unsafe extern "system" fn(*const u16) -> u32;
type GetFileAttributesExW = unsafe extern "system" fn(*const u16, i32, *mut c_void) -> BOOL;
type FindFirstFileW = unsafe extern "system" fn(*const u16, *mut WIN32_FIND_DATAW) -> HANDLE;
type FindFirstFileExW =
    unsafe extern "system" fn(*const u16, i32, *mut c_void, i32, *const c_void, u32) -> HANDLE;
type FindNextFileW = unsafe extern "system" fn(HANDLE, *mut WIN32_FIND_DATAW) -> BOOL;
type FindClose = unsafe extern "system" fn(HANDLE) -> BOOL;
type CreateDirectoryW = unsafe extern "system" fn(*const u16, *const c_void) -> BOOL;
type MoveFileExW = unsafe extern "system" fn(*const u16, *const u16, u32) -> BOOL;
type DeleteFileW = unsafe extern "system" fn(*const u16) -> BOOL;
type LoadLibraryW = unsafe extern "system" fn(*const u16) -> HMODULE;
type LoadLibraryExW = unsafe extern "system" fn(*const u16, HANDLE, u32) -> HMODULE;

/// The real functions, looked up once in `kernelbase`, where Windows keeps
/// the implementations `kernel32` forwards to.
struct Real {
    create_file: CreateFileW,
    get_attributes: GetFileAttributesW,
    get_attributes_ex: GetFileAttributesExW,
    find_first_ex: FindFirstFileExW,
    find_next: FindNextFileW,
    find_close: FindClose,
    create_directory: CreateDirectoryW,
    move_file_ex: MoveFileExW,
    delete_file: DeleteFileW,
    load_library: LoadLibraryW,
    load_library_ex: LoadLibraryExW,
}

static REAL: OnceLock<Real> = OnceLock::new();
static VFS: OnceLock<RwLock<Vfs>> = OnceLock::new();
/// Directory searches the overlay answered itself, by the handle it gave out.
static SEARCHES: Mutex<Option<HashMap<usize, Box<Search>>>> = Mutex::new(None);
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

struct Search {
    entries: Vec<WIN32_FIND_DATAW>,
    next: usize,
}

/// Exit codes of [`sbmm_vfs_init`], which the launcher reports.
const OK: u32 = 0;
const NO_MAPPING_VARIABLE: u32 = 1;
const MAPPING_UNREADABLE: u32 = 2;
const REAL_FUNCTIONS_MISSING: u32 = 3;

/// Set up the overlay and install the hooks.
///
/// Called by the launcher on a thread of its own while the game's main thread
/// is still suspended, so nothing races the patching. The mapping's path
/// comes from `SBMM_VFS_MAPPING` in the game's environment.
#[no_mangle]
pub extern "system" fn sbmm_vfs_init(_parameter: *mut c_void) -> u32 {
    if let Some(path) = std::env::var_os("SBMM_VFS_LOG") {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            if let Ok(mut log) = LOG.lock() {
                *log = Some(file);
            }
        }
    }

    let Some(mapping_path) = std::env::var_os("SBMM_VFS_MAPPING") else {
        return NO_MAPPING_VARIABLE;
    };
    let vfs = match Mapping::load(Path::new(&mapping_path)) {
        Ok(mapping) => mapping.into_vfs(),
        Err(error) => {
            log(&format!("cannot read the mapping: {error}"));
            return MAPPING_UNREADABLE;
        }
    };
    let files = vfs.overlay().len();
    let _ = VFS.set(RwLock::new(vfs));

    let Some(real) = (unsafe { resolve_real() }) else {
        log("the real file functions could not be found");
        return REAL_FUNCTIONS_MISSING;
    };
    let _ = REAL.set(real);

    let patched = iat::patch_all(&patches(), &skipped_modules());
    log(&format!(
        "virtual file system ready: {files} files mapped, {patched} imports redirected"
    ));
    OK
}

/// Each hook is cast to the real function's type first, so a signature that
/// drifts from the one Windows calls with is a compile error, not a crash.
fn patches() -> Vec<Patch> {
    vec![
        Patch {
            name: b"CreateFileW",
            hook: hook_create_file as CreateFileW as usize,
        },
        Patch {
            name: b"GetFileAttributesW",
            hook: hook_get_attributes as GetFileAttributesW as usize,
        },
        Patch {
            name: b"GetFileAttributesExW",
            hook: hook_get_attributes_ex as GetFileAttributesExW as usize,
        },
        Patch {
            name: b"FindFirstFileW",
            hook: hook_find_first as FindFirstFileW as usize,
        },
        Patch {
            name: b"FindFirstFileExW",
            hook: hook_find_first_ex as FindFirstFileExW as usize,
        },
        Patch {
            name: b"FindNextFileW",
            hook: hook_find_next as FindNextFileW as usize,
        },
        Patch {
            name: b"FindClose",
            hook: hook_find_close as FindClose as usize,
        },
        Patch {
            name: b"CreateDirectoryW",
            hook: hook_create_directory as CreateDirectoryW as usize,
        },
        Patch {
            name: b"MoveFileExW",
            hook: hook_move_file_ex as MoveFileExW as usize,
        },
        Patch {
            name: b"DeleteFileW",
            hook: hook_delete_file as DeleteFileW as usize,
        },
        Patch {
            name: b"LoadLibraryW",
            hook: hook_load_library as LoadLibraryW as usize,
        },
        Patch {
            name: b"LoadLibraryExW",
            hook: hook_load_library_ex as LoadLibraryExW as usize,
        },
    ]
}

/// This DLL, which must keep reaching the real functions, and the system
/// modules that implement them.
fn skipped_modules() -> Vec<HMODULE> {
    let mut skip = Vec::new();
    let mut own: HMODULE = std::ptr::null_mut();
    // SAFETY: asks which module contains this function's address.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            sbmm_vfs_init as *const u16,
            &mut own,
        )
    };
    if found != 0 {
        skip.push(own);
    }
    for name in ["kernel32.dll", "kernelbase.dll", "ntdll.dll"] {
        let module = unsafe { GetModuleHandleW(wide(name).as_ptr()) };
        if !module.is_null() {
            skip.push(module);
        }
    }
    skip
}

unsafe fn resolve_real() -> Option<Real> {
    let kernelbase = GetModuleHandleW(wide("kernelbase.dll").as_ptr());
    let kernel32 = GetModuleHandleW(wide("kernel32.dll").as_ptr());
    let find = |name: &[u8]| -> Option<usize> {
        let mut symbol = name.to_vec();
        symbol.push(0);
        [kernelbase, kernel32]
            .into_iter()
            .filter(|m| !m.is_null())
            .find_map(|module| GetProcAddress(module, symbol.as_ptr()))
            .map(|f| f as usize)
    };
    macro_rules! real {
        ($name:literal, $type:ty) => {
            std::mem::transmute::<usize, $type>(find($name)?)
        };
    }
    Some(Real {
        create_file: real!(b"CreateFileW", CreateFileW),
        get_attributes: real!(b"GetFileAttributesW", GetFileAttributesW),
        get_attributes_ex: real!(b"GetFileAttributesExW", GetFileAttributesExW),
        find_first_ex: real!(b"FindFirstFileExW", FindFirstFileExW),
        find_next: real!(b"FindNextFileW", FindNextFileW),
        find_close: real!(b"FindClose", FindClose),
        create_directory: real!(b"CreateDirectoryW", CreateDirectoryW),
        move_file_ex: real!(b"MoveFileExW", MoveFileExW),
        delete_file: real!(b"DeleteFileW", DeleteFileW),
        load_library: real!(b"LoadLibraryW", LoadLibraryW),
        load_library_ex: real!(b"LoadLibraryExW", LoadLibraryExW),
    })
}

fn real() -> &'static Real {
    // The hooks are only installed after `REAL` is set.
    REAL.get()
        .expect("hooks installed before the real functions were resolved")
}

// -- path helpers -----------------------------------------------------------

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    wide(&path.to_string_lossy())
}

/// Read a NUL-terminated wide string.
///
/// # Safety
/// `ptr` must be null or point to a NUL-terminated UTF-16 string.
unsafe fn read_wide(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        ptr, len,
    )))
}

/// The absolute form of a path the game passed, as Windows would resolve it
/// against the current directory.
fn full_path(raw: &[u16]) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    loop {
        // SAFETY: `raw` is NUL-terminated and `buffer` is as long as we say.
        let written = unsafe {
            GetFullPathNameW(
                raw.as_ptr(),
                buffer.len() as u32,
                buffer.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        } as usize;
        if written == 0 {
            return None;
        }
        if written < buffer.len() {
            return Some(String::from_utf16_lossy(&buffer[..written]));
        }
        buffer.resize(written + 1, 0);
    }
}

/// Where a path the game passed falls inside the game folder, if it does.
unsafe fn locate(ptr: *const u16) -> Option<(VPath, String)> {
    let text = read_wide(ptr)?;
    let full = full_path(&wide(&text))?;
    let vfs = VFS.get()?.read().ok()?;
    let path = vfs.locate(&full)?;
    Some((path, full))
}

fn real_exists(full: &str) -> bool {
    unsafe { (real().get_attributes)(wide(full).as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn redirect(path: &VPath, access: Access, exists: bool) -> Option<Redirect> {
    Some(VFS.get()?.read().ok()?.redirect(path, access, exists))
}

fn in_overwrite(real: &Path) -> bool {
    VFS.get()
        .and_then(|v| v.read().ok().map(|v| real.starts_with(v.overwrite_root())))
        .unwrap_or(false)
}

fn log(message: &str) {
    if let Ok(mut guard) = LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{message}");
        }
    }
}

// -- opening files -----------------------------------------------------------

const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;

unsafe extern "system" fn hook_create_file(
    name: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    disposition: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let call = |path: *const u16| unsafe {
        (real().create_file)(path, access, share, security, disposition, flags, template)
    };
    let Some((path, full)) = locate(name) else {
        return call(name);
    };

    let wants = if matches!(disposition, CREATE_NEW | CREATE_ALWAYS | OPEN_ALWAYS) {
        Access::Create
    } else if access & (GENERIC_WRITE | FILE_WRITE_DATA | FILE_APPEND_DATA) != 0 {
        Access::WriteExisting
    } else {
        Access::Read
    };
    match redirect(&path, wants, real_exists(&full)) {
        Some(Redirect::To(target)) => {
            let new_in_overwrite = in_overwrite(&target) && !target.exists();
            if new_in_overwrite {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            let handle = call(wide_path(&target).as_ptr());
            if handle != INVALID_HANDLE_VALUE && new_in_overwrite {
                if let Some(vfs) = VFS.get() {
                    if let Ok(mut vfs) = vfs.write() {
                        vfs.created(&path, target);
                    }
                }
            }
            handle
        }
        // A directory handle for a folder only the overlay has: it is made
        // real in overwrite, empty, so there is something to hand back.
        Some(Redirect::VirtualDir) => {
            let Some(target) = VFS
                .get()
                .and_then(|v| v.read().ok().map(|v| v.overwrite_path(&path)))
            else {
                return call(name);
            };
            let _ = std::fs::create_dir_all(&target);
            call(wide_path(&target).as_ptr())
        }
        _ => call(name),
    }
}

/// Where to read attributes from: the overlay's file, nothing for a folder
/// only the overlay has, or the path as given.
enum AttributeSource {
    Real,
    File(PathBuf),
    VirtualDir,
}

unsafe fn attribute_source(name: *const u16) -> AttributeSource {
    let Some((path, full)) = locate(name) else {
        return AttributeSource::Real;
    };
    match redirect(&path, Access::Read, real_exists(&full)) {
        Some(Redirect::To(target)) => AttributeSource::File(target),
        Some(Redirect::VirtualDir) => AttributeSource::VirtualDir,
        _ => AttributeSource::Real,
    }
}

unsafe extern "system" fn hook_get_attributes(name: *const u16) -> u32 {
    match attribute_source(name) {
        AttributeSource::File(target) => (real().get_attributes)(wide_path(&target).as_ptr()),
        AttributeSource::VirtualDir => FILE_ATTRIBUTE_DIRECTORY,
        AttributeSource::Real => (real().get_attributes)(name),
    }
}

/// `GetFileExInfoStandard`, the only level there is.
const FILE_EX_INFO_STANDARD: i32 = 0;

unsafe extern "system" fn hook_get_attributes_ex(
    name: *const u16,
    level: i32,
    info: *mut c_void,
) -> BOOL {
    match attribute_source(name) {
        AttributeSource::File(target) => {
            (real().get_attributes_ex)(wide_path(&target).as_ptr(), level, info)
        }
        AttributeSource::VirtualDir if level == FILE_EX_INFO_STANDARD && !info.is_null() => {
            let data = info as *mut WIN32_FILE_ATTRIBUTE_DATA;
            std::ptr::write_bytes(data, 0, 1);
            (*data).dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY;
            1
        }
        _ => (real().get_attributes_ex)(name, level, info),
    }
}

// -- listing directories -------------------------------------------------------

/// `FindExInfoStandard` and `FindExInfoBasic`; both fill a `WIN32_FIND_DATAW`.
const FIND_EX_INFO_STANDARD: i32 = 0;

unsafe extern "system" fn hook_find_first(name: *const u16, data: *mut WIN32_FIND_DATAW) -> HANDLE {
    hook_find_first_ex(
        name,
        FIND_EX_INFO_STANDARD,
        data as *mut c_void,
        0,
        std::ptr::null(),
        0,
    )
}

unsafe extern "system" fn hook_find_first_ex(
    name: *const u16,
    level: i32,
    data: *mut c_void,
    search: i32,
    filter: *const c_void,
    flags: u32,
) -> HANDLE {
    let call = || unsafe { (real().find_first_ex)(name, level, data, search, filter, flags) };
    let Some(entries) = virtual_search(name) else {
        return call();
    };
    let mut entries = entries.into_iter();
    let Some(first) = entries.next() else {
        SetLastError(ERROR_FILE_NOT_FOUND);
        return INVALID_HANDLE_VALUE;
    };
    std::ptr::write(data as *mut WIN32_FIND_DATAW, first);

    let search = Box::new(Search {
        entries: entries.collect(),
        next: 0,
    });
    // The box's address is the handle: it stays put while the box moves into
    // the table, is unique while the search is open, and can never equal a
    // real search handle, which is also a live allocation.
    let handle = &*search as *const Search as usize;
    match SEARCHES.lock() {
        Ok(mut searches) => {
            searches
                .get_or_insert_with(HashMap::new)
                .insert(handle, search);
            handle as HANDLE
        }
        Err(_) => call(),
    }
}

/// The merged, filtered listing for a search inside a folder the overlay
/// adds to, or `None` to let the real search run.
unsafe fn virtual_search(name: *const u16) -> Option<Vec<WIN32_FIND_DATAW>> {
    let text = read_wide(name)?;
    let full = full_path(&wide(&text))?;
    let split = full.rfind(['\\', '/'])?;
    let (directory, pattern) = (&full[..split], &full[split + 1..]);

    let vfs = VFS.get()?.read().ok()?;
    let dir = vfs.locate(directory)?;
    if !vfs.overlay().is_dir(&dir) {
        return None;
    }

    let real_entries = real_listing(directory);
    let names = real_entries.iter().map(|(name, data)| {
        (
            name.clone(),
            data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        )
    });
    let mut out = Vec::new();
    for listed in vfs.list(&dir, names.collect::<Vec<_>>()) {
        if !pattern::matches(pattern, &listed.name) {
            continue;
        }
        let mut entry = match (&listed.source, listed.is_dir) {
            (Some(source), _) => single_entry(source).unwrap_or_else(empty_entry),
            (None, _) => real_entries
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&listed.name))
                .map(|(_, d)| *d)
                .unwrap_or_else(|| {
                    let mut d = empty_entry();
                    d.dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY;
                    d
                }),
        };
        set_name(&mut entry, &listed.name);
        out.push(entry);
    }
    // A folder only the overlay has still lists `.` and `..`, as every real
    // folder does, since some code skips the first two entries blindly.
    if real_entries.is_empty() && pattern::matches(pattern, ".") {
        for dot in ["..", "."] {
            let mut entry = empty_entry();
            entry.dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY;
            set_name(&mut entry, dot);
            out.insert(0, entry);
        }
    }
    Some(out)
}

/// Every entry of a real folder, with its find data.
unsafe fn real_listing(directory: &str) -> Vec<(String, WIN32_FIND_DATAW)> {
    let mut out = Vec::new();
    let query = wide(&format!("{directory}\\*"));
    let mut data = empty_entry();
    let handle = (real().find_first_ex)(
        query.as_ptr(),
        FIND_EX_INFO_STANDARD,
        &mut data as *mut _ as *mut c_void,
        0,
        std::ptr::null(),
        0,
    );
    if handle == INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        out.push((entry_name(&data), data));
        if (real().find_next)(handle, &mut data) == 0 {
            break;
        }
    }
    (real().find_close)(handle);
    out
}

/// The find data of one real file, used for a file the overlay provides.
unsafe fn single_entry(path: &Path) -> Option<WIN32_FIND_DATAW> {
    let mut data = empty_entry();
    let handle = (real().find_first_ex)(
        wide_path(path).as_ptr(),
        FIND_EX_INFO_STANDARD,
        &mut data as *mut _ as *mut c_void,
        0,
        std::ptr::null(),
        0,
    );
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    (real().find_close)(handle);
    Some(data)
}

fn empty_entry() -> WIN32_FIND_DATAW {
    // SAFETY: a plain C struct for which all-zero is a valid value.
    unsafe { std::mem::zeroed() }
}

fn entry_name(data: &WIN32_FIND_DATAW) -> String {
    let len = data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(data.cFileName.len());
    String::from_utf16_lossy(&data.cFileName[..len])
}

fn set_name(data: &mut WIN32_FIND_DATAW, name: &str) {
    data.cFileName = [0; 260];
    data.cAlternateFileName = [0; 14];
    for (slot, unit) in data.cFileName.iter_mut().zip(name.encode_utf16().take(259)) {
        *slot = unit;
    }
}

unsafe extern "system" fn hook_find_next(handle: HANDLE, data: *mut WIN32_FIND_DATAW) -> BOOL {
    if let Ok(mut searches) = SEARCHES.lock() {
        if let Some(search) = searches
            .as_mut()
            .and_then(|s| s.get_mut(&(handle as usize)))
        {
            return match search.entries.get(search.next) {
                Some(entry) => {
                    std::ptr::write(data, *entry);
                    search.next += 1;
                    1
                }
                None => {
                    SetLastError(ERROR_NO_MORE_FILES);
                    0
                }
            };
        }
    }
    (real().find_next)(handle, data)
}

unsafe extern "system" fn hook_find_close(handle: HANDLE) -> BOOL {
    if let Ok(mut searches) = SEARCHES.lock() {
        if searches
            .as_mut()
            .and_then(|s| s.remove(&(handle as usize)))
            .is_some()
        {
            return 1;
        }
    }
    (real().find_close)(handle)
}

// -- changing the folder ---------------------------------------------------------

unsafe extern "system" fn hook_create_directory(name: *const u16, security: *const c_void) -> BOOL {
    let Some((path, full)) = locate(name) else {
        return (real().create_directory)(name, security);
    };
    let exists = real_exists(&full);
    let Some(vfs) = VFS.get() else {
        return (real().create_directory)(name, security);
    };
    let decision = vfs
        .read()
        .ok()
        .map(|v| (v.redirect_mkdir(&path, exists), v.overlay().is_dir(&path)));
    match decision {
        Some((Redirect::To(target), _)) => {
            if std::fs::create_dir_all(&target).is_err() {
                return (real().create_directory)(name, security);
            }
            if let Ok(mut vfs) = vfs.write() {
                vfs.created_dir(&path);
            }
            1
        }
        // Already there, but only virtually: the answer Windows gives for an
        // existing folder.
        Some((Redirect::Passthrough, true)) if !exists => {
            SetLastError(ERROR_ALREADY_EXISTS);
            0
        }
        _ => (real().create_directory)(name, security),
    }
}

unsafe extern "system" fn hook_move_file_ex(from: *const u16, to: *const u16, flags: u32) -> BOOL {
    let source = locate(from);
    let destination = locate(to);
    if source.is_none() && destination.is_none() {
        return (real().move_file_ex)(from, to, flags);
    }

    let from_real = match &source {
        Some((path, full)) => match redirect(path, Access::Read, real_exists(full)) {
            Some(Redirect::To(target)) => wide_path(&target),
            _ => wide(full),
        },
        None => wide(&read_wide(from).unwrap_or_default()),
    };
    let mut created = None;
    let to_real = match &destination {
        Some((path, full)) => match redirect(path, Access::Create, real_exists(full)) {
            Some(Redirect::To(target)) => {
                if in_overwrite(&target) && !target.exists() {
                    if let Some(parent) = target.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    created = Some((path.clone(), target.clone()));
                }
                wide_path(&target)
            }
            _ => wide(full),
        },
        None => wide(&read_wide(to).unwrap_or_default()),
    };

    let moved = (real().move_file_ex)(from_real.as_ptr(), to_real.as_ptr(), flags);
    if moved != 0 {
        if let Some(vfs) = VFS.get() {
            if let Ok(mut vfs) = vfs.write() {
                if let Some((path, _)) = &source {
                    vfs.removed(path);
                }
                if let Some((path, target)) = created {
                    vfs.created(&path, target);
                }
            }
        }
    }
    moved
}

unsafe extern "system" fn hook_delete_file(name: *const u16) -> BOOL {
    let Some((path, full)) = locate(name) else {
        return (real().delete_file)(name);
    };
    match redirect(&path, Access::Read, real_exists(&full)) {
        // Deleting a mod's file deletes it from the mod, as in Mod Organizer.
        Some(Redirect::To(target)) => {
            let deleted = (real().delete_file)(wide_path(&target).as_ptr());
            if deleted != 0 {
                if let Some(vfs) = VFS.get() {
                    if let Ok(mut vfs) = vfs.write() {
                        vfs.removed(&path);
                    }
                }
            }
            deleted
        }
        _ => (real().delete_file)(name),
    }
}

// -- loading code ------------------------------------------------------------------

/// A library path the overlay provides, for a name that is a path at all —
/// bare names go through the normal search order untouched.
unsafe fn library_path(name: *const u16) -> Option<Vec<u16>> {
    let text = read_wide(name)?;
    if !text.contains(['\\', '/']) {
        return None;
    }
    let (path, full) = locate(name)?;
    match redirect(&path, Access::Read, real_exists(&full))? {
        Redirect::To(target) => Some(wide_path(&target)),
        _ => None,
    }
}

/// Modules a library brings in have import tables of their own, which need
/// the same treatment — UE4SS's C++ mods arrive this way.
fn after_load(module: HMODULE) -> HMODULE {
    if !module.is_null() {
        iat::patch_all(&patches(), &skipped_modules());
    }
    module
}

unsafe extern "system" fn hook_load_library(name: *const u16) -> HMODULE {
    let module = match library_path(name) {
        Some(target) => (real().load_library)(target.as_ptr()),
        None => (real().load_library)(name),
    };
    after_load(module)
}

unsafe extern "system" fn hook_load_library_ex(
    name: *const u16,
    file: HANDLE,
    flags: u32,
) -> HMODULE {
    let module = match library_path(name) {
        Some(target) => (real().load_library_ex)(target.as_ptr(), file, flags),
        None => (real().load_library_ex)(name, file, flags),
    };
    after_load(module)
}
