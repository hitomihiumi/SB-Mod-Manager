//! The download queue.
//!
//! Nexus hands out a download URL that is valid only briefly, so the URL is
//! resolved at the moment a transfer starts rather than when it is queued. A
//! collection queued forty mods deep would otherwise find every link stale by
//! the time it reached the end.
//!
//! The queue owns no I/O of its own: it drives a [`Fetcher`] and reports through
//! a [`Sink`], which is what lets the whole thing be tested without a server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::client::{should_retry, NexusClient, NexusError, Transport};
use crate::http::{Fetcher, Progress};
use crate::ratelimit::backoff;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Queued,
    Running,
    /// Downloaded and handed to the installer.
    Done,
    Failed,
    Cancelled,
    /// A free account has to fetch this one through the website. The sink is
    /// asked to open the mod page and the item waits for an `nxm://` link.
    NeedsUserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: u64,
    pub mod_id: u64,
    pub file_id: u64,
    /// What to call it in the UI.
    pub name: String,
    /// What to call it on disk.
    pub file_name: String,
    /// The file's version, when the API could be asked. Recorded on the
    /// installed mod so a later update check has something to compare against.
    pub version: Option<String>,
    pub state: DownloadState,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub error: Option<String>,
    /// Which collection queued it, when one did.
    pub collection: Option<String>,
    #[serde(skip)]
    credentials: Option<(String, u64)>,
    #[serde(skip)]
    attempts: u32,
}

impl QueueItem {
    pub fn new(
        mod_id: u64,
        file_id: u64,
        name: impl Into<String>,
        file_name: impl Into<String>,
    ) -> Self {
        Self {
            id: 0,
            mod_id,
            file_id,
            name: name.into(),
            file_name: file_name.into(),
            version: None,
            state: DownloadState::Queued,
            bytes_done: 0,
            bytes_total: None,
            error: None,
            collection: None,
            credentials: None,
            attempts: 0,
        }
    }

    /// Attach the short-lived credentials from an `nxm://` link.
    pub fn with_credentials(mut self, key: impl Into<String>, expires: u64) -> Self {
        self.credentials = Some((key.into(), expires));
        self
    }

    pub fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }

    pub fn in_collection(mut self, slug: impl Into<String>) -> Self {
        self.collection = Some(slug.into());
        self
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }
}

/// What the queue reports back to the application.
pub trait Sink: Send + Sync {
    /// Called often while bytes arrive; keep it cheap.
    fn progress(&self, item: &QueueItem);
    /// The file is on disk and ready to install.
    fn completed(&self, item: &QueueItem, file: &Path);
    /// A free account must click Mod Manager Download on this page.
    fn needs_user_action(&self, item: &QueueItem, mod_page: &str);
}

/// How many times a transient failure is retried before giving up.
const MAX_ATTEMPTS: u32 = 4;

#[derive(Default)]
struct State {
    items: Vec<QueueItem>,
    cancelled: HashMap<u64, bool>,
}

pub struct Queue {
    state: Mutex<State>,
    next_id: AtomicU64,
    domain: String,
}

impl Queue {
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            next_id: AtomicU64::new(1),
            domain: domain.into(),
        }
    }

    /// Add an item, or top up an existing one with fresh credentials.
    ///
    /// A user clicking Mod Manager Download for something already waiting
    /// should resume that item rather than queue a duplicate.
    pub fn push(&self, mut item: QueueItem) -> u64 {
        let mut state = self.state.lock().expect("queue lock");

        if let Some(existing) = state
            .items
            .iter_mut()
            .find(|q| q.mod_id == item.mod_id && q.file_id == item.file_id && is_open(q.state))
        {
            if item.credentials.is_some() {
                existing.credentials = item.credentials;
                existing.state = DownloadState::Queued;
                existing.error = None;
                existing.attempts = 0;
            }
            return existing.id;
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        item.id = id;
        state.items.push(item);
        id
    }

    pub fn items(&self) -> Vec<QueueItem> {
        self.state.lock().expect("queue lock").items.clone()
    }

    pub fn cancel(&self, id: u64) {
        let mut state = self.state.lock().expect("queue lock");
        state.cancelled.insert(id, true);
        if let Some(item) = state.items.iter_mut().find(|i| i.id == id) {
            if is_open(item.state) {
                item.state = DownloadState::Cancelled;
            }
        }
    }

    /// Forget everything that has finished one way or another.
    pub fn clear_finished(&self) {
        let mut state = self.state.lock().expect("queue lock");
        state.items.retain(|i| is_open(i.state));
    }

    fn take_next(&self) -> Option<QueueItem> {
        let mut state = self.state.lock().expect("queue lock");
        let item = state
            .items
            .iter_mut()
            .find(|i| i.state == DownloadState::Queued)?;
        item.state = DownloadState::Running;
        Some(item.clone())
    }

    fn update(&self, id: u64, apply: impl FnOnce(&mut QueueItem)) -> Option<QueueItem> {
        let mut state = self.state.lock().expect("queue lock");
        let item = state.items.iter_mut().find(|i| i.id == id)?;
        apply(item);
        Some(item.clone())
    }

    fn is_cancelled(&self, id: u64) -> bool {
        self.state
            .lock()
            .expect("queue lock")
            .cancelled
            .get(&id)
            .copied()
            .unwrap_or(false)
    }

    /// The page a free account has to use for this file.
    fn mod_page(&self, item: &QueueItem) -> String {
        format!(
            "https://www.nexusmods.com/{}/mods/{}?tab=files&file_id={}&nmm=1",
            self.domain, item.mod_id, item.file_id
        )
    }

    /// Take one queued item to completion. Returns false when nothing was
    /// waiting, which is the driver's cue to idle.
    pub async fn step<T, F, S>(
        &self,
        client: &NexusClient<T>,
        fetcher: &F,
        sink: &S,
        into: &Path,
    ) -> bool
    where
        T: Transport,
        F: Fetcher,
        S: Sink,
    {
        let Some(item) = self.take_next() else {
            return false;
        };

        match self.run_one(client, fetcher, sink, into, &item).await {
            Ok(path) => {
                if let Some(done) = self.update(item.id, |i| i.state = DownloadState::Done) {
                    sink.completed(&done, &path);
                }
            }
            Err(Stop::Cancelled) => {
                self.update(item.id, |i| i.state = DownloadState::Cancelled);
            }
            Err(Stop::NeedsUser) => {
                // Not a failure: the website hands over a usable link when the
                // user presses the button, and push() will revive this item.
                if let Some(waiting) = self.update(item.id, |i| {
                    i.state = DownloadState::NeedsUserAction;
                    i.error = None;
                }) {
                    sink.needs_user_action(&waiting, &self.mod_page(&waiting));
                }
            }
            Err(Stop::Failed(reason)) => {
                self.update(item.id, |i| {
                    i.state = DownloadState::Failed;
                    i.error = Some(reason);
                });
            }
        }
        true
    }

    async fn run_one<T, F, S>(
        &self,
        client: &NexusClient<T>,
        fetcher: &F,
        sink: &S,
        into: &Path,
        item: &QueueItem,
    ) -> Result<PathBuf, Stop>
    where
        T: Transport,
        F: Fetcher,
        S: Sink,
    {
        let credentials = item
            .credentials
            .as_ref()
            .map(|(key, expires)| (key.as_str(), *expires));

        let mut attempt = item.attempts;
        loop {
            if self.is_cancelled(item.id) {
                return Err(Stop::Cancelled);
            }
            attempt += 1;

            // Resolved per attempt: these URLs go stale quickly.
            let url = match client
                .download_link(item.mod_id, item.file_id, credentials)
                .await
            {
                Ok(url) => url,
                Err(NexusError::PremiumRequired) | Err(NexusError::LinkExpired) => {
                    return Err(Stop::NeedsUser)
                }
                Err(err) if should_retry(&err) && attempt < MAX_ATTEMPTS => {
                    self.wait(attempt).await;
                    continue;
                }
                Err(err) => return Err(Stop::Failed(err.to_string())),
            };

            let dest = into.join(&item.file_name);
            let id = item.id;
            let report = |progress: Progress| {
                if let Some(updated) = self.update(id, |i| {
                    i.bytes_done = progress.done;
                    i.bytes_total = progress.total;
                }) {
                    sink.progress(&updated);
                }
            };

            match fetcher.fetch(&url, &dest, &report).await {
                Ok(_) => return Ok(dest),
                Err(err) if should_retry(&err) && attempt < MAX_ATTEMPTS => {
                    self.update(item.id, |i| i.attempts = attempt);
                    self.wait(attempt).await;
                }
                Err(err) => return Err(Stop::Failed(err.to_string())),
            }
        }
    }

    async fn wait(&self, attempt: u32) {
        tokio::time::sleep(backoff(attempt, None)).await;
    }
}

/// Why a transfer stopped.
enum Stop {
    Cancelled,
    NeedsUser,
    Failed(String),
}

fn is_open(state: DownloadState) -> bool {
    matches!(
        state,
        DownloadState::Queued | DownloadState::Running | DownloadState::NeedsUserAction
    )
}

/// Convenience for sharing a queue across the app.
pub type SharedQueue = Arc<Queue>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{HttpResponse, NexusError};
    use std::sync::atomic::AtomicUsize;

    struct FakeApi {
        /// Status to answer download_link with; 200 means success.
        status: u16,
        calls: AtomicUsize,
    }

    impl Transport for FakeApi {
        async fn get(&self, _url: &str, _h: &[(&str, &str)]) -> Result<HttpResponse, NexusError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            // A throttled first call, then success, exercises the retry path.
            let status = if self.status == 429 && n > 0 {
                200
            } else {
                self.status
            };
            Ok(HttpResponse {
                status,
                headers: vec![],
                body: r#"[{"URI":"http://example.invalid/f.zip"}]"#.into(),
            })
        }

        async fn post_json(
            &self,
            _url: &str,
            _h: &[(&str, &str)],
            _b: &str,
        ) -> Result<HttpResponse, NexusError> {
            unreachable!("the queue does not use graphql")
        }
    }

    struct FakeFetcher {
        payload: Vec<u8>,
        fail_first: bool,
        calls: AtomicUsize,
    }

    impl Fetcher for FakeFetcher {
        async fn fetch(
            &self,
            _url: &str,
            dest: &Path,
            on_progress: &(dyn Fn(Progress) + Send + Sync),
        ) -> Result<u64, NexusError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_first && n == 0 {
                return Err(NexusError::RateLimited);
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(dest, &self.payload).unwrap();
            on_progress(Progress {
                done: self.payload.len() as u64,
                total: Some(self.payload.len() as u64),
            });
            Ok(self.payload.len() as u64)
        }
    }

    #[derive(Default)]
    struct Recorder {
        completed: Mutex<Vec<(u64, PathBuf)>>,
        prompted: Mutex<Vec<String>>,
        progress: AtomicUsize,
    }

    impl Sink for Recorder {
        fn progress(&self, _item: &QueueItem) {
            self.progress.fetch_add(1, Ordering::SeqCst);
        }
        fn completed(&self, item: &QueueItem, file: &Path) {
            self.completed
                .lock()
                .unwrap()
                .push((item.id, file.to_path_buf()));
        }
        fn needs_user_action(&self, _item: &QueueItem, page: &str) {
            self.prompted.lock().unwrap().push(page.to_string());
        }
    }

    fn client(status: u16) -> NexusClient<FakeApi> {
        NexusClient::new(
            FakeApi {
                status,
                calls: AtomicUsize::new(0),
            },
            "key",
            "stellarblade",
        )
    }

    fn fetcher(fail_first: bool) -> FakeFetcher {
        FakeFetcher {
            payload: b"archive contents".to_vec(),
            fail_first,
            calls: AtomicUsize::new(0),
        }
    }

    #[tokio::test]
    async fn a_queued_item_downloads_and_reports_completion() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        let id = queue.push(QueueItem::new(1, 2, "Cool Outfit", "outfit.zip"));

        assert!(
            queue
                .step(&client(200), &fetcher(false), &sink, dir.path())
                .await
        );

        let completed = sink.completed.lock().unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, id);
        assert_eq!(std::fs::read(&completed[0].1).unwrap(), b"archive contents");
        assert_eq!(queue.items()[0].state, DownloadState::Done);
    }

    #[tokio::test]
    async fn an_empty_queue_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        assert!(
            !queue
                .step(
                    &client(200),
                    &fetcher(false),
                    &Recorder::default(),
                    dir.path()
                )
                .await
        );
    }

    #[tokio::test]
    async fn without_premium_the_item_waits_for_the_website() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        queue.push(QueueItem::new(42, 7, "Cool Outfit", "outfit.zip"));

        // 403 with no credentials means Premium is required.
        queue
            .step(&client(403), &fetcher(false), &sink, dir.path())
            .await;

        assert_eq!(queue.items()[0].state, DownloadState::NeedsUserAction);
        let prompted = sink.prompted.lock().unwrap();
        assert_eq!(prompted.len(), 1);
        assert!(
            prompted[0].contains("/stellarblade/mods/42") && prompted[0].contains("file_id=7"),
            "the prompt must point at the right file: {}",
            prompted[0]
        );
    }

    #[tokio::test]
    async fn an_nxm_link_revives_a_waiting_item_instead_of_duplicating_it() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        let first = queue.push(QueueItem::new(42, 7, "Cool Outfit", "outfit.zip"));
        queue
            .step(&client(403), &fetcher(false), &sink, dir.path())
            .await;

        // The user presses Mod Manager Download; the link carries credentials.
        let second = queue
            .push(QueueItem::new(42, 7, "Cool Outfit", "outfit.zip").with_credentials("abc", 1700));

        assert_eq!(second, first, "the same item must be reused");
        assert_eq!(queue.items().len(), 1);
        assert_eq!(queue.items()[0].state, DownloadState::Queued);

        assert!(
            queue
                .step(&client(200), &fetcher(false), &sink, dir.path())
                .await
        );
        assert_eq!(queue.items()[0].state, DownloadState::Done);
    }

    #[tokio::test]
    async fn a_throttled_transfer_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        queue.push(QueueItem::new(1, 2, "Cool Outfit", "outfit.zip"));

        queue
            .step(&client(200), &fetcher(true), &sink, dir.path())
            .await;

        assert_eq!(
            queue.items()[0].state,
            DownloadState::Done,
            "a 429 on the first attempt should not end the download"
        );
    }

    #[tokio::test]
    async fn progress_reaches_the_sink() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        queue.push(QueueItem::new(1, 2, "Cool Outfit", "outfit.zip"));

        queue
            .step(&client(200), &fetcher(false), &sink, dir.path())
            .await;

        assert!(sink.progress.load(Ordering::SeqCst) > 0);
        let item = &queue.items()[0];
        assert_eq!(item.bytes_done, b"archive contents".len() as u64);
        assert_eq!(item.bytes_total, Some(b"archive contents".len() as u64));
    }

    #[tokio::test]
    async fn a_cancelled_item_is_not_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        let sink = Recorder::default();
        let id = queue.push(QueueItem::new(1, 2, "Cool Outfit", "outfit.zip"));
        queue.cancel(id);

        // Cancelling takes it out of the queued set entirely.
        assert!(
            !queue
                .step(&client(200), &fetcher(false), &sink, dir.path())
                .await
        );
        assert_eq!(queue.items()[0].state, DownloadState::Cancelled);
        assert!(sink.completed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn finished_items_can_be_cleared_while_others_stay() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::new("stellarblade");
        queue.push(QueueItem::new(1, 2, "Done", "a.zip"));
        queue.push(QueueItem::new(3, 4, "Waiting", "b.zip"));

        queue
            .step(
                &client(200),
                &fetcher(false),
                &Recorder::default(),
                dir.path(),
            )
            .await;
        queue.clear_finished();

        let remaining = queue.items();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "Waiting");
    }
}
