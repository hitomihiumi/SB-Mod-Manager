//! Where the manager meets the operating system's shell: what it was asked to
//! do on its command line, and the shortcuts that ask it.
//!
//! Mod Organizer 2 lets a desktop shortcut start the game with a chosen
//! profile without opening the manager first. The same is done here with two
//! arguments — `--profile <name>` and `--launch` — and a shortcut is nothing
//! more than the manager's executable with those arguments.

pub mod args;
pub mod shortcut;

pub use args::{arguments, command_line, parse, quote, Request};
pub use shortcut::{create, file_name, Shortcut};
