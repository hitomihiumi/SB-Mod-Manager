//! A stand-in for the game, for checking the hook end to end.
//!
//! Started by `vfs_launch` under the virtual file system, it does what the
//! game does — opens paks, lists folders, writes a log — through ordinary
//! file calls, and records whether each one saw the overlay. See
//! `scripts/test-vfs-wine.sh`.

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game), Some(report)) = (args.next(), args.next()) else {
        eprintln!("usage: vfs_probe <game root> <report file>");
        std::process::exit(2);
    };
    let game = std::path::PathBuf::from(game);
    let mut lines = Vec::new();
    let mut failed = false;
    let mut check = |name: &str, ok: bool, detail: String| {
        failed |= !ok;
        lines.push(format!(
            "{} {name}: {detail}",
            if ok { "PASS" } else { "FAIL" }
        ));
    };

    // A pak only a mod provides, opened by its game path.
    let pak = game.join(r"SB\Content\Paks\~mods\0010_Outfit_P.pak");
    let read = std::fs::read_to_string(&pak);
    check(
        "mod file is readable at its game path",
        read.as_deref().ok() == Some("outfit from the mod"),
        format!("{read:?}"),
    );

    // A game file the mod replaces.
    let movie = game.join(r"SB\Content\Movies\Intro.mp4");
    let read = std::fs::read_to_string(&movie);
    check(
        "replaced game file comes from the mod",
        read.as_deref().ok() == Some("modded intro"),
        format!("{read:?}"),
    );

    // A game file nobody replaces.
    let vanilla = game.join(r"SB\Content\Paks\pakchunk0-WindowsNoEditor.pak");
    let read = std::fs::read_to_string(&vanilla);
    check(
        "untouched game file passes through",
        read.as_deref().ok() == Some("vanilla"),
        format!("{read:?}"),
    );

    // Listing a folder that exists only in the overlay.
    let listing: Result<Vec<String>, _> = std::fs::read_dir(game.join(r"SB\Content\Paks\~mods"))
        .map(|dir| {
            let mut names: Vec<String> = dir
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        });
    check(
        "virtual folder lists the mod's files",
        listing.as_ref().ok()
            == Some(&vec![
                "0010_Outfit_P.pak".to_string(),
                "0010_Outfit_P.ucas".to_string(),
            ]),
        format!("{listing:?}"),
    );

    // Listing a real folder the overlay adds to.
    let listing: Result<Vec<String>, _> =
        std::fs::read_dir(game.join(r"SB\Content\Paks")).map(|dir| {
            let mut names: Vec<String> = dir
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        });
    check(
        "real folder lists real and virtual entries once each",
        listing.as_ref().ok()
            == Some(&vec![
                "pakchunk0-WindowsNoEditor.pak".to_string(),
                "~mods".to_string(),
            ]),
        format!("{listing:?}"),
    );

    let meta = std::fs::metadata(game.join(r"SB\Content\Paks\~mods"));
    check(
        "virtual folder is a folder",
        meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
        format!("{:?}", meta.map(|m| m.is_dir())),
    );

    // Writing a new file: it must work, read back, and stay out of the game.
    let log = game.join(r"SB\Saved\Logs\probe.log");
    let made = std::fs::create_dir_all(log.parent().unwrap())
        .and_then(|_| std::fs::write(&log, "written by the game"));
    let back = std::fs::read_to_string(&log);
    check(
        "a new file can be written and read back",
        made.is_ok() && back.as_deref().ok() == Some("written by the game"),
        format!("{made:?} {back:?}"),
    );

    lines.push(if failed { "RESULT FAIL" } else { "RESULT PASS" }.to_string());
    let _ = std::fs::write(&report, lines.join("\n") + "\n");
    std::process::exit(if failed { 1 } else { 0 });
}
