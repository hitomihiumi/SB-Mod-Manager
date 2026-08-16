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

use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

/// The Discord application this presence belongs to.
///
/// **This is a placeholder and presence will not appear until it is replaced.**
/// The id can only come from registering an application at
/// <https://discord.com/developers/applications>; it decides the name and the
/// artwork Discord shows, so there is nothing sensible to guess and borrowing
/// another app's id would put somebody else's branding on the user's profile.
/// Upload an icon named `icon` to that application's Rich Presence assets to
/// match [`Assets::large_image`] below. See the README.
const APP_ID: &str = "0000000000000000000";

/// What the presence should say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub enabled: bool,
    /// How many mods are installed, and how many of those are switched on.
    pub installed: usize,
    pub enabled_mods: usize,
    /// Set while downloads are in flight.
    pub downloading: usize,
}

impl Presence {
    /// The line Discord shows under the app name.
    ///
    /// Counts only. Naming the mod being downloaded would put the contents of
    /// somebody's load order in front of their friends list.
    pub fn details(&self) -> String {
        if self.downloading > 0 {
            return format!(
                "Downloading {} mod{}",
                self.downloading,
                if self.downloading == 1 { "" } else { "s" }
            );
        }
        "Managing mods".to_string()
    }

    pub fn state(&self) -> String {
        if self.installed == 0 {
            return "Nothing installed yet".to_string();
        }
        format!("{} of {} enabled", self.enabled_mods, self.installed)
    }
}

/// A handle the app writes updates to; the work happens on its own thread.
pub struct Discord {
    updates: SyncSender<Presence>,
    last: Mutex<Option<Presence>>,
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

        Discord {
            updates: tx,
            last: Mutex::new(None),
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

        let details = presence.details();
        let state = presence.state();
        let activity = Activity::new()
            .details(&details)
            .state(&state)
            .assets(Assets::new().large_image("icon").large_text("SB Mod Manager"))
            .timestamps(Timestamps::new().start(started));

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

    fn presence(installed: usize, enabled_mods: usize, downloading: usize) -> Presence {
        Presence {
            enabled: true,
            installed,
            enabled_mods,
            downloading,
        }
    }

    #[test]
    fn the_presence_reports_counts_and_never_names_anything() {
        let p = presence(42, 17, 0);
        assert_eq!(p.details(), "Managing mods");
        assert_eq!(p.state(), "17 of 42 enabled");
    }

    #[test]
    fn downloading_is_shown_as_a_count_not_as_a_mod_name() {
        assert_eq!(presence(10, 3, 1).details(), "Downloading 1 mod");
        assert_eq!(presence(10, 3, 5).details(), "Downloading 5 mods");
    }

    #[test]
    fn an_empty_install_reads_as_empty_rather_than_zero_of_zero() {
        assert_eq!(presence(0, 0, 0).state(), "Nothing installed yet");
    }

    /// Presence is cosmetic and the socket is slow, so repeating the same
    /// state must not cost a write.
    #[test]
    fn an_unchanged_presence_is_not_published_twice() {
        let discord = Discord::start();
        let p = presence(3, 1, 0);

        discord.update(p.clone());
        let after_first = discord.last.lock().unwrap().clone();
        discord.update(p.clone());

        assert_eq!(after_first, Some(p));
    }
}
