//! Wiring the download queue into the application.
//!
//! The queue itself lives in `sbmm-nexus` and knows nothing about Tauri or the
//! installer. This module supplies the two things it needs from the outside: a
//! sink that reports to the window, and a driver that keeps stepping it.
//!
//! Installing happens on a blocking thread rather than inline in the sink,
//! because extracting a few hundred megabytes would otherwise stall an async
//! worker for the duration — and because the installer takes the `App` lock,
//! which every other command needs too.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sbmm_app::Origin;
use sbmm_nexus::queue::{Queue, QueueItem, Sink};
use sbmm_nexus::{NexusClient, ReqwestTransport};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::{AppState, USER_AGENT};

/// Files that finished downloading and are waiting to be installed.
type Finished = Arc<Mutex<Vec<(QueueItem, PathBuf)>>>;

pub struct Downloads {
    pub queue: Arc<Queue>,
    finished: Finished,
}

impl Downloads {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Queue::new(sbmm_game::NEXUS_DOMAIN)),
            finished: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

struct WindowSink {
    app: tauri::AppHandle,
    finished: Finished,
}

impl Sink for WindowSink {
    fn progress(&self, item: &QueueItem) {
        let _ = self.app.emit("download-progress", item);
    }

    fn completed(&self, item: &QueueItem, file: &std::path::Path) {
        if let Ok(mut queued) = self.finished.lock() {
            queued.push((item.clone(), file.to_path_buf()));
        }
        let _ = self.app.emit("downloads-changed", ());
    }

    fn needs_user_action(&self, item: &QueueItem, mod_page: &str) {
        // Opening the page is the whole point: pressing Mod Manager Download
        // there sends back an nxm:// link carrying the credentials a free
        // account needs, and the queue picks the item straight back up.
        let _ = self.app.opener().open_url(mod_page, None::<&str>);
        let _ = self.app.emit("download-needs-action", item);
        let _ = self.app.emit("downloads-changed", ());
    }
}

/// Keep stepping the queue for as long as the app runs.
pub fn spawn_driver(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let (queue, finished) = {
            let downloads = app.state::<Downloads>();
            (
                Arc::clone(&downloads.queue),
                Arc::clone(&downloads.finished),
            )
        };
        let sink = WindowSink {
            app: app.clone(),
            finished: Arc::clone(&finished),
        };

        let Ok(transport) = ReqwestTransport::new(USER_AGENT) else {
            let _ = app.emit(
                "download-install-failed",
                ("", "could not start the HTTP client; downloads are disabled"),
            );
            return;
        };

        loop {
            let installed = install_finished(&app, &finished).await;

            // Without a key even a free account cannot ask for a link, so the
            // queue is left alone until one is entered.
            let key = current_api_key(&app);
            let stepped = if key.is_empty() {
                false
            } else {
                let client = NexusClient::new(transport.clone(), key, sbmm_game::NEXUS_DOMAIN);
                let into = downloads_dir(&app);
                queue.step(&client, &transport, &sink, &into).await
            };

            if !stepped && !installed {
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
        }
    });
}

/// Hand every finished file to the installer, off the async workers.
///
/// Returns whether anything was installed, so the driver knows it made
/// progress even when the queue itself had nothing left to do.
async fn install_finished(app: &tauri::AppHandle, finished: &Finished) -> bool {
    let batch: Vec<(QueueItem, PathBuf)> = match finished.lock() {
        Ok(mut queued) => std::mem::take(&mut *queued),
        Err(_) => return false,
    };
    if batch.is_empty() {
        return false;
    }

    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        for (item, path) in batch {
            let outcome = {
                let state = app.state::<AppState>();
                let Ok(mut guard) = state.0.lock() else {
                    return;
                };
                let origin = Origin::nexus(item.mod_id as i64, item.file_id as i64)
                    .with_version(item.version.clone());
                guard.install_download(&path, &item.name, origin)
            };

            match outcome {
                Ok(_) => {
                    // The archive has been extracted into the library; keeping
                    // it would double the space every Nexus mod costs.
                    let _ = std::fs::remove_file(&path);
                }
                Err(error) => {
                    let _ = app.emit(
                        "download-install-failed",
                        (item.name.clone(), error.to_string()),
                    );
                }
            }
        }
        let _ = app.emit("downloads-changed", ());
    })
    .await;
    true
}

fn current_api_key(app: &tauri::AppHandle) -> String {
    let state = app.state::<AppState>();
    let Ok(guard) = state.0.lock() else {
        return String::new();
    };
    guard.nexus_api_key().ok().flatten().unwrap_or_default()
}

fn downloads_dir(app: &tauri::AppHandle) -> PathBuf {
    let state = app.state::<AppState>();
    let Ok(guard) = state.0.lock() else {
        return PathBuf::from(".");
    };
    guard.downloads_dir()
}
