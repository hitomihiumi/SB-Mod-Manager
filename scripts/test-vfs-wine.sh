#!/usr/bin/env bash
# End-to-end check of the virtual file system hook under Wine.
#
# Builds the hook DLL, the launcher and a probe that stands in for the game
# for x86_64-pc-windows-gnu, starts the probe through the real launcher, and
# checks what it saw. Needs wine64, mingw-w64 and the windows-gnu Rust target.
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

wine "$bin/vfs_launch.exe" "Z:${work//\//\\}\\run"
wineserver -w

echo "--- hook log"; cat "$work/run/hook.log" 2>/dev/null || echo "(none)"
echo "--- probe report"; cat "$work/run/report.txt"
test ! -e "$work/run/game/SB/Saved" || { echo "FAIL: the probe wrote into the game folder"; exit 1; }
test -f "$work/run/library/overwrite/SB/Saved/Logs/probe.log" || { echo "FAIL: the new file is not in overwrite"; exit 1; }
grep -q '^RESULT PASS' "$work/run/report.txt"
echo "virtual file system: all checks passed under Wine"
