//! The manager's command line.
//!
//! ```text
//! sb-mod-manager.exe --profile "Cosmetics only" --launch
//! ```
//!
//! `--profile` (or `-p`) makes that profile active, `--launch` (or `-l`)
//! starts the game; either may come alone. Anything else — an `nxm://` link
//! from the browser, arguments of the webview — is left to whoever wants it.

use std::path::Path;

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// The profile to make active, by name.
    pub profile: Option<String>,
    /// Start the game once the profile is in place.
    pub launch: bool,
}

/// Read a request from the arguments after the program name. `Ok(None)` when
/// there is none, so an ordinary start stays ordinary.
pub fn parse<I, S>(args: I) -> Result<Option<Request>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut request = Request::default();
    let mut args = args.into_iter();
    let mut asked = false;
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        if let Some(name) = arg.strip_prefix("--profile=") {
            request.profile = Some(profile_name(name)?);
            asked = true;
        } else if arg == "--profile" || arg == "-p" {
            let name = args
                .next()
                .ok_or_else(|| format!("{arg} needs the name of a profile"))?;
            request.profile = Some(profile_name(name.as_ref())?);
            asked = true;
        } else if arg == "--launch" || arg == "-l" {
            request.launch = true;
            asked = true;
        }
    }
    Ok(asked.then_some(request))
}

fn profile_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("--profile needs the name of a profile".into());
    }
    Ok(name.to_string())
}

/// The arguments that ask for `profile` and, when `launch`, the game.
pub fn arguments(profile: Option<&str>, launch: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(profile) = profile {
        args.push("--profile".to_string());
        args.push(profile.to_string());
    }
    if launch {
        args.push("--launch".to_string());
    }
    args
}

/// One argument quoted so Windows hands it back unchanged — the rules of
/// `CommandLineToArgvW`, which Rust's own argument parsing follows.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        // Backslashes only escape when a quote follows them.
        let escapes = if c == '"' { slashes * 2 + 1 } else { slashes };
        out.extend(std::iter::repeat_n('\\', escapes));
        out.push(c);
        slashes = 0;
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// Arguments joined into one line, each quoted as needed.
pub fn arguments_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The whole command, as it would be typed or pasted into a launcher.
pub fn command_line(program: &Path, args: &[String]) -> String {
    let program = quote(&program.to_string_lossy());
    if args.is_empty() {
        program
    } else {
        format!("{program} {}", arguments_line(args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How Windows splits a command line, for checking what `quote` wrote.
    fn split(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut started = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let run = chars[i..].iter().take_while(|c| **c == '\\').count();
                if chars.get(i + run) == Some(&'"') {
                    current.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        current.push('"');
                        i += run + 1;
                    } else {
                        i += run;
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', run));
                    i += run;
                }
                started = true;
                continue;
            }
            match c {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                ' ' | '\t' if !quoted => {
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                _ => {
                    current.push(c);
                    started = true;
                }
            }
            i += 1;
        }
        if started {
            args.push(current);
        }
        args
    }

    #[test]
    fn an_ordinary_start_asks_for_nothing() {
        assert_eq!(parse(Vec::<String>::new()), Ok(None));
        assert_eq!(parse(["nxm://stellarblade/mods/1/files/2"]), Ok(None));
    }

    #[test]
    fn profile_and_launch_are_read_in_any_spelling() {
        let both = Request {
            profile: Some("Cosmetics only".into()),
            launch: true,
        };
        assert_eq!(
            parse(["--profile", "Cosmetics only", "--launch"]),
            Ok(Some(both.clone()))
        );
        assert_eq!(
            parse(["-l", "-p", "Cosmetics only"]),
            Ok(Some(both.clone()))
        );
        assert_eq!(
            parse(["--profile=Cosmetics only", "--launch"]),
            Ok(Some(both))
        );
        assert_eq!(
            parse(["--launch"]),
            Ok(Some(Request {
                profile: None,
                launch: true
            }))
        );
    }

    #[test]
    fn a_profile_without_a_name_is_refused() {
        assert!(parse(["--profile"]).is_err());
        assert!(parse(["--profile", "  "]).is_err());
        assert!(parse(["--profile="]).is_err());
    }

    #[test]
    fn quoting_survives_the_trip_through_windows() {
        for arg in [
            "plain",
            "Cosmetics only",
            "Косметика и \"фото\"",
            r"C:\Program Files\SB Mod Manager\",
            r#"ends with \"#,
            r"a\\b",
            "",
        ] {
            assert_eq!(split(&quote(arg)), vec![arg.to_string()], "{arg:?}");
        }
    }

    #[test]
    fn a_command_line_reads_back_as_its_arguments() {
        let args = arguments(Some("Без модов"), true);
        let line = command_line(Path::new(r"C:\Games\SB Mod Manager\sbmm.exe"), &args);
        assert_eq!(
            line,
            r#""C:\Games\SB Mod Manager\sbmm.exe" --profile "Без модов" --launch"#
        );
        let mut read = split(&line);
        read.remove(0);
        assert_eq!(
            parse(&read),
            Ok(Some(Request {
                profile: Some("Без модов".into()),
                launch: true
            }))
        );
    }
}
