//! What Nexus says about a mod beyond its files: who made it, what it is,
//! its picture and tags, and the mods it needs.
//!
//! Read through the v2 GraphQL API, which has requirements the v1 REST API
//! lacks and answers without an API key.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{NexusClient, NexusError, Transport};

pub(crate) const QUERY: &str = "query($modId: ID!, $gameId: ID!) {
  mod(modId: $modId, gameId: $gameId) {
    modId name author version summary endorsements downloads
    pictureUrl updatedAt adultContent category
    tags { name }
    modRequirements {
      nexusRequirements { nodes { modId modName url notes externalRequirement } }
    }
  }
}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModDetails {
    pub mod_id: u64,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub summary: Option<String>,
    pub endorsements: u64,
    pub downloads: u64,
    pub picture_url: Option<String>,
    /// ISO 8601.
    pub updated_at: Option<String>,
    pub adult: bool,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub requirements: Vec<Requirement>,
}

/// A mod or tool this mod needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// The Nexus mod id, or `None` for something hosted elsewhere.
    pub mod_id: Option<u64>,
    pub name: String,
    pub url: String,
    pub notes: Option<String>,
}

/// The mod out of a GraphQL `data` object.
pub fn parse(data: &Value) -> Option<ModDetails> {
    let m = data.get("mod")?;
    let text = |key: &str| {
        m.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let number = |key: &str| m.get(key).and_then(Value::as_u64).unwrap_or(0);
    let requirements = m
        .pointer("/modRequirements/nexusRequirements/nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| {
                    let id = node.get("modId").and_then(|v| match v {
                        Value::String(s) => s.parse::<u64>().ok(),
                        other => other.as_u64(),
                    });
                    let external = node
                        .get("externalRequirement")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some(Requirement {
                        mod_id: id.filter(|id| *id != 0 && !external),
                        name: node.get("modName")?.as_str()?.trim().to_string(),
                        url: node
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        notes: node
                            .get("notes")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ModDetails {
        mod_id: m
            .get("modId")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))?,
        name: text("name")?,
        author: text("author"),
        version: text("version"),
        summary: text("summary"),
        endorsements: number("endorsements"),
        downloads: number("downloads"),
        picture_url: text("pictureUrl"),
        updated_at: text("updatedAt"),
        adult: m
            .get("adultContent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        category: text("category"),
        tags: m
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| t.get("name")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        requirements,
    })
}

impl<T: Transport> NexusClient<T> {
    /// A mod's page details, or `None` if Nexus has no such mod.
    pub async fn mod_details(
        &self,
        game_id: u64,
        mod_id: u64,
    ) -> Result<Option<ModDetails>, NexusError> {
        let data = self
            .graphql(
                QUERY,
                serde_json::json!({ "modId": mod_id.to_string(), "gameId": game_id.to_string() }),
            )
            .await?;
        Ok(parse(&data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real answer for a Stellar Blade mod.
    #[test]
    fn a_real_answer_is_read() {
        let data: Value = serde_json::from_str(
            r#"{"mod":{"modId":1496,"name":"CNS - Custom Nanosuit System","author":"Someone",
            "version":"2.1","summary":"Suits.","endorsements":812,"downloads":40211,
            "pictureUrl":"https://staticdelivery.nexusmods.com/mods/7804/images/1496/a.jpg",
            "updatedAt":"2026-07-11T05:48:12Z","adultContent":false,"category":"Outfits",
            "tags":[{"name":"Replacer"},{"name":"Quality of Life"}],
            "modRequirements":{"nexusRequirements":{"nodes":[
              {"modId":"0","modName":"UE4SS For Stellar Blade","url":"https://github.com/Chrisr0/RE-UE4SS/releases","notes":"","externalRequirement":true},
              {"modId":"5","modName":"Fast-Intro-No-Splash","url":"https://www.nexusmods.com/stellarblade/mods/5","notes":"for the menu","externalRequirement":false}
            ]}}}}"#,
        )
        .unwrap();
        let details = parse(&data).unwrap();
        assert_eq!(details.mod_id, 1496);
        assert_eq!(details.tags, ["Replacer", "Quality of Life"]);
        assert_eq!(details.requirements.len(), 2);
        assert_eq!(details.requirements[0].mod_id, None, "hosted elsewhere");
        assert_eq!(details.requirements[0].notes, None);
        assert_eq!(details.requirements[1].mod_id, Some(5));
        assert_eq!(
            details.requirements[1].notes.as_deref(),
            Some("for the menu")
        );
    }

    #[test]
    fn a_missing_mod_reads_as_none() {
        assert_eq!(parse(&serde_json::json!({ "mod": null })), None);
    }
}
