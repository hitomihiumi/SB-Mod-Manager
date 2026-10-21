//! Knowing more about a mod: what it changes, where it must sit in the load
//! order, and what its Nexus page says — including the mods it needs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sbmm_core::rules::{self, Rule};
use sbmm_nexus::ModDetails;
use sbmm_store::ModRecord;

use crate::dto::{AssetArea, ModClash, ModContents, NexusView, OrderRuleView, RequirementView};
use crate::{App, AppError, Result};

/// Areas listed before the rest are folded into "elsewhere".
const MAX_AREAS: usize = 12;
const EXAMPLES_PER_AREA: usize = 4;
/// Nexus answers older than this are asked again when the panel opens.
pub const NEXUS_DETAILS_TTL_HOURS: i64 = 24;

/// Where an asset sits, two folders deep under `Content`, so a mod's
/// changes read as "Characters/Eve" rather than a thousand paths.
fn area_of(asset: &str) -> String {
    let path = asset.trim_start_matches('/');
    let inside = path
        .split_once("Content/")
        .map(|(_, rest)| rest)
        .unwrap_or(path);
    let folders: Vec<&str> = inside.split('/').collect();
    let depth = folders.len().saturating_sub(1).min(2);
    if depth == 0 {
        "(top level)".to_string()
    } else {
        folders[..depth].join("/")
    }
}

/// An asset without its extension: `.uasset`, `.uexp` and `.ubulk` of one
/// asset count once.
fn asset_key(asset: &str) -> &str {
    match asset.rfind('.') {
        Some(dot) if !asset[dot..].contains('/') => &asset[..dot],
        _ => asset,
    }
}

impl App {
    // -- what a mod changes -------------------------------------------------

    pub fn mod_contents(&self, mod_id: i64) -> Result<ModContents> {
        let records = self.store.list_mods()?;
        let record = records
            .iter()
            .find(|r| r.id == mod_id)
            .ok_or(AppError::Store(sbmm_store::StoreError::UnknownMod(mod_id)))?;

        let targets = self.targets_of(record)?;
        let assets = self.store.mod_assets(mod_id)?;
        let mut keys: BTreeMap<&str, bool> = BTreeMap::new();
        for (asset, named) in &assets {
            keys.insert(asset_key(asset), *named);
        }

        let mut areas: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, named) in &keys {
            let area = if *named {
                area_of(key)
            } else {
                "(unnamed chunks)".to_string()
            };
            areas.entry(area).or_default().push(key.to_string());
        }
        let mut areas: Vec<AssetArea> = areas
            .into_iter()
            .map(|(area, list)| AssetArea {
                count: list.len(),
                examples: list
                    .iter()
                    .take(EXAMPLES_PER_AREA)
                    .map(|a| a.rsplit('/').next().unwrap_or(a).to_string())
                    .collect(),
                area,
            })
            .collect();
        areas.sort_by(|a, b| b.count.cmp(&a.count).then(a.area.cmp(&b.area)));
        if areas.len() > MAX_AREAS {
            let rest: usize = areas[MAX_AREAS..].iter().map(|a| a.count).sum();
            areas.truncate(MAX_AREAS);
            areas.push(AssetArea {
                area: "(elsewhere)".into(),
                count: rest,
                examples: Vec::new(),
            });
        }

        // Who else changes the same things, and who wins. Paks clash by
        // asset; loose files by path.
        let mut shared: BTreeMap<i64, usize> = BTreeMap::new();
        for contested in self.store.contested_assets()? {
            if contested.mod_ids.contains(&mod_id) {
                for other in contested.mod_ids.iter().filter(|id| **id != mod_id) {
                    *shared.entry(*other).or_default() += 1;
                }
            }
        }
        let mine: BTreeSet<&PathBuf> = targets.iter().collect();
        for other in records.iter().filter(|r| r.id != mod_id) {
            let overlap = self
                .targets_of(other)?
                .iter()
                .filter(|t| mine.contains(t))
                .count();
            if overlap > 0 {
                *shared.entry(other.id).or_default() += overlap;
            }
        }
        let mut clashes: Vec<ModClash> = shared
            .into_iter()
            .filter_map(|(other, count)| {
                let theirs = records.iter().find(|r| r.id == other)?;
                Some(ModClash {
                    mod_id: other,
                    name: theirs.name.clone(),
                    shared: count,
                    enabled: theirs.enabled,
                    // Later in the load order wins.
                    wins: record.priority > theirs.priority,
                })
            })
            .collect();
        clashes.sort_by_key(|c| std::cmp::Reverse(c.shared));

        Ok(ModContents {
            files: targets
                .iter()
                .take(500)
                .map(|t| t.to_string_lossy().replace('\\', "/"))
                .collect(),
            file_count: targets.len(),
            indexed: record.assets_indexed_at.is_some(),
            asset_count: keys.len(),
            areas,
            clashes,
        })
    }

    /// Every path a mod puts in the game, whatever the deployment mode.
    fn targets_of(&self, record: &ModRecord) -> Result<Vec<PathBuf>> {
        let components = self.store.components_for(record.id)?;
        Ok(self
            .plan_for(record, &components)
            .files
            .into_iter()
            .map(|f| f.target)
            .collect())
    }

    // -- load order rules ---------------------------------------------------

    fn rules(&self) -> Result<Vec<(i64, Rule)>> {
        Ok(self
            .store
            .order_rules()?
            .into_iter()
            .map(|(id, before, after)| (id, Rule { before, after }))
            .collect())
    }

    fn current_order(&self) -> Result<Vec<i64>> {
        Ok(self.store.list_mods()?.iter().map(|m| m.id).collect())
    }

    pub fn order_rules(&self) -> Result<Vec<OrderRuleView>> {
        let mods = self.store.list_mods()?;
        let name = |id: i64| {
            mods.iter()
                .find(|m| m.id == id)
                .map(|m| m.name.clone())
                .unwrap_or_default()
        };
        let order: Vec<i64> = mods.iter().map(|m| m.id).collect();
        let rules = self.rules()?;
        let broken = rules::broken(&order, &rules.iter().map(|(_, r)| *r).collect::<Vec<_>>());
        Ok(rules
            .into_iter()
            .map(|(id, rule)| OrderRuleView {
                id,
                before_id: rule.before,
                before: name(rule.before),
                after_id: rule.after,
                after: name(rule.after),
                broken: broken.contains(&rule),
            })
            .collect())
    }

    /// `before` must load before `after`, so `after` wins where they clash.
    pub fn add_order_rule(&mut self, before: i64, after: i64) -> Result<()> {
        let order = self.current_order()?;
        if !order.contains(&before) || !order.contains(&after) {
            return Err(AppError::Invalid("both mods must be installed".into()));
        }
        let rules: Vec<Rule> = self.rules()?.into_iter().map(|(_, r)| r).collect();
        if rules::creates_cycle(&rules, Rule { before, after }) {
            return Err(AppError::Invalid(
                "that contradicts the rules already set — one of them has to go first".into(),
            ));
        }
        self.store.add_order_rule(before, after)?;
        Ok(())
    }

    pub fn delete_order_rule(&mut self, id: i64) -> Result<()> {
        Ok(self.store.delete_order_rule(id)?)
    }

    /// Reorder the list to obey every rule, moving as little as possible.
    /// Returns how many mods changed place.
    pub fn sort_by_rules(&mut self) -> Result<usize> {
        let order = self.current_order()?;
        let rules: Vec<Rule> = self.rules()?.into_iter().map(|(_, r)| r).collect();
        let sorted = rules::sort(&order, &rules)
            .ok_or_else(|| AppError::Invalid("the rules contradict each other".into()))?;
        let moved = order.iter().zip(&sorted).filter(|(a, b)| a != b).count();
        if moved > 0 {
            self.store.set_order(&sorted)?;
        }
        Ok(moved)
    }

    /// For the mod list: what each mod's broken rules ask of it.
    pub(crate) fn misordered(&self, records: &[ModRecord]) -> Result<BTreeMap<i64, Vec<String>>> {
        let order: Vec<i64> = records.iter().map(|m| m.id).collect();
        let rules: Vec<Rule> = self.rules()?.into_iter().map(|(_, r)| r).collect();
        let name = |id: i64| {
            records
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.name.as_str())
                .unwrap_or("")
                .to_string()
        };
        let mut out: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for rule in rules::broken(&order, &rules) {
            out.entry(rule.after)
                .or_default()
                .push(format!("should load after {}", name(rule.before)));
            out.entry(rule.before)
                .or_default()
                .push(format!("should load before {}", name(rule.after)));
        }
        Ok(out)
    }

    // -- Nexus --------------------------------------------------------------

    fn cached_details(&self) -> Result<BTreeMap<i64, (ModDetails, String)>> {
        Ok(self
            .store
            .nexus_details()?
            .into_iter()
            .filter_map(|(id, (json, at))| Some((id, (serde_json::from_str(&json).ok()?, at))))
            .collect())
    }

    pub fn record_nexus_details(&mut self, details: &ModDetails) -> Result<()> {
        self.store
            .put_nexus_details(details.mod_id as i64, &serde_json::to_string(details)?)?;
        Ok(())
    }

    /// What Nexus said about a mod, with each requirement checked against
    /// what is installed. `None` for mods not from Nexus.
    pub fn nexus_view(&self, mod_id: i64) -> Result<Option<NexusView>> {
        let records = self.store.list_mods()?;
        let Some(nexus_id) = records
            .iter()
            .find(|r| r.id == mod_id)
            .and_then(|r| r.nexus_mod_id)
        else {
            return Ok(None);
        };
        let cached = self.cached_details()?.remove(&nexus_id);
        let ue4ss_loads = self.ue4ss_presence(&records)?.loads();
        Ok(Some(match cached {
            None => NexusView {
                nexus_mod_id: nexus_id,
                details: None,
                fetched_at: None,
                requirements: Vec::new(),
            },
            Some((details, fetched_at)) => NexusView {
                nexus_mod_id: nexus_id,
                requirements: details
                    .requirements
                    .iter()
                    .map(|req| RequirementView {
                        state: requirement_state(req, &records, ue4ss_loads).to_string(),
                        installed_as: req.mod_id.and_then(|id| {
                            records
                                .iter()
                                .find(|r| r.nexus_mod_id == Some(id as i64))
                                .map(|r| r.id)
                        }),
                        requirement: req.clone(),
                    })
                    .collect(),
                details: Some(details),
                fetched_at: Some(fetched_at),
            },
        }))
    }

    /// For the mod list: Nexus requirements of enabled mods that are not
    /// installed and enabled. Things hosted elsewhere are only checked when
    /// they are UE4SS, which the list already reports.
    pub(crate) fn missing_nexus_requirements(
        &self,
        records: &[ModRecord],
    ) -> Result<BTreeMap<i64, Vec<String>>> {
        let cached = self.cached_details()?;
        let mut out: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for record in records.iter().filter(|r| r.enabled) {
            let Some((details, _)) = record.nexus_mod_id.and_then(|id| cached.get(&id)) else {
                continue;
            };
            for req in &details.requirements {
                if req.mod_id.is_some() && requirement_state(req, records, true) != "enabled" {
                    out.entry(record.id).or_default().push(req.name.clone());
                }
            }
        }
        Ok(out)
    }

    /// Mods from Nexus whose details were never fetched, or not lately.
    pub fn nexus_details_due(&self, mod_id: i64) -> Result<bool> {
        let Some(view) = self.nexus_view(mod_id)? else {
            return Ok(false);
        };
        Ok(match view.fetched_at {
            None => true,
            Some(at) => self.store.older_than_hours(&at, NEXUS_DETAILS_TTL_HOURS)?,
        })
    }

    /// Where the mod page's picture goes in the mod's folder, so it shows
    /// with the archive's own pictures. `None` when the URL has no picture
    /// extension.
    pub fn nexus_picture_path(&self, mod_id: i64, url: &str) -> Result<Option<PathBuf>> {
        let record = self.record(mod_id)?;
        let ext = Path::new(url.split(['?', '#']).next().unwrap_or(url))
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .filter(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"));
        Ok(ext.map(|ext| {
            self.staging_path(&record.staging_folder)
                .join(format!("_nexus.{ext}"))
        }))
    }

    /// After the page picture is downloaded: it becomes the mod's picture if
    /// the mod had none.
    pub fn adopt_nexus_picture(&mut self, mod_id: i64, path: &Path) -> Result<()> {
        let record = self.record(mod_id)?;
        if record.image_path.is_none() {
            if let Some(name) = path.file_name() {
                self.store
                    .set_image(mod_id, Some(&name.to_string_lossy()))?;
            }
        }
        Ok(())
    }
}

/// `enabled`, `disabled`, `missing`, or `external` for something hosted
/// elsewhere that cannot be checked. UE4SS, hosted on GitHub, is checked.
fn requirement_state(
    req: &sbmm_nexus::Requirement,
    records: &[ModRecord],
    ue4ss_loads: bool,
) -> &'static str {
    match req.mod_id {
        Some(id) => match records.iter().find(|r| r.nexus_mod_id == Some(id as i64)) {
            Some(r) if r.enabled => "enabled",
            Some(_) => "disabled",
            None => "missing",
        },
        None if req.name.to_lowercase().contains("ue4ss") => {
            if ue4ss_loads {
                "enabled"
            } else {
                "missing"
            }
        }
        None => "external",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_group_two_folders_under_content() {
        assert_eq!(
            area_of("SB/Content/Characters/Eve/Body.uasset"),
            "Characters/Eve"
        );
        assert_eq!(area_of("/SB/Content/Movies/Intro.bk2"), "Movies");
        assert_eq!(area_of("SB/Content/Thing.uasset"), "(top level)");
        assert_eq!(asset_key("SB/Content/A/B.uexp"), "SB/Content/A/B");
        assert_eq!(asset_key("chunk-0001"), "chunk-0001");
    }
}
