//! Lay out a fake game and mod, then start `vfs_probe` under the virtual file
//! system with the real launcher and hook. Windows (or Wine) only; see
//! `scripts/test-vfs-wine.sh`.

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
    let pid = launch(&LaunchOptions {
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
    })?;
    println!("launched probe as process {pid}");
    Ok(())
}
