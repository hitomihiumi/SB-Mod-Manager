//! Reading the `collection.json` manifest out of a collection archive.
//!
//! There is no published schema for this file, so the parser is deliberately
//! forgiving: unknown fields are ignored, and an entry only has to name its
//! source, mod and file to be usable. Anything it cannot place is reported
//! rather than dropped, so a collection with one odd entry still installs.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("collection.json is not valid JSON: {0}")]
    Invalid(String),
    #[error("the collection archive has no collection.json")]
    Missing,
}

/// A collection, as far as installing it is concerned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    /// Entries that can be downloaded through the API.
    pub mods: Vec<CollectionMod>,
    /// Entries that name a file somewhere else on the internet. These cannot
    /// be fetched for the user and are listed for them to handle by hand.
    pub manual: Vec<ManualMod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionMod {
    pub name: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub version: Option<String>,
    /// Optional entries are offered as a choice rather than installed blindly.
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualMod {
    pub name: String,
    /// Where the user has to go to get it, when the manifest says.
    pub url: Option<String>,
    pub reason: String,
}

/// Parse a manifest.
pub fn parse(json: &str) -> Result<Collection, CollectionError> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| CollectionError::Invalid(e.to_string()))?;

    let info = root.get("info");
    let name = info
        .and_then(|i| i.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Collection")
        .to_string();

    let mut mods = Vec::new();
    let mut manual = Vec::new();

    let entries = root
        .get("mods")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    for entry in entries {
        let display = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed mod")
            .to_string();

        // `optional` is expressed either as a flag or through the phase/type
        // fields depending on when the collection was authored.
        let optional = entry
            .get("optional")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                entry
                    .get("type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t.eq_ignore_ascii_case("recommended"))
            });

        let source = entry.get("source");
        let source_type = source
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let mod_id = source.and_then(|s| number(s, "modId"));
        let file_id = source.and_then(|s| number(s, "fileId"));

        match (source_type, mod_id, file_id) {
            // The only source the API can fetch on the user's behalf.
            ("nexus", Some(mod_id), Some(file_id)) => mods.push(CollectionMod {
                name: display,
                mod_id,
                file_id,
                version: entry
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                optional,
            }),
            ("nexus", _, _) => manual.push(ManualMod {
                name: display,
                url: None,
                reason: "the collection does not say which file to download".into(),
            }),
            (other, _, _) => manual.push(ManualMod {
                name: display,
                url: source
                    .and_then(|s| s.get("url"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                reason: if other.is_empty() {
                    "no download source given".into()
                } else {
                    format!("hosted outside Nexus ({other})")
                },
            }),
        }
    }

    Ok(Collection {
        name,
        version: info
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        author: info
            .and_then(|i| i.get("author"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        mods,
        manual,
    })
}

// -- the v2 GraphQL route ---------------------------------------------------

/// Ask for one revision of a collection.
///
/// Two things are wanted and either is enough: a link to the revision archive,
/// whose `collection.json` is the authoritative manifest, and the mod list
/// inline, which saves a download when it is present.
pub const REVISION_QUERY: &str = r#"
query CollectionRevision($slug: String!, $revision: Int!, $domain: String!) {
  collectionRevision(
    slug: $slug
    revisionNumber: $revision
    domainName: $domain
    viewAdultContent: true
  ) {
    revisionNumber
    downloadLink
    collection { name slug }
    modFiles {
      optional
      file { modId fileId name version }
    }
  }
}
"#;

/// What a revision lookup produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revision {
    pub name: String,
    pub revision: u64,
    /// The collection archive, when the response offered one.
    pub download_url: Option<String>,
    pub mods: Vec<CollectionMod>,
}

/// Read a `collectionRevision` response.
///
/// The v2 schema is not published and has changed shape before, so nothing
/// here depends on the exact nesting: the response is walked for objects that
/// carry both a mod id and a file id. A field being renamed one level up then
/// costs nothing, where a path-based reader would return an empty collection
/// and look like the collection was empty.
pub fn parse_revision(data: &serde_json::Value) -> Revision {
    let root = data.get("collectionRevision").unwrap_or(data);

    let mut mods = Vec::new();
    harvest(root, false, &mut mods);
    // The same file can appear under more than one key in a response that
    // nests it; the first mention wins.
    mods.dedup_by(|a, b| a.mod_id == b.mod_id && a.file_id == b.file_id);

    Revision {
        name: root
            .get("collection")
            .and_then(|c| c.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Collection")
            .to_string(),
        revision: root
            .get("revisionNumber")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        download_url: root
            .get("downloadLink")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        mods,
    }
}

/// Walk the response for anything that names a downloadable file.
///
/// `optional` is carried down because it sits on the wrapper around the file
/// rather than on the file itself.
fn harvest(value: &serde_json::Value, optional: bool, out: &mut Vec<CollectionMod>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                harvest(item, optional, out);
            }
        }
        serde_json::Value::Object(fields) => {
            let optional = fields
                .get("optional")
                .and_then(|v| v.as_bool())
                .unwrap_or(optional);

            if let (Some(mod_id), Some(file_id)) = (number(value, "modId"), number(value, "fileId"))
            {
                out.push(CollectionMod {
                    name: fields
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed mod")
                        .to_string(),
                    mod_id,
                    file_id,
                    version: fields
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    optional,
                });
                return;
            }

            for child in fields.values() {
                harvest(child, optional, out);
            }
        }
        _ => {}
    }
}

/// Ids appear as numbers in some manifests and strings in others.
fn number(value: &serde_json::Value, key: &str) -> Option<u64> {
    let field = value.get(key)?;
    field
        .as_u64()
        .or_else(|| field.as_str().and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "info": { "name": "Essential Stellar Blade", "version": "1.2.0", "author": "someone" },
      "mods": [
        {
          "name": "Aurora Suit",
          "version": "1.0",
          "optional": false,
          "source": { "type": "nexus", "modId": 123, "fileId": 456 }
        },
        {
          "name": "Optional Extras",
          "optional": true,
          "source": { "type": "nexus", "modId": "789", "fileId": "1011" }
        },
        {
          "name": "Something Elsewhere",
          "source": { "type": "browse", "url": "https://example.com/mod.zip" }
        }
      ]
    }"#;

    #[test]
    fn reads_the_collection_details() {
        let collection = parse(MANIFEST).unwrap();
        assert_eq!(collection.name, "Essential Stellar Blade");
        assert_eq!(collection.version.as_deref(), Some("1.2.0"));
        assert_eq!(collection.author.as_deref(), Some("someone"));
    }

    #[test]
    fn nexus_entries_become_installable_mods() {
        let collection = parse(MANIFEST).unwrap();
        assert_eq!(collection.mods.len(), 2);
        assert_eq!(collection.mods[0].mod_id, 123);
        assert_eq!(collection.mods[0].file_id, 456);
        assert!(!collection.mods[0].optional);
    }

    #[test]
    fn ids_written_as_strings_are_accepted() {
        let collection = parse(MANIFEST).unwrap();
        assert_eq!(collection.mods[1].mod_id, 789);
        assert_eq!(collection.mods[1].file_id, 1011);
        assert!(collection.mods[1].optional);
    }

    #[test]
    fn entries_hosted_elsewhere_are_listed_for_the_user() {
        let collection = parse(MANIFEST).unwrap();
        assert_eq!(collection.manual.len(), 1);
        assert_eq!(collection.manual[0].name, "Something Elsewhere");
        assert_eq!(
            collection.manual[0].url.as_deref(),
            Some("https://example.com/mod.zip")
        );
        assert!(collection.manual[0].reason.contains("browse"));
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        let json = r#"{
          "info": { "name": "X", "somethingNew": 1 },
          "somethingElse": true,
          "mods": [
            { "name": "A", "futureField": [], "source": { "type": "nexus", "modId": 1, "fileId": 2 } }
          ]
        }"#;
        let collection = parse(json).unwrap();
        assert_eq!(collection.mods.len(), 1);
    }

    #[test]
    fn a_nexus_entry_missing_its_file_id_is_not_silently_dropped() {
        let json = r#"{"info":{"name":"X"},"mods":[
          {"name":"Half specified","source":{"type":"nexus","modId":1}}
        ]}"#;
        let collection = parse(json).unwrap();
        assert!(collection.mods.is_empty());
        assert_eq!(collection.manual.len(), 1, "it must still be reported");
    }

    #[test]
    fn a_recommended_entry_counts_as_optional() {
        let json = r#"{"info":{"name":"X"},"mods":[
          {"name":"Maybe","type":"recommended","source":{"type":"nexus","modId":1,"fileId":2}}
        ]}"#;
        assert!(parse(json).unwrap().mods[0].optional);
    }

    #[test]
    fn a_manifest_with_no_mods_is_still_a_collection() {
        let collection = parse(r#"{"info":{"name":"Empty"}}"#).unwrap();
        assert_eq!(collection.name, "Empty");
        assert!(collection.mods.is_empty());
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(matches!(
            parse("not json"),
            Err(CollectionError::Invalid(_))
        ));
    }

    /// The response shape as documented; the happy path.
    #[test]
    fn a_revision_response_yields_its_mods_and_archive_link() {
        let data = serde_json::json!({
            "collectionRevision": {
                "revisionNumber": 7,
                "downloadLink": "https://example.invalid/revision.zip",
                "collection": { "name": "Essential Stellar Blade", "slug": "abc123" },
                "modFiles": [
                    { "optional": false,
                      "file": { "modId": 123, "fileId": 456, "name": "Aurora Suit", "version": "1.0" } },
                    { "optional": true,
                      "file": { "modId": 789, "fileId": 1011, "name": "Optional Extras" } }
                ]
            }
        });

        let revision = parse_revision(&data);
        assert_eq!(revision.name, "Essential Stellar Blade");
        assert_eq!(revision.revision, 7);
        assert_eq!(
            revision.download_url.as_deref(),
            Some("https://example.invalid/revision.zip")
        );
        assert_eq!(revision.mods.len(), 2);
        assert_eq!(revision.mods[0].mod_id, 123);
        assert_eq!(revision.mods[0].file_id, 456);
        assert!(!revision.mods[0].optional);
        assert!(
            revision.mods[1].optional,
            "the flag sits on the wrapper, not the file"
        );
    }

    /// The point of walking the tree: a schema change one level up must not
    /// turn a full collection into an empty one.
    #[test]
    fn mods_are_still_found_when_the_response_is_nested_differently() {
        let data = serde_json::json!({
            "collectionRevision": {
                "collection": { "name": "Renamed Shape" },
                "someNewWrapper": {
                    "edges": [
                        { "node": { "optional": true,
                                    "modFile": { "modId": "42", "fileId": "99", "name": "Thing" } } }
                    ]
                }
            }
        });

        let revision = parse_revision(&data);
        assert_eq!(revision.mods.len(), 1);
        assert_eq!(revision.mods[0].mod_id, 42, "ids may arrive as strings");
        assert_eq!(revision.mods[0].file_id, 99);
        assert!(revision.mods[0].optional);
    }

    #[test]
    fn a_response_with_nothing_usable_is_empty_rather_than_an_error() {
        let revision = parse_revision(&serde_json::json!({ "collectionRevision": null }));
        assert!(revision.mods.is_empty());
        assert_eq!(revision.download_url, None);
        assert_eq!(revision.name, "Collection");
    }
}
