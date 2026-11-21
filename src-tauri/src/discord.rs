//! Telling Discord what the manager is doing.
//!
//! This is the one part of the app that publishes anything outward, so two
//! rules shape it. It is a single switch away from off, and it never names a
//! mod, a collection or a game folder — only counts. What somebody has
//! installed is their business, and a friends list is not the place to find
//! out.
//!
//! Discord may not be running, may be started later, or may be closed
//! mid-session. All three are ordinary, so the connection is attempted in the
//! background, failures are silent, and a dropped socket is reconnected on the
//! next update rather than retried in a loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// The Discord application this presence belongs to.
///
/// **This is a placeholder and presence will not appear until it is replaced.**
/// The id can only come from registering an application at
/// <https://discord.com/developers/applications>; it decides the name and the
/// artwork Discord shows, so there is nothing sensible to guess and borrowing
/// another app's id would put somebody else's branding on the user's profile.
/// Upload an icon named `icon` to that application's Rich Presence assets as the
/// large image. Optional keys in [`ActivityKind::small_image`] are shown as the
/// small overlay. See the README.
const APP_ID: &str = "1561824958705832067";

const MAIN_IMAGE: &str = "icon";
const MAIN_IMAGE_TEXT: &str = "Stelle — Stellar Blade Mod Manager";

/// Which part of the UI the user is on. Drives the idle line when nothing
/// urgent (downloads, pending apply, game update) is going on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Mods,
    Order,
    Conflicts,
    Files,
    Downloads,
    Collections,
    Reshade,
    SettingsGame,
    SettingsLibrary,
    SettingsNexus,
    SettingsGraphics,
    SettingsBackups,
    SettingsApp,
}

impl Screen {
    pub fn parse(raw: &str) -> Option<Screen> {
        match raw {
            "mods" => Some(Screen::Mods),
            "order" => Some(Screen::Order),
            "conflicts" => Some(Screen::Conflicts),
            "files" => Some(Screen::Files),
            "downloads" => Some(Screen::Downloads),
            "collections" => Some(Screen::Collections),
            "reshade" => Some(Screen::Reshade),
            "settings:game" => Some(Screen::SettingsGame),
            "settings:library" => Some(Screen::SettingsLibrary),
            "settings:nexus" => Some(Screen::SettingsNexus),
            "settings:graphics" => Some(Screen::SettingsGraphics),
            "settings:backups" => Some(Screen::SettingsBackups),
            "settings:app" => Some(Screen::SettingsApp),
            _ => None,
        }
    }
}

/// The single activity Discord should show after priorities are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityKind {
    Playing,
    Downloading,
    AwaitingNexus,
    Applying,
    GameUpdated,
    Setup,
    Idle(Screen),
}

impl ActivityKind {
    /// Per-activity art in the corner; `None` keeps only the main logo.
    fn small_image(self) -> Option<&'static str> {
        Some(match self {
            ActivityKind::Playing => "playing",
            ActivityKind::Downloading |ActivityKind::AwaitingNexus => "download",
            ActivityKind::Applying => "apply",
            ActivityKind::GameUpdated => "update",
            ActivityKind::Setup => "setup",
            ActivityKind::Idle(screen) => match screen {
                Screen::Mods | Screen::SettingsApp => return None,
                Screen::Order => "order",
                Screen::Conflicts => "conflicts",
                Screen::Files => "files",
                Screen::Downloads => "download",
                Screen::Collections => "collections",
                Screen::Reshade | Screen::SettingsGraphics => "graphics",
                Screen::SettingsBackups => "backup",
                Screen::SettingsNexus => "nexus",
                Screen::SettingsGame | Screen::SettingsLibrary => "setup",
            },
        })
    }

    fn details(self, presence: &Presence) -> String {
        match self {
            ActivityKind::Playing => "Playing Stellar Blade".to_string(),
            ActivityKind::Downloading => format!(
                "Downloading {} mod{}",
                presence.downloading,
                if presence.downloading == 1 { "" } else { "s" }
            ),
            ActivityKind::AwaitingNexus => format!(
                "Waiting on Nexus for {} mod{}",
                presence.awaiting_action,
                if presence.awaiting_action == 1 { "" } else { "s" }
            ),
            ActivityKind::Applying => format!(
                "Applying {} change{}",
                presence.pending,
                if presence.pending == 1 { "" } else { "s" }
            ),
            ActivityKind::GameUpdated => "Game was updated".to_string(),
            ActivityKind::Setup => "Setting up".to_string(),
            ActivityKind::Idle(screen) => match screen {
                Screen::Mods => "Managing mods".to_string(),
                Screen::Order => "Sorting load order".to_string(),
                Screen::Conflicts => "Reviewing conflicts".to_string(),
                Screen::Files => "Browsing mod files".to_string(),
                Screen::Downloads => "In the download queue".to_string(),
                Screen::Collections => "Browsing collections".to_string(),
                Screen::Reshade => "Tuning ReShade".to_string(),
                Screen::SettingsGame => "Configuring the game".to_string(),
                Screen::SettingsLibrary => "Library settings".to_string(),
                Screen::SettingsNexus => "Nexus settings".to_string(),
                Screen::SettingsGraphics => "Graphics settings".to_string(),
                Screen::SettingsBackups => "Backups & safety".to_string(),
                Screen::SettingsApp => "App settings".to_string(),
            },
        }
    }

    fn state(self, presence: &Presence) -> String {
        if let ActivityKind::Playing = self {
            return if presence.installed == 0 {
                "Without mods".to_string()
            } else {
                format!("{} of {} mods enabled", presence.enabled_mods, presence.installed)
            };
        }
        if let ActivityKind::GameUpdated = self {
            return format!(
                "{} file{} need attention",
                presence.game_update_drift,
                if presence.game_update_drift == 1 { "" } else { "s" }
            );
        }
        if let ActivityKind::Setup = self {
            return if presence.installed == 0 {
                "Nothing installed yet".to_string()
            } else {
                "Choose a game folder".to_string()
            };
        }
        if presence.installed == 0 {
            return "Nothing installed yet".to_string();
        }
        format!("{} of {} enabled", presence.enabled_mods, presence.installed)
    }
}

/// What the presence should say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub enabled: bool,
    /// How many mods are installed, and how many of those are switched on.
    pub installed: usize,
    pub enabled_mods: usize,
    pub pending: usize,
    /// Files the game changed outside the manager.
    pub drift: usize,
    pub game_configured: bool,
    pub game_update_drift: usize,
    /// Active transfers (queued or running).
    pub downloading: usize,
    /// Queue items waiting for a click on Nexus.
    pub awaiting_action: usize,
    pub screen: Screen,
    /// The game process is alive right now.
    pub game_running: bool,
}

impl Presence {
    fn activity_kind(&self) -> ActivityKind {
        if self.game_running {
            return ActivityKind::Playing;
        }
        if self.downloading > 0 {
            return ActivityKind::Downloading;
        }
        if self.awaiting_action > 0 {
            return ActivityKind::AwaitingNexus;
        }
        if self.pending > 0 {
            return ActivityKind::Applying;
        }
        if self.game_update_drift > 0 {
            return ActivityKind::GameUpdated;
        }
        if !self.game_configured {
            return ActivityKind::Setup;
        }
        ActivityKind::Idle(self.screen)
    }

    pub fn details(&self) -> String {
        self.activity_kind().details(self)
    }

    pub fn state(&self) -> String {
        self.activity_kind().state(self)
    }

    fn build_activity(&self, started: i64) -> Activity<'_> {
        let kind = self.activity_kind();
        let mut assets = Assets::new()
            .large_image(MAIN_IMAGE)
            .large_text(MAIN_IMAGE_TEXT);
        if let Some(image) = kind.small_image() {
            assets = assets.small_image(image);
        }
        Activity::new()
            .details(self.details())
            .state(self.state())
            .assets(assets)
            .timestamps(Timestamps::new().start(started))
    }
}

/// A handle the app writes updates to; the work happens on its own thread.
pub struct Discord {
    updates: SyncSender<Presence>,
    last: Arc<Mutex<Option<Presence>>>,
    screen: Mutex<Screen>,
    game_running: Arc<AtomicBool>,
}

/// Name of the game's process, compared case-insensitively.
const GAME_PROCESS: &str = "SB-Win64-Shipping.exe";

#[cfg(windows)]
fn game_process_alive() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: plain snapshot enumeration; the handle is closed on every path
    // and the entry struct is zeroed with its size field set as the API asks.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(GAME_PROCESS) {
                found = true;
                break;
            }
            ok = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
fn game_process_alive() -> bool {
    false
}

impl Discord {
    /// Start the presence worker.
    ///
    /// The channel is bounded and sends never block: presence is cosmetic, so
    /// dropping an update when the worker is busy is better than holding up
    /// whatever produced it.
    pub fn start() -> Discord {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Presence>(4);
        std::thread::Builder::new()
            .name("discord-presence".into())
            .spawn(move || worker(rx))
            .ok();

        let last: Arc<Mutex<Option<Presence>>> = Arc::new(Mutex::new(None));
        let game_running = Arc::new(AtomicBool::new(false));

        // The game can be started from Steam or a shortcut, so launching it
        // through the manager is not the only signal. Watch the process and
        // republish the last presence whenever it appears or exits.
        {
            let tx = tx.clone();
            let last = Arc::clone(&last);
            let game_running = Arc::clone(&game_running);
            std::thread::Builder::new()
                .name("discord-game-watch".into())
                .spawn(move || loop {
                    let alive = game_process_alive();
                    if game_running.swap(alive, Ordering::Relaxed) != alive {
                        if let Ok(mut guard) = last.lock() {
                            if let Some(mut presence) = guard.clone() {
                                presence.game_running = alive;
                                *guard = Some(presence.clone());
                                let _ = tx.try_send(presence);
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_secs(5));
                })
                .ok();
        }

        Discord {
            updates: tx,
            last,
            screen: Mutex::new(Screen::default()),
            game_running,
        }
    }

    pub fn game_running(&self) -> bool {
        self.game_running.load(Ordering::Relaxed)
    }

    pub fn screen(&self) -> Screen {
        self.screen.lock().map(|s| *s).unwrap_or_default()
    }

    pub fn set_screen(&self, screen: Screen) {
        if let Ok(mut guard) = self.screen.lock() {
            *guard = screen;
        }
    }

    /// Publish a new presence, if it says anything different from the last.
    pub fn update(&self, presence: Presence) {
        if let Ok(mut last) = self.last.lock() {
            if last.as_ref() == Some(&presence) {
                return;
            }
            *last = Some(presence.clone());
        }
        // Full channel means an update is already in flight; the next one
        // carries the newer state anyway.
        let _ = self.updates.try_send(presence);
    }
}

/// Owns the connection for the life of the app.
fn worker(updates: Receiver<Presence>) {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();

    let mut client: Option<DiscordIpcClient> = None;

    for presence in updates {
        if !presence.enabled {
            // Switching it off has to clear what is already showing, not just
            // stop updating it.
            if let Some(mut open) = client.take() {
                let _ = open.clear_activity();
                let _ = open.close();
            }
            continue;
        }

        if client.is_none() {
            client = connect();
        }
        let Some(open) = client.as_mut() else {
            // Discord is not running. Nothing to report and nothing to retry
            // until there is a reason to try again.
            continue;
        };

        let activity = presence.build_activity(started);

        if open.set_activity(activity).is_err() {
            // Discord went away mid-session. Drop the socket so the next
            // update reconnects instead of writing into a dead pipe.
            let _ = open.close();
            client = None;
        }
    }
}

fn connect() -> Option<DiscordIpcClient> {
    // `new` is infallible in this version; only connecting can fail, and it
    // does whenever Discord is not running, which is not an error worth
    // reporting.
    let mut client = DiscordIpcClient::new(APP_ID);
    client.connect().ok()?;
    Some(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence() -> Presence {
        Presence {
            enabled: true,
            installed: 42,
            enabled_mods: 17,
            pending: 0,
            drift: 0,
            game_configured: true,
            game_update_drift: 0,
            downloading: 0,
            awaiting_action: 0,
            screen: Screen::Mods,
            game_running: false,
        }
    }

    #[test]
    fn a_running_game_outranks_everything_else() {
        let mut p = presence();
        p.downloading = 2;
        p.pending = 1;
        p.game_running = true;
        assert_eq!(p.details(), "Playing Stellar Blade");
        assert_eq!(p.state(), "17 of 42 mods enabled");
    }

    #[test]
    fn the_presence_reports_counts_and_never_names_anything() {
        let p = presence();
        assert_eq!(p.details(), "Managing mods");
        assert_eq!(p.state(), "17 of 42 enabled");
    }

    #[test]
    fn downloading_is_shown_as_a_count_not_as_a_mod_name() {
        let mut p = presence();
        p.downloading = 1;
        assert_eq!(p.details(), "Downloading 1 mod");
        p.downloading = 5;
        assert_eq!(p.details(), "Downloading 5 mods");
    }

    #[test]
    fn an_empty_install_reads_as_empty_rather_than_zero_of_zero() {
        let mut p = presence();
        p.installed = 0;
        p.enabled_mods = 0;
        assert_eq!(p.state(), "Nothing installed yet");
    }

    #[test]
    fn pending_changes_take_priority_over_the_current_screen() {
        let mut p = presence();
        p.screen = Screen::Order;
        p.pending = 3;
        assert_eq!(p.details(), "Applying 3 changes");
    }

    #[test]
    fn the_active_screen_changes_the_idle_line() {
        let mut p = presence();
        p.screen = Screen::Collections;
        assert_eq!(p.details(), "Browsing collections");
        p.screen = Screen::SettingsGraphics;
        assert_eq!(p.details(), "Graphics settings");
    }

    #[test]
    fn game_update_uses_drift_counts_not_names() {
        let mut p = presence();
        p.game_update_drift = 2;
        assert_eq!(p.details(), "Game was updated");
        assert_eq!(p.state(), "2 files need attention");
    }

    #[test]
    fn setup_when_no_game_is_configured() {
        let mut p = presence();
        p.game_configured = false;
        assert_eq!(p.details(), "Setting up");
        assert_eq!(p.state(), "Choose a game folder");
    }

    /// Presence is cosmetic and the socket is slow, so repeating the same
    /// state must not cost a write.
    #[test]
    fn an_unchanged_presence_is_not_published_twice() {
        let discord = Discord::start();
        let p = presence();

        discord.update(p.clone());
        let after_first = discord.last.lock().unwrap().clone();
        discord.update(p.clone());

        assert_eq!(after_first, Some(p));
    }
}