//! Exercises the file fetcher against a real socket.
//!
//! Resume is the part worth proving: mods run to hundreds of megabytes, and
//! getting the append-vs-restart decision wrong corrupts the archive in a way
//! that only shows up much later, as a mod that will not extract.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sbmm_nexus::http::{Fetcher, Progress, ReqwestTransport};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// How the test server should behave for one connection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Honour `Range` requests with a 206.
    SupportsRange,
    /// Ignore `Range` and always send the whole body with a 200.
    IgnoresRange,
    /// Reply 429 once, then behave normally.
    ThrottleFirst,
}

async fn serve(body: Vec<u8>, behaviour: Behaviour) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));

    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let _ = handle_one(stream, body, behaviour, seen).await;
            });
        }
    });

    (format!("http://{addr}/file.bin"), handle)
}

async fn handle_one(
    mut stream: TcpStream,
    body: Vec<u8>,
    behaviour: Behaviour,
    seen: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);

    let mut range_from: Option<u64> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("range:") {
            if let Some(start) = value.trim().strip_prefix("bytes=") {
                range_from = start.split('-').next().and_then(|s| s.trim().parse().ok());
            }
        }
    }

    let attempt = seen.fetch_add(1, Ordering::SeqCst);
    if behaviour == Behaviour::ThrottleFirst && attempt == 0 {
        write_half
            .write_all(
                b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 1\r\n\r\n",
            )
            .await?;
        return Ok(());
    }

    let honour = behaviour == Behaviour::SupportsRange;
    let (status, slice) = match range_from {
        Some(from) if honour && (from as usize) < body.len() => {
            ("206 Partial Content", &body[from as usize..])
        }
        _ => ("200 OK", &body[..]),
    };

    write_half
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                slice.len()
            )
            .as_bytes(),
        )
        .await?;
    write_half.write_all(slice).await?;
    write_half.flush().await
}

fn transport() -> ReqwestTransport {
    ReqwestTransport::new("SB Mod Manager test").unwrap()
}

async fn fetch_to(url: &str, dest: &Path) -> Result<u64, sbmm_nexus::NexusError> {
    transport().fetch(url, dest, &|_: Progress| {}).await
}

#[tokio::test]
async fn downloads_a_whole_file() {
    let body: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let (url, server) = serve(body.clone(), Behaviour::SupportsRange).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("nested/mod.zip");

    let written = fetch_to(&url, &dest).await.unwrap();
    server.abort();

    assert_eq!(written, body.len() as u64);
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        body,
        "content must match byte for byte"
    );
}

#[tokio::test]
async fn reports_progress_towards_a_known_total() {
    let body: Vec<u8> = vec![7u8; 4096];
    let (url, server) = serve(body.clone(), Behaviour::SupportsRange).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("mod.zip");

    let last = Arc::new(std::sync::Mutex::new(Progress {
        done: 0,
        total: None,
    }));
    let sink = Arc::clone(&last);
    transport()
        .fetch(&url, &dest, &move |p: Progress| {
            *sink.lock().unwrap() = p;
        })
        .await
        .unwrap();
    server.abort();

    let final_progress = *last.lock().unwrap();
    assert_eq!(final_progress.done, body.len() as u64);
    assert_eq!(final_progress.total, Some(body.len() as u64));
}

#[tokio::test]
async fn resumes_from_a_partial_file() {
    let body: Vec<u8> = (0..8000u32).map(|i| (i % 253) as u8).collect();
    let (url, server) = serve(body.clone(), Behaviour::SupportsRange).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("mod.zip");
    // Pretend an earlier attempt got a third of the way.
    std::fs::write(&dest, &body[..2600]).unwrap();

    fetch_to(&url, &dest).await.unwrap();
    server.abort();

    assert_eq!(
        std::fs::read(&dest).unwrap(),
        body,
        "the resumed half must join up with what was already there"
    );
}

#[tokio::test]
async fn a_server_that_ignores_range_restarts_cleanly() {
    let body: Vec<u8> = (0..6000u32).map(|i| (i % 249) as u8).collect();
    let (url, server) = serve(body.clone(), Behaviour::IgnoresRange).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("mod.zip");
    std::fs::write(&dest, &body[..1500]).unwrap();

    fetch_to(&url, &dest).await.unwrap();
    server.abort();

    assert_eq!(
        std::fs::read(&dest).unwrap(),
        body,
        "a 200 means the whole file came again, so the partial must be discarded, not appended to"
    );
}

#[tokio::test]
async fn throttling_is_surfaced_rather_than_written_to_disk() {
    let (url, server) = serve(vec![1u8; 100], Behaviour::ThrottleFirst).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("mod.zip");

    let err = fetch_to(&url, &dest).await.unwrap_err();
    assert!(
        matches!(err, sbmm_nexus::NexusError::RateLimited),
        "got {err:?}"
    );

    // The retry succeeds, which is what the queue's backoff relies on.
    fetch_to(&url, &dest).await.unwrap();
    server.abort();
    assert_eq!(std::fs::read(&dest).unwrap().len(), 100);
}
