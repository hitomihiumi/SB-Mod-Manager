//! The real HTTP implementations, over `reqwest`.
//!
//! Kept apart from [`crate::client`] so the client's logic can be tested
//! against canned responses without any network at all.

use std::path::Path;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::client::{HttpResponse, NexusError, Transport};

/// Shared `reqwest` client. Building one per request would throw away
/// connection pooling, which matters when a collection queues forty downloads.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(user_agent: &str) -> Result<Self, NexusError> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .build()
            .map_err(|e| NexusError::Network(e.to_string()))?;
        Ok(Self { client })
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.client
    }
}

impl Transport for ReqwestTransport {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, NexusError> {
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| NexusError::Network(e.to_string()))?;
        into_response(response).await
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, NexusError> {
        let mut request = self.client.post(url).body(body.to_string());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| NexusError::Network(e.to_string()))?;
        into_response(response).await
    }
}

async fn into_response(response: reqwest::Response) -> Result<HttpResponse, NexusError> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let body = response
        .text()
        .await
        .map_err(|e| NexusError::Network(e.to_string()))?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Progress of one file transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    /// `None` when the server did not say how big the file is.
    pub total: Option<u64>,
}

/// Fetching a file to disk. A trait so the queue can be tested without a
/// server, and so a future backend can replace it.
#[allow(async_fn_in_trait)]
pub trait Fetcher: Send + Sync {
    /// Append to `dest`, resuming from its current length when the server
    /// allows it. `on_progress` is called as bytes arrive.
    async fn fetch(
        &self,
        url: &str,
        dest: &Path,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<u64, NexusError>;
}

impl Fetcher for ReqwestTransport {
    async fn fetch(
        &self,
        url: &str,
        dest: &Path,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<u64, NexusError> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| NexusError::Network(e.to_string()))?;
        }

        // A part-finished file from an interrupted run is resumed rather than
        // fetched again; large mods are hundreds of megabytes.
        let already = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let mut request = self.client.get(url);
        if already > 0 {
            request = request.header("Range", format!("bytes={already}-"));
        }
        let response = request
            .send()
            .await
            .map_err(|e| NexusError::Network(e.to_string()))?;

        let status = response.status().as_u16();
        if status == 429 {
            return Err(NexusError::RateLimited);
        }
        if !(200..300).contains(&status) {
            return Err(NexusError::Http {
                status,
                body: response.text().await.unwrap_or_default(),
            });
        }

        // 206 means the range was honoured; 200 means the server sent the whole
        // file, so anything already on disk has to be discarded.
        let resuming = status == 206 && already > 0;
        let total = response
            .content_length()
            .map(|len| len + if resuming { already } else { 0 });

        let mut file = if resuming {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(dest)
                .await
                .map_err(|e| NexusError::Network(e.to_string()))?
        } else {
            tokio::fs::File::create(dest)
                .await
                .map_err(|e| NexusError::Network(e.to_string()))?
        };

        let mut done = if resuming { already } else { 0 };
        on_progress(Progress { done, total });

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| NexusError::Network(e.to_string()))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| NexusError::Network(e.to_string()))?;
            done += chunk.len() as u64;
            on_progress(Progress { done, total });
        }
        file.flush()
            .await
            .map_err(|e| NexusError::Network(e.to_string()))?;

        Ok(done)
    }
}
