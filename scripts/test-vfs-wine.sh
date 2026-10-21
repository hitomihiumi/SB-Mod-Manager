#!/usr/bin/env bash
# End-to-end check of the virtual file system hook under Wine.
#
# Builds the hook DLL, the launcher and a probe that stands in for the game
# for x86_64-pc-windows-gnu and runs the launcher, which starts the probe
# under the hook and checks what it saw. CI runs the same test natively on
# Windows. Needs wine64, mingw-w64 and the windows-gnu Rust target.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
target=x86_64-pc-windows-gnu
work="${SBMM_VFS_WINE_DIR:-/tmp/sbmm-vfs-wine}"
export WINEDEBUG=-all
# Wine 9 aborts in its own start-up when TMPDIR is set.
unset TMPDIR
export WINEPREFIX="${WINEPREFIX:-$work/prefix}"

cargo build --manifest-path "$root/Cargo.toml" --target "$target" \
    -p sbmm-vfs-hook -p sbmm-vfs --examples --lib
out="$root/target/$target/debug"
bin="$work/bin"
rm -rf "$work/run" "$bin" && mkdir -p "$bin" "$work/run"
cp "$out/sbmm_vfs_hook.dll" "$out/examples/vfs_probe.exe" "$out/examples/vfs_launch.exe" "$bin/"

# vfs_launch waits for the probe and checks everything itself; its exit code
# is the result. The same binary is the Windows CI test.
status=0
wine "$bin/vfs_launch.exe" "Z:${work//\//\\}\\run" || status=$?
wineserver -w
exit "$status"
