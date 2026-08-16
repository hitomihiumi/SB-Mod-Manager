//! Where newer upscaler DLLs come from.
//!
//! Both vendors publish the redistributable DLLs themselves, on GitHub, tagged
//! by version — NVIDIA in `NVIDIA/DLSS` and AMD in the FidelityFX SDK. Those
//! are used directly rather than any of the mirror sites that repackage them,
//! because a tag is a stable, checkable name for a specific build and the file
//! arrives exactly as the vendor signed it.
//!
//! The important subtlety is the *line*. A game built against FSR 3.1 loads
//! `amd_fidelityfx_dx12.dll`; FidelityFX SDK 2.0 renamed that file and changed
//! the ABI, so its releases are newer but useless here — dropping one in gets
//! a DLL the game will not load. So each source declares which tags belong to
//! the line it serves, and anything outside it is ignored rather than offered.

use crate::{Component, Family, Version};

/// A place to get one family's DLLs from.
#[derive(Debug, Clone)]
pub struct Source {
    pub family: Family,
    /// `owner/repo` on GitHub, used to list tags and to build download URLs.
    pub repo: &'static str,
    /// Where in the tree the DLLs sit, at any tag of this line.
    pub directory: &'static str,
    /// Highest tag this line may use, exclusive. `None` means no ceiling.
    ///
    /// This is the FSR problem above: SDK 2.0 and later are a different line
    /// with different file names, so a game on 3.1 is capped below them.
    pub line_below: Option<Version>,
    /// What the tag numbers mean to a user, when they differ from the tag.
    pub version_note: Option<&'static str>,
}

impl Source {
    /// The URL for one DLL at one tag.
    pub fn download_url(&self, tag: &str, component: Component) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            self.repo,
            tag,
            self.directory,
            component.file_name(),
        )
    }

    /// Where to ask GitHub for the tag list.
    pub fn tags_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/tags?per_page=100",
            self.repo
        )
    }

    /// Whether a tag belongs to the line this source serves.
    pub fn accepts(&self, version: Version) -> bool {
        match self.line_below {
            Some(ceiling) => version < ceiling,
            None => true,
        }
    }

    /// The newest usable release among `tags`.
    ///
    /// Tags that do not parse as versions, and tags from another line, are
    /// dropped rather than sorted alphabetically — `v3.7.10` must not beat
    /// `v310.7.0`.
    pub fn newest<'a>(&self, tags: impl IntoIterator<Item = &'a str>) -> Option<Release> {
        tags.into_iter()
            .filter_map(|tag| {
                let version = Version::parse(tag)?;
                self.accepts(version).then(|| Release {
                    tag: tag.to_string(),
                    version,
                })
            })
            .max_by_key(|release| release.version)
    }
}

/// One published version of a family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The git tag, which is what the download URL is built from.
    pub tag: String,
    pub version: Version,
}

/// FidelityFX SDK 2.0 is where the FSR 3.1 line ends: the DLLs are renamed and
/// the ABI is not compatible with a game built against 3.1.
const FSR_LINE_CEILING: Version = Version(2, 0, 0, 0);

/// Every source the manager knows about.
pub fn catalog() -> Vec<Source> {
    vec![
        Source {
            family: Family::Dlss,
            repo: "NVIDIA/DLSS",
            directory: "lib/Windows_x86_64/rel",
            line_below: None,
            version_note: None,
        },
        Source {
            family: Family::Fsr,
            repo: "GPUOpen-LibrariesAndSDKs/FidelityFX-SDK",
            directory: "PrebuiltSignedDLL",
            line_below: Some(FSR_LINE_CEILING),
            version_note: Some("SDK 1.1.x ships FSR 3.1.x, which is the line this game uses"),
        },
    ]
}

/// The source for a family, if there is one.
pub fn source_for(family: Family) -> Option<Source> {
    catalog().into_iter().find(|s| s.family == family)
}

/// Pull tag names out of the GitHub tags response.
///
/// Deliberately forgiving: GitHub adds fields over time, and a tag list that
/// gained a key should not stop the check from working.
pub fn parse_tags(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| Some(entry.get("name")?.as_str()?.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dlss() -> Source {
        source_for(Family::Dlss).unwrap()
    }

    fn fsr() -> Source {
        source_for(Family::Fsr).unwrap()
    }

    #[test]
    fn the_newest_dlss_tag_wins_on_number_not_on_text() {
        // These are the real tags in NVIDIA/DLSS, in the order they appear.
        let tags = [
            "v3.1.30", "v3.5.0", "v3.5.10", "v3.7.0", "v3.7.10", "v3.7.20", "v310.1.0", "v310.5.3",
            "v310.7.0",
        ];
        let newest = dlss().newest(tags).unwrap();
        assert_eq!(newest.tag, "v310.7.0");
        assert_eq!(newest.version, Version(310, 7, 0, 0));
    }

    /// The whole reason the ceiling exists: SDK 2.x is newer and unusable.
    #[test]
    fn fsr_stays_on_the_3_1_line_and_ignores_the_newer_sdk() {
        let tags = [
            "fsr3-v3.0.4",
            "v1.0.0",
            "v1.1.0",
            "v1.1.3",
            "v1.1.4",
            "v2.0.0",
            "v2.3.0",
        ];
        let newest = fsr().newest(tags).unwrap();
        assert_eq!(
            newest.tag, "v1.1.4",
            "SDK 2.x renamed the DLLs, so it is not an upgrade for a 3.1 game"
        );
    }

    #[test]
    fn tags_that_are_not_versions_are_dropped() {
        let newest = dlss().newest(["latest", "main", "release-candidate", "v310.2.1"]);
        assert_eq!(newest.unwrap().tag, "v310.2.1");
        assert_eq!(dlss().newest(["nightly", "wip"]), None);
    }

    #[test]
    fn download_urls_point_at_the_vendors_own_files() {
        assert_eq!(
            dlss().download_url("v310.7.0", Component::DlssSuperResolution),
            "https://raw.githubusercontent.com/NVIDIA/DLSS/v310.7.0/lib/Windows_x86_64/rel/nvngx_dlss.dll"
        );
        assert_eq!(
            fsr().download_url("v1.1.4", Component::FsrDx12),
            "https://raw.githubusercontent.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/v1.1.4/PrebuiltSignedDLL/amd_fidelityfx_dx12.dll"
        );
    }

    #[test]
    fn tag_names_are_read_out_of_the_github_response() {
        let body = r#"[
            {"name": "v310.7.0", "zipball_url": "…", "commit": {"sha": "abc"}},
            {"name": "v310.6.0", "commit": {"sha": "def"}}
        ]"#;
        assert_eq!(parse_tags(body), vec!["v310.7.0", "v310.6.0"]);
    }

    #[test]
    fn a_broken_tag_response_yields_nothing_rather_than_failing() {
        assert!(parse_tags("").is_empty());
        assert!(parse_tags(r#"{"message": "rate limit exceeded"}"#).is_empty());
        assert!(parse_tags("[{\"no_name\": 1}]").is_empty());
    }
}
