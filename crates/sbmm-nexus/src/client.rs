//! The Nexus Mods REST v1 client.
//!
//! HTTP sits behind [`Transport`] so every call can be exercised against
//! canned responses — the API host is not reachable from CI, and even where it
//! is, tests that depend on somebody's live account are not worth having.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::ratelimit::{self, RateLimit};

pub const API_BASE: &str = "https://api.nexusmods.com";

#[derive(Debug, thiserror::Error)]
pub enum NexusError {
    #[error("no API key has been set")]
    NoApiKey,
    #[error("the API key was rejected")]
    BadApiKey,
    #[error(
        "a direct download link needs Nexus Premium. Use the Mod Manager Download button on the \
         mod page and the manager will pick it up."
    )]
    PremiumRequired,
    #[error("the download link has expired; start the download again from the mod page")]
    LinkExpired,
    #[error("rate limited by Nexus; try again later")]
    RateLimited,
    #[error("nexus returned {status}: {body}")]
    Http { status: u16, body: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("could not read the response: {0}")]
    Decode(String),
}

/// One HTTP response, reduced to what the client needs.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// The HTTP surface the client needs. Implemented over `reqwest` in
/// [`crate::http`], and over a fixture map in tests.
#[allow(async_fn_in_trait)]
pub trait Transport: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, NexusError>;
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<HttpResponse, NexusError>;
}

// -- API models -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "is_premium", alias = "is_premium?", default)]
    pub is_premium: bool,
    #[serde(rename = "user_id", default)]
    pub user_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    #[serde(rename = "mod_id")]
    pub mod_id: u64,
    pub name: Option<String>,
    pub version: Option<String>,
    pub summary: Option<String>,
    #[serde(rename = "picture_url")]
    pub picture_url: Option<String>,
    #[serde(rename = "category_id")]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    #[serde(rename = "file_id")]
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
    #[serde(rename = "file_name")]
    pub file_name: String,
    #[serde(rename = "size_in_bytes")]
    pub size_in_bytes: Option<u64>,
    #[serde(rename = "category_name")]
    pub category_name: Option<String>,
    #[serde(rename = "is_primary", default)]
    pub is_primary: bool,
}

#[derive(Debug, Deserialize)]
struct FilesResponse {
    files: Vec<ModFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct DownloadLink {
    #[serde(rename = "URI")]
    uri: String,
    #[serde(rename = "short_name")]
    short_name: Option<String>,
}

/// A mod whose files changed inside the queried window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatedMod {
    #[serde(rename = "mod_id")]
    pub mod_id: u64,
    #[serde(rename = "latest_file_update")]
    pub latest_file_update: i64,
}

// -- client -----------------------------------------------------------------

pub struct NexusClient<T: Transport> {
    transport: T,
    api_key: String,
    domain: String,
    app_name: String,
    app_version: String,
    limit: Mutex<RateLimit>,
}

impl<T: Transport> NexusClient<T> {
    pub fn new(transport: T, api_key: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            transport,
            api_key: api_key.into(),
            domain: domain.into(),
            app_name: "SB Mod Manager".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            limit: Mutex::new(RateLimit::default()),
        }
    }

    /// The allowance as of the last call.
    pub fn rate_limit(&self) -> RateLimit {
        *self.limit.lock().expect("rate limit lock")
    }

    /// Confirm the key works and report who it belongs to.
    ///
    /// Premium status decides whether collections can install unattended, so
    /// it is read here once rather than inferred from a failure later.
    pub async fn validate(&self) -> Result<Account, NexusError> {
        let body = self.get_v1("/v1/users/validate.json").await?;
        serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))
    }

    pub async fn mod_info(&self, mod_id: u64) -> Result<ModInfo, NexusError> {
        let path = format!("/v1/games/{}/mods/{mod_id}.json", self.domain);
        let body = self.get_v1(&path).await?;
        serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))
    }

    pub async fn mod_files(&self, mod_id: u64) -> Result<Vec<ModFile>, NexusError> {
        let path = format!("/v1/games/{}/mods/{mod_id}/files.json", self.domain);
        let body = self.get_v1(&path).await?;
        let parsed: FilesResponse =
            serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))?;
        Ok(parsed.files)
    }

    /// Ask for a URL to download a file from.
    ///
    /// Without Premium the API only answers when the request carries the
    /// `key`/`expires` pair from an `nxm://` link, so a missing pair is
    /// reported as [`NexusError::PremiumRequired`] rather than a bare 403.
    pub async fn download_link(
        &self,
        mod_id: u64,
        file_id: u64,
        credentials: Option<(&str, u64)>,
    ) -> Result<String, NexusError> {
        let mut path = format!(
            "/v1/games/{}/mods/{mod_id}/files/{file_id}/download_link.json",
            self.domain
        );
        if let Some((key, expires)) = credentials {
            path.push_str(&format!("?key={key}&expires={expires}"));
        }

        let body = match self.get_v1(&path).await {
            Ok(body) => body,
            Err(NexusError::Http { status: 403, body }) => {
                // 403 here means one of two very different things.
                return Err(if credentials.is_some() {
                    NexusError::LinkExpired
                } else {
                    let _ = body;
                    NexusError::PremiumRequired
                });
            }
            Err(other) => return Err(other),
        };

        let links: Vec<DownloadLink> =
            serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))?;
        links
            .into_iter()
            .next()
            .map(|link| {
                let _ = &link.short_name;
                link.uri
            })
            .ok_or_else(|| NexusError::Decode("no download links in the response".into()))
    }

    /// Mods whose files changed recently. `period` is one of `1d`, `1w`, `1m`.
    pub async fn updated(&self, period: &str) -> Result<Vec<UpdatedMod>, NexusError> {
        let path = format!(
            "/v1/games/{}/mods/updated.json?period={period}",
            self.domain
        );
        let body = self.get_v1(&path).await?;
        serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))
    }

    /// Look up one revision of a collection.
    ///
    /// Revision `0` means "whatever is current": the API treats a missing
    /// revision that way, and a bare collection link carries no number.
    pub async fn collection_revision(
        &self,
        slug: &str,
        revision: u64,
    ) -> Result<crate::collection::Revision, NexusError> {
        let variables = serde_json::json!({
            "slug": slug,
            "revision": revision,
            "domain": self.domain,
        });
        let data = self
            .graphql(crate::collection::REVISION_QUERY, variables)
            .await?;
        Ok(crate::collection::parse_revision(&data))
    }

    /// Run a GraphQL query against the v2 API, used for collections.
    pub async fn graphql(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, NexusError> {
        let payload = serde_json::json!({ "query": query, "variables": variables });
        let url = format!("{API_BASE}/v2/graphql");
        let headers = self.headers();
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let response = self
            .transport
            .post_json(&url, &borrowed, &payload.to_string())
            .await?;
        self.record_limits(&response);
        let body = self.check(response)?;

        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| NexusError::Decode(e.to_string()))?;
        if let Some(errors) = parsed.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let messages: Vec<String> = errors
                    .iter()
                    .map(|e| {
                        e.get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error")
                            .to_string()
                    })
                    .collect();
                return Err(NexusError::Decode(messages.join("; ")));
            }
        }
        Ok(parsed
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    async fn get_v1(&self, path: &str) -> Result<String, NexusError> {
        if self.api_key.is_empty() {
            return Err(NexusError::NoApiKey);
        }
        let url = format!("{API_BASE}{path}");
        let headers = self.headers();
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let response = self.transport.get(&url, &borrowed).await?;
        self.record_limits(&response);
        self.check(response)
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("apikey".into(), self.api_key.clone()),
            ("Application-Name".into(), self.app_name.clone()),
            ("Application-Version".into(), self.app_version.clone()),
            ("Accept".into(), "application/json".into()),
            ("Content-Type".into(), "application/json".into()),
        ]
    }

    fn record_limits(&self, response: &HttpResponse) {
        let fresh = RateLimit::from_headers(
            response
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        self.limit.lock().expect("rate limit lock").merge(fresh);
    }

    fn check(&self, response: HttpResponse) -> Result<String, NexusError> {
        match response.status {
            200..=299 => Ok(response.body),
            401 => Err(NexusError::BadApiKey),
            429 => Err(NexusError::RateLimited),
            status => Err(NexusError::Http {
                status,
                body: response.body,
            }),
        }
    }
}

/// Whether an error is worth another attempt.
pub fn should_retry(error: &NexusError) -> bool {
    match error {
        NexusError::RateLimited | NexusError::Network(_) => true,
        NexusError::Http { status, .. } => ratelimit::is_retryable(*status),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Replays canned responses keyed by the path of the requested URL.
    #[derive(Default)]
    struct Fake {
        responses: HashMap<String, HttpResponse>,
    }

    impl Fake {
        fn with(mut self, path: &str, status: u16, body: &str) -> Self {
            self.responses.insert(
                path.to_string(),
                HttpResponse {
                    status,
                    headers: vec![("X-RL-Hourly-Remaining".into(), "42".into())],
                    body: body.into(),
                },
            );
            self
        }
    }

    impl Transport for Fake {
        async fn get(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<HttpResponse, NexusError> {
            let path = url.trim_start_matches(API_BASE);
            self.responses
                .get(path)
                .cloned()
                .ok_or_else(|| NexusError::Network(format!("no fixture for {path}")))
        }

        async fn post_json(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
            _body: &str,
        ) -> Result<HttpResponse, NexusError> {
            let path = url.trim_start_matches(API_BASE);
            self.responses
                .get(path)
                .cloned()
                .ok_or_else(|| NexusError::Network(format!("no fixture for {path}")))
        }
    }

    fn client(fake: Fake) -> NexusClient<Fake> {
        NexusClient::new(fake, "test-key", "stellarblade")
    }

    #[tokio::test]
    async fn validate_reports_the_account_and_premium_status() {
        let fake = Fake::default().with(
            "/v1/users/validate.json",
            200,
            r#"{"name":"tester","is_premium":true,"user_id":7}"#,
        );

        let account = client(fake).validate().await.unwrap();
        assert_eq!(account.name, "tester");
        assert!(account.is_premium);
        assert_eq!(account.user_id, 7);
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_as_such() {
        let fake = Fake::default().with("/v1/users/validate.json", 401, "{}");
        assert!(matches!(
            client(fake).validate().await,
            Err(NexusError::BadApiKey)
        ));
    }

    #[tokio::test]
    async fn rate_limit_headers_are_recorded() {
        let fake = Fake::default().with("/v1/users/validate.json", 200, r#"{"name":"t"}"#);
        let client = client(fake);
        client.validate().await.unwrap();
        assert_eq!(client.rate_limit().hourly_remaining, Some(42));
    }

    #[tokio::test]
    async fn file_listings_are_unwrapped() {
        let fake = Fake::default().with(
            "/v1/games/stellarblade/mods/5/files.json",
            200,
            r#"{"files":[{"file_id":9,"name":"Main","file_name":"main.zip","size_in_bytes":10,"is_primary":true}]}"#,
        );

        let files = client(fake).mod_files(5).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_id, 9);
        assert!(files[0].is_primary);
    }

    #[tokio::test]
    async fn a_premium_account_gets_a_direct_link() {
        let fake = Fake::default().with(
            "/v1/games/stellarblade/mods/5/files/9/download_link.json",
            200,
            r#"[{"URI":"https://cdn.example/file.zip","short_name":"CDN"}]"#,
        );

        let url = client(fake).download_link(5, 9, None).await.unwrap();
        assert_eq!(url, "https://cdn.example/file.zip");
    }

    #[tokio::test]
    async fn without_premium_the_error_explains_the_way_through() {
        let fake = Fake::default().with(
            "/v1/games/stellarblade/mods/5/files/9/download_link.json",
            403,
            "forbidden",
        );

        let err = client(fake).download_link(5, 9, None).await.unwrap_err();
        assert!(matches!(err, NexusError::PremiumRequired));
        assert!(
            err.to_string().contains("Mod Manager Download"),
            "the message must point at the button that works: {err}"
        );
    }

    #[tokio::test]
    async fn an_nxm_key_that_no_longer_works_reads_as_expired() {
        let fake = Fake::default().with(
            "/v1/games/stellarblade/mods/5/files/9/download_link.json?key=abc&expires=1",
            403,
            "forbidden",
        );

        let err = client(fake)
            .download_link(5, 9, Some(("abc", 1)))
            .await
            .unwrap_err();
        assert!(matches!(err, NexusError::LinkExpired), "got {err:?}");
    }

    #[tokio::test]
    async fn credentials_are_carried_on_the_query_string() {
        let fake = Fake::default().with(
            "/v1/games/stellarblade/mods/5/files/9/download_link.json?key=abc&expires=1700",
            200,
            r#"[{"URI":"https://cdn.example/f.zip"}]"#,
        );

        let url = client(fake)
            .download_link(5, 9, Some(("abc", 1700)))
            .await
            .unwrap();
        assert_eq!(url, "https://cdn.example/f.zip");
    }

    #[tokio::test]
    async fn graphql_surfaces_errors_rather_than_returning_empty_data() {
        let fake = Fake::default().with(
            "/v2/graphql",
            200,
            r#"{"errors":[{"message":"nope"}],"data":null}"#,
        );

        let err = client(fake)
            .graphql("query {}", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, NexusError::Decode(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_empty_key_fails_before_any_request() {
        let client = NexusClient::new(Fake::default(), "", "stellarblade");
        assert!(matches!(client.validate().await, Err(NexusError::NoApiKey)));
    }

    #[test]
    fn only_transient_failures_are_retried() {
        assert!(should_retry(&NexusError::RateLimited));
        assert!(should_retry(&NexusError::Network("reset".into())));
        assert!(should_retry(&NexusError::Http {
            status: 502,
            body: String::new()
        }));
        assert!(!should_retry(&NexusError::PremiumRequired));
        assert!(!should_retry(&NexusError::BadApiKey));
    }
}
