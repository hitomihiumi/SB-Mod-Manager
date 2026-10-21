use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    build_vfs_hook();
    tauri_build::build()
}

/// Build the virtual file system's hook DLL and leave it in `OUT_DIR` for the
/// app to embed.
///
/// The hook is loaded into the game, so it cannot be linked into the app; it
/// is built by a nested cargo instead, which keeps `pnpm tauri build` one step
/// and guarantees the app never ships a hook from another build. Off Windows
/// there is nothing to load it into, so an empty file stands in.
fn build_vfs_hook() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let dest = out.join("sbmm_vfs_hook.dll");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri sits inside the repository")
        .to_path_buf();
    for watched in ["crates/sbmm-vfs-hook", "crates/sbmm-vfs"] {
        println!("cargo:rerun-if-changed={}", root.join(watched).display());
    }
    println!("cargo:rerun-if-env-changed=SBMM_SKIP_VFS_HOOK");

    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if !windows || env::var_os("SBMM_SKIP_VFS_HOOK").is_some() {
        fs::write(&dest, b"").expect("OUT_DIR is writable");
        return;
    }

    let target = env::var("TARGET").expect("cargo sets TARGET");
    let release = env::var("PROFILE").as_deref() == Ok("release");
    let target_dir = out.join("vfs-hook");
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&root)
        .args([
            "build",
            "-p",
            "sbmm-vfs-hook",
            "--target",
            &target,
            "--target-dir",
        ])
        .arg(&target_dir);
    if release {
        cargo.arg("--release");
    }
    // Settings meant for this build would otherwise leak into the nested one.
    for variable in ["CARGO_TARGET_DIR", "CARGO_ENCODED_RUSTFLAGS"] {
        cargo.env_remove(variable);
    }
    // Link the C runtime statically so the injected DLL loads inside the game
    // without the Visual C++ redistributable being present — a missing
    // vcruntime/ucrt is the usual reason an injected DLL fails to load. This
    // replaces, rather than inherits, any RUSTFLAGS from the parent build.
    cargo.env("RUSTFLAGS", "-C target-feature=+crt-static");
    let status = cargo
        .status()
        .expect("cargo can be run from a build script");
    assert!(
        status.success(),
        "building the virtual file system hook failed"
    );

    let built = target_dir
        .join(&target)
        .join(if release { "release" } else { "debug" })
        .join("sbmm_vfs_hook.dll");
    fs::copy(&built, &dest).expect("the hook DLL was built where cargo puts it");
}
