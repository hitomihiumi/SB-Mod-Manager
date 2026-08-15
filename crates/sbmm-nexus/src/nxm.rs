//! Parsing the `nxm://` links the "Mod Manager Download" button produces.
//!
//! A link looks like:
//!
//! ```text
//! nxm://stellarblade/mods/123/files/456?key=abc&expires=1700000000&user_id=42
//! ```
//!
//! The `key` and `expires` pair is what lets a free account ask the API for a
//! download link at all — without Premium they are the only way through, and
//! they are short-lived. Collection links use a different shape and are
//! recognised here too so the UI can route them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NxmError {
    #[error("not an nxm:// link")]
    NotNxm,
    #[error("this link is for {found}, not {expected}")]
    WrongGame { found: String, expected: String },
    #[error("malformed nxm:// link: {0}")]
    Malformed(String),
}

/// What an `nxm://` link asks the manager to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum NxmLink {
    /// A single mod file.
    File(NxmFile),
    /// A collection revision.
    Collection(NxmCollection),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NxmFile {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// Present for non-premium downloads; short-lived.
    pub key: Option<String>,
    pub expires: Option<u64>,
    pub user_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NxmCollection {
    pub domain: String,
    pub slug: String,
    pub revision: u64,
}

/// Parse a link, rejecting anything that is not for `expected_domain`.
///
/// Links for other games are a normal thing to receive — the handler is
/// registered process-wide — so this is a plain error the UI can explain,
/// not a failure.
pub fn parse(url: &str, expected_domain: &str) -> Result<NxmLink, NxmError> {
    let rest = url
        .strip_prefix("nxm://")
        .or_else(|| url.strip_prefix("NXM://"))
        .ok_or(NxmError::NotNxm)?;

    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest, ""),
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let domain = *segments
        .first()
        .ok_or_else(|| NxmError::Malformed("no game domain".into()))?;

    if !domain.eq_ignore_ascii_case(expected_domain) {
        return Err(NxmError::WrongGame {
            found: domain.to_string(),
            expected: expected_domain.to_string(),
        });
    }

    let params = parse_query(query);

    match segments.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("mods") => {
            // mods/<mod_id>/files/<file_id>
            let mod_id = number(segments.get(2), "mod id")?;
            if segments.get(3).map(|s| s.to_ascii_lowercase()).as_deref() != Some("files") {
                return Err(NxmError::Malformed(
                    "expected /files/ after the mod id".into(),
                ));
            }
            let file_id = number(segments.get(4), "file id")?;

            Ok(NxmLink::File(NxmFile {
                domain: domain.to_ascii_lowercase(),
                mod_id,
                file_id,
                key: params.get("key").cloned(),
                expires: params.get("expires").and_then(|v| v.parse().ok()),
                user_id: params.get("user_id").and_then(|v| v.parse().ok()),
            }))
        }
        Some("collections") => {
            // collections/<slug>/revisions/<n>
            let slug = segments
                .get(2)
                .ok_or_else(|| NxmError::Malformed("no collection slug".into()))?;
            let revision = match segments.get(3).map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("revisions") => number(segments.get(4), "revision")?,
                // A bare collection link means "the latest revision".
                None => 0,
                _ => {
                    return Err(NxmError::Malformed(
                        "expected /revisions/ after the slug".into(),
                    ))
                }
            };

            Ok(NxmLink::Collection(NxmCollection {
                domain: domain.to_ascii_lowercase(),
                slug: (*slug).to_string(),
                revision,
            }))
        }
        other => Err(NxmError::Malformed(format!(
            "unsupported link type: {}",
            other.unwrap_or("(none)")
        ))),
    }
}

impl NxmFile {
    /// Whether this link carries the short-lived credentials a free account
    /// needs to request a download URL.
    pub fn has_credentials(&self) -> bool {
        self.key.is_some() && self.expires.is_some()
    }
}

fn number(segment: Option<&&str>, what: &str) -> Result<u64, NxmError> {
    segment
        .ok_or_else(|| NxmError::Malformed(format!("missing {what}")))?
        .parse()
        .map_err(|_| NxmError::Malformed(format!("{what} is not a number")))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.to_ascii_lowercase(), percent_decode(v)))
        })
        .collect()
}

/// Minimal percent-decoding: keys are base64-ish and can contain `%2B` etc.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "stellarblade";

    #[test]
    fn reads_a_mod_file_link_with_credentials() {
        let link = parse(
            "nxm://stellarblade/mods/123/files/456?key=abc123&expires=1700000000&user_id=42",
            GAME,
        )
        .unwrap();

        let NxmLink::File(file) = link else {
            panic!("expected a file link");
        };
        assert_eq!(file.mod_id, 123);
        assert_eq!(file.file_id, 456);
        assert_eq!(file.key.as_deref(), Some("abc123"));
        assert_eq!(file.expires, Some(1_700_000_000));
        assert_eq!(file.user_id, Some(42));
        assert!(file.has_credentials());
    }

    #[test]
    fn a_link_without_credentials_is_still_valid() {
        let NxmLink::File(file) = parse("nxm://stellarblade/mods/1/files/2", GAME).unwrap() else {
            panic!("expected a file link");
        };
        assert!(
            !file.has_credentials(),
            "a premium account can fetch this without a key"
        );
    }

    #[test]
    fn percent_encoded_keys_survive() {
        let NxmLink::File(file) = parse(
            "nxm://stellarblade/mods/1/files/2?key=a%2Bb%2Fc%3D&expires=1",
            GAME,
        )
        .unwrap() else {
            panic!("expected a file link");
        };
        assert_eq!(file.key.as_deref(), Some("a+b/c="));
    }

    #[test]
    fn reads_a_collection_link() {
        let link = parse("nxm://stellarblade/collections/abc123/revisions/7", GAME).unwrap();
        assert_eq!(
            link,
            NxmLink::Collection(NxmCollection {
                domain: "stellarblade".into(),
                slug: "abc123".into(),
                revision: 7,
            })
        );
    }

    #[test]
    fn a_bare_collection_link_means_the_latest_revision() {
        let NxmLink::Collection(collection) =
            parse("nxm://stellarblade/collections/abc123", GAME).unwrap()
        else {
            panic!("expected a collection link");
        };
        assert_eq!(collection.revision, 0);
    }

    #[test]
    fn a_link_for_another_game_is_refused_by_name() {
        let err = parse("nxm://skyrimspecialedition/mods/1/files/2", GAME).unwrap_err();
        assert_eq!(
            err,
            NxmError::WrongGame {
                found: "skyrimspecialedition".into(),
                expected: GAME.into(),
            }
        );
    }

    #[test]
    fn the_scheme_is_matched_case_insensitively() {
        assert!(parse("NXM://StellarBlade/mods/1/files/2", GAME).is_ok());
    }

    #[test]
    fn rejects_things_that_are_not_nxm_links() {
        assert_eq!(
            parse("https://nexusmods.com/stellarblade/mods/1", GAME).unwrap_err(),
            NxmError::NotNxm
        );
    }

    #[test]
    fn rejects_malformed_links() {
        for bad in [
            "nxm://stellarblade",
            "nxm://stellarblade/mods",
            "nxm://stellarblade/mods/notanumber/files/2",
            "nxm://stellarblade/mods/1/2",
            "nxm://stellarblade/something/1",
        ] {
            assert!(
                matches!(parse(bad, GAME), Err(NxmError::Malformed(_))),
                "{bad} should be malformed, got {:?}",
                parse(bad, GAME)
            );
        }
    }
}
