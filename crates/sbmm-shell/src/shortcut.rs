//! Shortcuts to the manager: a `.lnk` file on Windows, a `.desktop` entry
//! elsewhere (which only matters while developing on Linux).

use std::io;
use std::path::{Path, PathBuf};

/// What a shortcut runs and how it looks.
#[derive(Debug, Clone)]
pub struct Shortcut {
    /// The name shown under the icon; also the file name, made safe.
    pub name: String,
    pub description: String,
    pub target: PathBuf,
    pub arguments: Vec<String>,
    pub working_dir: Option<PathBuf>,
    /// A file whose first icon is used, such as the game's executable.
    pub icon: Option<PathBuf>,
}

/// `name` with what a file name cannot hold replaced, and without the
/// trailing dots and spaces Windows drops.
pub fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    if cleaned.is_empty() {
        "SB Mod Manager".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Write `shortcut` into `dir`, replacing one of the same name, and return
/// where it went.
pub fn create(shortcut: &Shortcut, dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.{}", file_name(&shortcut.name), EXTENSION));
    write(shortcut, &path)?;
    Ok(path)
}

#[cfg(windows)]
const EXTENSION: &str = "lnk";
#[cfg(not(windows))]
const EXTENSION: &str = "desktop";

#[cfg(windows)]
fn write(shortcut: &Shortcut, path: &Path) -> io::Result<()> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let arguments = crate::args::arguments_line(&shortcut.arguments);
    // SAFETY: plain COM calls on this thread. COM is initialised for the
    // duration and released only if this call was the one to initialise it;
    // a thread already set up differently keeps its own setup.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // Each step is named in the error, since the shell's own message
        // ("File not found.") rarely says which file.
        let step = |what: &'static str| move |error: windows::core::Error| (what, error);
        let result = (|| -> Result<(), (&str, windows::core::Error)> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(step("the shell link object"))?;
            link.SetPath(&HSTRING::from(shortcut.target.as_os_str()))
                .map_err(step("the target"))?;
            link.SetArguments(&HSTRING::from(arguments.as_str()))
                .map_err(step("the arguments"))?;
            link.SetDescription(&HSTRING::from(shortcut.description.as_str()))
                .map_err(step("the description"))?;
            if let Some(dir) = &shortcut.working_dir {
                link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
                    .map_err(step("the working folder"))?;
            }
            if let Some(icon) = &shortcut.icon {
                link.SetIconLocation(&HSTRING::from(icon.as_os_str()), 0)
                    .map_err(step("the icon"))?;
            }
            let file: IPersistFile = link.cast().map_err(step("saving"))?;
            file.Save(&HSTRING::from(path.as_os_str()), true)
                .map_err(step("saving"))
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result.map_err(|(what, error)| {
            io::Error::other(format!("could not set {what}: {}", error.message()))
        })
    }
}

#[cfg(not(windows))]
fn write(shortcut: &Shortcut, path: &Path) -> io::Result<()> {
    let exec = std::iter::once(shortcut.target.to_string_lossy().into_owned())
        .chain(shortcut.arguments.iter().cloned())
        .map(|arg| desktop_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    let mut entry = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\n",
        one_line(&shortcut.name),
        one_line(&shortcut.description),
        exec,
    );
    if let Some(dir) = &shortcut.working_dir {
        entry.push_str(&format!("Path={}\n", one_line(&dir.to_string_lossy())));
    }
    if let Some(icon) = &shortcut.icon {
        entry.push_str(&format!("Icon={}\n", one_line(&icon.to_string_lossy())));
    }
    std::fs::write(path, entry)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn one_line(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

/// An `Exec=` argument: quoted per the desktop entry spec, then with its
/// backslashes doubled again for the key file's own string escaping.
#[cfg(not(windows))]
fn desktop_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:=+,@".contains(c));
    let quoted = if plain {
        arg.to_string()
    } else {
        let mut out = String::from('"');
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    };
    one_line(&quoted.replace('\\', "\\\\")).replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_lose_what_windows_refuses() {
        assert_eq!(
            file_name("Stellar Blade (Cosmetics only)"),
            "Stellar Blade (Cosmetics only)"
        );
        assert_eq!(file_name("a/b:c*d?"), "a_b_c_d_");
        assert_eq!(file_name("Фото режим. "), "Фото режим");
        assert_eq!(file_name("..."), "SB Mod Manager");
    }

    #[cfg(windows)]
    #[test]
    fn the_shell_writes_a_link_to_the_manager() {
        let dir = tempfile::tempdir().unwrap();
        let target = std::env::current_exe().unwrap();
        let shortcut = Shortcut {
            name: "Stellar Blade (Фото режим)".into(),
            description: "Start Stellar Blade with this profile".into(),
            target: target.clone(),
            arguments: crate::args::arguments(Some("Фото режим"), true),
            working_dir: target.parent().map(Path::to_path_buf),
            icon: Some(target),
        };
        let path = create(&shortcut, dir.path()).unwrap();
        assert_eq!(path.file_name().unwrap(), "Stellar Blade (Фото режим).lnk");
        let bytes = std::fs::read(&path).unwrap();
        // ShellLinkHeader: size 0x4C, then the link class id.
        assert_eq!(&bytes[..4], &[0x4C, 0, 0, 0]);
        // The arguments are stored as UTF-16, Cyrillic intact.
        let wide: Vec<u8> = "--profile \"Фото режим\" --launch"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(bytes.windows(wide.len()).any(|w| w == wide.as_slice()));
    }

    #[cfg(not(windows))]
    #[test]
    fn a_desktop_entry_runs_the_manager_with_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let shortcut = Shortcut {
            name: "Stellar Blade (Фото: \"режим\")".into(),
            description: "Start Stellar Blade with this profile".into(),
            target: PathBuf::from("/opt/sb mod manager/sbmm"),
            arguments: crate::args::arguments(Some("Фото \"режим\""), true),
            working_dir: None,
            icon: None,
        };
        let path = create(&shortcut, dir.path()).unwrap();
        assert_eq!(
            path.file_name().unwrap(),
            "Stellar Blade (Фото_ _режим_).desktop"
        );
        let entry = std::fs::read_to_string(&path).unwrap();
        assert!(entry
            .contains(r#"Exec="/opt/sb mod manager/sbmm" --profile "Фото \\"режим\\"" --launch"#));
    }
}
