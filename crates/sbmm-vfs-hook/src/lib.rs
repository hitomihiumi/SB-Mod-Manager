//! The half of the virtual file system that runs inside the game.
//!
//! The launcher in `sbmm-vfs` starts the game suspended, loads this DLL into
//! it and calls [`sbmm_vfs_init`] before the game's first instruction runs.
//! Init reads the mapping the manager wrote, then rewrites the import tables
//! of every loaded module so their calls to the Win32 file functions come
//! here first. Each hook places the path inside the game folder, asks the
//! overlay where it really is, and calls the real function with that path —
//! the same job USVFS does for Mod Organizer 2.
//!
//! Import tables rather than inline patches: Unreal, UE4SS and the C runtime
//! all reach the file API through their imports, the patch is a pointer write
//! with nothing to disassemble, and the real functions stay untouched so the
//! hooks call them directly. What this cannot see is the Windows loader
//! itself, which resolves a proxy DLL's static imports in `ntdll` before any
//! hook exists — which is why the manager keeps loader-adjacent files in the
//! binaries folder physical, as Mod Organizer's Root Builder does.

#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod iat;

#[cfg(windows)]
pub use hooks::sbmm_vfs_init;
