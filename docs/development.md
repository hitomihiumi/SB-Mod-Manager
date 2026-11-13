# Development

Requires Rust 1.82+, Node 22+ and pnpm; on Windows, the WebView2 runtime.
See [Architecture](architecture.md) for how the crates fit together.

## Run and build

```sh
pnpm install
pnpm tauri dev      # run it
pnpm tauri build    # produce an installer
```

## Checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
pnpm typecheck
```

CI runs the same checks, plus the virtual file system end to end on Windows
and a Windows build of the app.

## Tools

```sh
scripts/test-vfs-wine.sh       # VFS end to end under Wine (wine64, mingw-w64)
cargo run -p sbmm-app --example seed -- <data-dir> <game-root> [library-root]   # fake install for the UI
python3 scripts/make-icons.py  # regenerate icons from docs/logo.png (needs Pillow)
```

## Against real releases

Download or unpack them first, then point the tests at them:

```sh
SBMM_UE4SS_ARCHIVE=/path/UE4SS_v3.1.0-6.zip cargo test -p sbmm-app --test lifecycle real_ue4ss
SBMM_SM86_PACKAGE=/path/unpacked cargo test -p sbmm-upscaler --test framegen_release
```

## Building without the hook

`src-tauri` embeds the virtual file system hook DLL. Set
`SBMM_SKIP_VFS_HOOK=1` to skip building it.
