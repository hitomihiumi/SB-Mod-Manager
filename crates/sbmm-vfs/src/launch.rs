//! Starting the game with the virtual file system in place.
//!
//! The order matters, and is the one USVFS uses for Mod Organizer 2: the game
//! is created suspended, so not one instruction of it runs before the hook is
//! ready; the hook DLL is loaded into it with a remote `LoadLibraryW`; its
//! init function is called on a second remote thread and must report success;
//! only then is the game's main thread let go. If any step fails the game is
//! terminated rather than left running without its mods, which would look
//! like the mods "not working" with nothing to say why.
//!
//! The mapping's location reaches the hook through the game's environment,
//! which also carries the Steam app id so a game started outside Steam does
//! not relaunch itself through Steam — and out from under the hook.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub executable: PathBuf,
    pub working_dir: PathBuf,
    pub arguments: Vec<String>,
    /// Added to, or replacing, the manager's own environment.
    pub environment: Vec<(String, String)>,
    /// `sbmm_vfs_hook.dll`.
    pub hook: PathBuf,
    pub mapping: PathBuf,
    /// Where the hook, running inside the game, writes what it did.
    pub log: Option<PathBuf>,
    /// Where the launcher itself records each step and any failure. Written
    /// from the manager's process, so it exists even when the hook never
    /// loads — which is exactly the case the hook's own log cannot cover.
    pub diagnostics: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("the virtual file system only runs on Windows")]
    Unsupported,
    #[error("{step} failed (Windows error {code})")]
    Os { step: &'static str, code: u32 },
    #[error("the game did not load the virtual file system hook (see the launch log)")]
    HookNotLoaded,
    #[error("the virtual file system hook could not start (code {0}); see the hook log")]
    HookFailed(u32),
    #[error("{0} did not finish in time")]
    Timeout(&'static str),
}

/// The variable the hook reads the mapping's path from.
pub const MAPPING_VARIABLE: &str = "SBMM_VFS_MAPPING";
/// The variable the hook reads its log file's path from.
pub const LOG_VARIABLE: &str = "SBMM_VFS_LOG";
/// The hook's init export, as `GetProcAddress` wants it.
pub const INIT_EXPORT: &str = "sbmm_vfs_init";

/// A Win32 command line for `executable` and `arguments`, quoted the way
/// `CommandLineToArgvW` reads it back.
pub fn command_line(executable: &str, arguments: &[String]) -> String {
    std::iter::once(executable)
        .chain(arguments.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in argument.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are escapes, so double them, then
                // escape the quote itself.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    // Backslashes before the closing quote would escape it.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// A `CREATE_UNICODE_ENVIRONMENT` block: `base` with `extra` laid over it,
/// sorted case-insensitively as Windows requires, double-NUL terminated.
pub fn environment_block(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    extra: &[(String, String)],
) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = base
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .filter(|(k, _)| !extra.iter().any(|(e, _)| e.eq_ignore_ascii_case(k)))
        .collect();
    vars.extend(extra.iter().cloned());
    vars.sort_by_key(|(k, _)| k.to_uppercase());

    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

/// Start the game under the virtual file system. Returns its process id.
pub fn launch(options: &LaunchOptions) -> Result<u32, LaunchError> {
    #[cfg(windows)]
    {
        windows::launch(options)
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        Err(LaunchError::Unsupported)
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        CloseHandle, FreeLibrary, GetLastError, HANDLE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleW, GetProcAddress, LoadLibraryExW, DONT_RESOLVE_DLL_REFERENCES,
    };
    use windows_sys::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CreateRemoteThread, GetExitCodeThread, ResumeThread, TerminateProcess,
        WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, LPTHREAD_START_ROUTINE,
        PROCESS_INFORMATION, STARTUPINFOW,
    };

    use super::*;

    /// Loading a DLL can mean the loader initialising the whole process, so
    /// this is generous; a hang beyond it is a real hang.
    const STEP_TIMEOUT_MS: u32 = 30_000;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A plain text log of the launch, written from the manager's process so
    /// it survives the hook never loading. Opening it is best-effort: a launch
    /// is never blocked because its log could not be written.
    pub(super) struct Diag(Option<std::fs::File>);

    impl Diag {
        fn open(path: &Option<std::path::PathBuf>) -> Diag {
            let file = path.as_ref().and_then(|path| {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .ok()
            });
            Diag(file)
        }

        pub(super) fn log(&mut self, message: impl AsRef<str>) {
            if let Some(file) = self.0.as_mut() {
                use std::io::Write;
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let _ = writeln!(file, "[{stamp}] {}", message.as_ref());
            }
        }
    }

    fn os_error(step: &'static str) -> LaunchError {
        LaunchError::Os {
            step,
            code: unsafe { GetLastError() },
        }
    }

    /// Closes a handle when dropped, so every early return cleans up.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    /// Kills the suspended game unless disarmed by a successful launch.
    struct Abort(HANDLE);

    impl Drop for Abort {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { TerminateProcess(self.0, 1) };
            }
        }
    }

    pub fn launch(options: &LaunchOptions) -> Result<u32, LaunchError> {
        let mut diag = Diag::open(&options.diagnostics);
        diag.log(format!("launching {}", options.executable.display()));
        diag.log(format!("hook: {}", options.hook.display()));
        diag.log(format!("mapping: {}", options.mapping.display()));
        match launch_inner(options, &mut diag) {
            Ok(pid) => {
                diag.log(format!("started, process {pid}"));
                Ok(pid)
            }
            Err(error) => {
                diag.log(format!("FAILED: {error}"));
                Err(error)
            }
        }
    }

    fn launch_inner(options: &LaunchOptions, diag: &mut Diag) -> Result<u32, LaunchError> {
        let mut extra = options.environment.clone();
        extra.push((
            MAPPING_VARIABLE.into(),
            options.mapping.to_string_lossy().into_owned(),
        ));
        if let Some(log) = &options.log {
            extra.push((LOG_VARIABLE.into(), log.to_string_lossy().into_owned()));
        }
        let mut environment = environment_block(std::env::vars_os(), &extra);

        let executable = options.executable.to_string_lossy().into_owned();
        let mut command = wide(&command_line(&executable, &options.arguments));
        let application = wide(&executable);
        let directory = wide(&options.working_dir.to_string_lossy());

        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

        // SAFETY: every pointer is to a NUL-terminated buffer that outlives
        // the call; the command line buffer is mutable as the API requires.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                environment.as_mut_ptr() as *const c_void,
                directory.as_ptr(),
                &startup,
                &mut process,
            )
        };
        if created == 0 {
            return Err(os_error("starting the game"));
        }
        diag.log(format!(
            "game created suspended, process {}",
            process.dwProcessId
        ));
        let _process_handle = Owned(process.hProcess);
        let _thread_handle = Owned(process.hThread);
        let mut abort = Abort(process.hProcess);

        let base = load_hook(process.hProcess, process.dwProcessId, &options.hook, diag)?;
        diag.log(format!("hook loaded at 0x{base:x}"));
        let init = remote_export(base, &options.hook)?;
        diag.log(format!("init export at 0x{init:x}; calling it"));
        let code = run_remote(
            process.hProcess,
            init,
            std::ptr::null_mut(),
            "starting the hook",
        )?;
        diag.log(format!("hook init returned {code}"));
        if code != 0 {
            return Err(LaunchError::HookFailed(code));
        }

        if unsafe { ResumeThread(process.hThread) } == u32::MAX {
            return Err(os_error("resuming the game"));
        }
        diag.log("game resumed");
        abort.0 = std::ptr::null_mut();
        Ok(process.dwProcessId)
    }

    /// `LoadLibraryW(hook)` inside the game. Returns the DLL's base there.
    fn load_hook(
        process: HANDLE,
        pid: u32,
        hook: &Path,
        diag: &mut Diag,
    ) -> Result<usize, LaunchError> {
        if !hook.is_file() {
            diag.log("the hook DLL is not on disk");
            return Err(LaunchError::HookNotLoaded);
        }
        let path = wide(&hook.to_string_lossy());
        let bytes = path.len() * 2;
        // SAFETY: plain allocation and copy into the target process.
        let remote = unsafe {
            VirtualAllocEx(
                process,
                std::ptr::null(),
                bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote.is_null() {
            return Err(os_error("reserving memory in the game"));
        }
        let written = unsafe {
            WriteProcessMemory(
                process,
                remote,
                path.as_ptr() as *const c_void,
                bytes,
                std::ptr::null_mut(),
            )
        };
        if written == 0 {
            unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
            return Err(os_error("writing into the game's memory"));
        }

        // kernel32 sits at the same address in every process of a session,
        // so its `LoadLibraryW` here is its `LoadLibraryW` there.
        let load_library = unsafe {
            let kernel32 = GetModuleHandleW(wide("kernel32.dll").as_ptr());
            GetProcAddress(kernel32, c"LoadLibraryW".as_ptr() as *const u8)
        }
        .ok_or_else(|| os_error("finding LoadLibraryW"))?;

        let result = run_remote(process, load_library as usize, remote, "loading the hook");
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        // The thread's exit code is the module handle cut to 32 bits: good
        // enough to tell success from failure, not to use as an address. Zero
        // means LoadLibraryW itself failed inside the game — most often a
        // missing dependency of the hook DLL.
        let low = result?;
        diag.log(format!("remote LoadLibraryW returned 0x{low:x}"));
        if low == 0 {
            return Err(LaunchError::HookNotLoaded);
        }

        let name = hook
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // A process this young can refuse a module list, or hand back one
        // that is cut short, while its loader is still busy; Windows' advice
        // is to ask again. LoadLibraryW has already returned, so a list that
        // does come back whole has the hook in it.
        let mut last_error = 0;
        for attempt in 0..MODULE_LIST_ATTEMPTS {
            match module_base(pid, &name) {
                Ok(Some(base)) => {
                    if attempt > 0 {
                        diag.log(format!("module list read on attempt {}", attempt + 1));
                    }
                    return Ok(base);
                }
                Ok(None) => last_error = 0,
                Err(code) => last_error = code,
            }
            std::thread::sleep(std::time::Duration::from_millis(MODULE_LIST_PAUSE_MS));
        }
        diag.log(if last_error == 0 {
            format!("{name} is not among the game's loaded modules")
        } else {
            format!("the game's module list could not be read (error {last_error})")
        });
        Err(LaunchError::HookNotLoaded)
    }

    /// How often, and how far apart, to ask for the module list.
    const MODULE_LIST_ATTEMPTS: u32 = 40;
    const MODULE_LIST_PAUSE_MS: u64 = 25;

    /// Where a module is loaded in another process, by file name: `Ok(None)`
    /// when a whole list was read without it, `Err` with the system's error
    /// code when no list could be read.
    fn module_base(pid: u32, name: &str) -> Result<Option<usize>, u32> {
        let snapshot = Owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) });
        if snapshot.0.is_null() || snapshot.0 as isize == -1 {
            return Err(unsafe { GetLastError() });
        }
        let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut more = unsafe { Module32FirstW(snapshot.0, &mut entry) } != 0;
        if !more {
            return Err(unsafe { GetLastError() });
        }
        while more {
            let len = entry
                .szModule
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szModule.len());
            if String::from_utf16_lossy(&entry.szModule[..len]).to_lowercase() == name {
                return Ok(Some(entry.modBaseAddr as usize));
            }
            more = unsafe { Module32NextW(snapshot.0, &mut entry) } != 0;
        }
        Ok(None)
    }

    /// The address of the init export inside the game, from its offset in a
    /// copy mapped here without running any of its code.
    fn remote_export(remote_base: usize, hook: &Path) -> Result<usize, LaunchError> {
        let path = wide(&hook.to_string_lossy());
        let local = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                std::ptr::null_mut(),
                DONT_RESOLVE_DLL_REFERENCES,
            )
        };
        if local.is_null() {
            return Err(os_error("reading the hook"));
        }
        let mut name = INIT_EXPORT.as_bytes().to_vec();
        name.push(0);
        let export = unsafe { GetProcAddress(local, name.as_ptr()) };
        let offset = export.map(|f| f as usize - local as usize);
        unsafe { FreeLibrary(local) };
        offset
            .map(|offset| remote_base + offset)
            .ok_or(LaunchError::HookNotLoaded)
    }

    /// Run `start(parameter)` on a new thread in the game and wait for it.
    fn run_remote(
        process: HANDLE,
        start: usize,
        parameter: *mut c_void,
        step: &'static str,
    ) -> Result<u32, LaunchError> {
        // SAFETY: `start` is the address of a function with the thread-start
        // signature inside the target process.
        let routine: LPTHREAD_START_ROUTINE = unsafe { std::mem::transmute::<usize, _>(start) };
        let thread = Owned(unsafe {
            CreateRemoteThread(
                process,
                std::ptr::null(),
                0,
                routine,
                parameter,
                0,
                std::ptr::null_mut(),
            )
        });
        if thread.0.is_null() {
            return Err(os_error(step));
        }
        if unsafe { WaitForSingleObject(thread.0, STEP_TIMEOUT_MS) } != WAIT_OBJECT_0 {
            return Err(LaunchError::Timeout(step));
        }
        let mut code = 0u32;
        if unsafe { GetExitCodeThread(thread.0, &mut code) } == 0 {
            return Err(os_error(step));
        }
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_arguments_are_left_alone_and_spaces_are_quoted() {
        assert_eq!(
            command_line(r"C:\Games\Stellar Blade\SB.exe", &["-dx12".into()]),
            r#""C:\Games\Stellar Blade\SB.exe" -dx12"#
        );
    }

    #[test]
    fn quotes_and_trailing_backslashes_survive_the_round_trip() {
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(r"C:\Path With Space\"), r#""C:\Path With Space\\""#);
        assert_eq!(quote(""), r#""""#);
    }

    #[test]
    fn the_environment_block_is_sorted_and_overridden() {
        let block = environment_block(
            [
                (OsString::from("Path"), OsString::from("C:\\Windows")),
                (OsString::from("SteamAppId"), OsString::from("1")),
            ],
            &[
                ("STEAMAPPID".into(), "3489700".into()),
                ("SBMM_VFS_MAPPING".into(), "C:\\lib\\mapping.json".into()),
            ],
        );
        let text = String::from_utf16(&block).unwrap();
        let entries: Vec<&str> = text.split('\0').filter(|e| !e.is_empty()).collect();
        assert_eq!(
            entries,
            vec![
                "Path=C:\\Windows",
                "SBMM_VFS_MAPPING=C:\\lib\\mapping.json",
                "STEAMAPPID=3489700"
            ]
        );
        assert!(block.ends_with(&[0, 0]));
    }

    #[test]
    fn launching_says_so_plainly_off_windows() {
        if cfg!(windows) {
            return;
        }
        let options = LaunchOptions {
            executable: "game.exe".into(),
            working_dir: ".".into(),
            arguments: Vec::new(),
            environment: Vec::new(),
            hook: "hook.dll".into(),
            mapping: "mapping.json".into(),
            log: None,
            diagnostics: None,
        };
        assert!(matches!(launch(&options), Err(LaunchError::Unsupported)));
    }
}
