//! Lay out a fake game and mod, start `vfs_probe` under the virtual file
//! system with the real launcher and hook, wait for its report and check it.
//! Exits non-zero on any failure, printing both logs, so the same binary is
//! the end-to-end test on Windows CI and under Wine
//! (`scripts/test-vfs-wine.sh`).

fn main() {
    #[cfg(windows)]
    {
        if let Err(error) = run() {
            eprintln!("vfs_launch: {error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("vfs_launch only runs on Windows");
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::fs;

    use sbmm_vfs::launch::{launch, LaunchOptions};
    use sbmm_vfs::{Layer, LayerFile, Mapping, Origin};

    let work = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: vfs_launch <work dir>")?,
    );
    let here = std::env::current_exe()?.parent().unwrap().to_path_buf();
    let _ = fs::remove_dir_all(&work);

    let game = work.join("game");
    let library = work.join("library");
    fs::create_dir_all(game.join(r"SB\Content\Paks"))?;
    fs::create_dir_all(game.join(r"SB\Content\Movies"))?;
    fs::write(
        game.join(r"SB\Content\Paks\pakchunk0-WindowsNoEditor.pak"),
        "vanilla",
    )?;
    fs::write(game.join(r"SB\Content\Movies\Intro.mp4"), "original intro")?;

    let staged = library.join(r"mods\Outfit");
    fs::create_dir_all(&staged)?;
    fs::write(staged.join("Outfit_P.pak"), "outfit from the mod")?;
    fs::write(staged.join("Outfit_P.ucas"), "ucas")?;
    fs::write(staged.join("Intro.mp4"), "modded intro")?;

    let file = |target: &str, source: &str| LayerFile {
        target: target.into(),
        source: staged.join(source),
    };
    let mapping = Mapping::new(
        &game,
        library.join("overwrite"),
        vec![Layer {
            origin: Origin::Mod(1),
            name: "Outfit".into(),
            files: vec![
                file(r"SB\Content\Paks\~mods\0010_Outfit_P.pak", "Outfit_P.pak"),
                file(r"SB\Content\Paks\~mods\0010_Outfit_P.ucas", "Outfit_P.ucas"),
                file(r"SB\Content\Movies\Intro.mp4", "Intro.mp4"),
            ],
        }],
    );
    let mapping_path = library.join(r"vfs\mapping.json");
    mapping.save(&mapping_path)?;

    let report = work.join("report.txt");
    let launched = launch(&LaunchOptions {
        executable: here.join("vfs_probe.exe"),
        working_dir: work.clone(),
        arguments: vec![
            game.to_string_lossy().into_owned(),
            report.to_string_lossy().into_owned(),
        ],
        environment: Vec::new(),
        hook: here.join("sbmm_vfs_hook.dll"),
        mapping: mapping_path,
        log: Some(work.join("hook.log")),
        diagnostics: Some(work.join("launch.log")),
    });
    let pid = match launched {
        Ok(pid) => pid,
        Err(error) => {
            print_logs(&work);
            return Err(error.into());
        }
    };
    println!("launched probe as process {pid}");

    // The probe writes its report last, ending with a RESULT line.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let text = loop {
        if let Ok(text) = fs::read_to_string(&report) {
            if text.contains("RESULT ") {
                break text;
            }
        }
        if std::time::Instant::now() > deadline {
            print_logs(&work);
            return Err("the probe did not report within 60 seconds".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    println!("--- probe report\n{text}");

    let mut failures = Vec::new();
    if !text.contains("RESULT PASS") {
        failures.push("the probe saw the wrong files");
    }
    if game.join(r"SB\Saved").exists() {
        failures.push("the probe wrote into the game folder");
    }
    if !library.join(r"overwrite\SB\Saved\Logs\probe.log").is_file() {
        failures.push("the new file is not in overwrite");
    }
    if !failures.is_empty() {
        print_logs(&work);
        return Err(failures.join("; ").into());
    }
    println!("virtual file system: all checks passed");
    Ok(())
}

#[cfg(windows)]
fn print_logs(work: &std::path::Path) {
    for name in ["launch.log", "hook.log"] {
        let text =
            std::fs::read_to_string(work.join(name)).unwrap_or_else(|_| "(not written)\n".into());
        println!("--- {name}\n{text}");
    }
}
