//! Carrying a user's INI settings over to a new version of the file.
//!
//! When a mod is reinstalled or updated its settings file arrives with the
//! author's defaults, and the user's edits would be lost. [`merge`] takes the
//! new file as the base — its layout, comments and any keys it adds — and
//! puts back the values the user changed. Knowing the file as it was
//! originally installed (`pristine`) is what tells an edit apart from a
//! default the author has since changed: only keys whose value differs from
//! the pristine copy are carried, so a new default for an untouched key wins.
//!
//! Unreal's array syntax (`+Key=`, `-Key=`, `.Key=`, `!Key=`) and keys that
//! appear more than once in a section are left to the new file, since there
//! is no single value to carry.

use std::collections::BTreeMap;

/// `(section, key)`, both lower-cased, as INI readers compare them.
type Slot = (String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    section: String,
    /// The key and its value, when this line sets one.
    setting: Option<(String, String)>,
}

fn lines(text: &str) -> Vec<Line> {
    let mut section = String::new();
    text.lines()
        .map(|raw| {
            let trimmed = raw.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed[1..trimmed.len() - 1].trim().to_string();
                return Line {
                    section: section.clone(),
                    setting: None,
                };
            }
            let setting = (!trimmed.starts_with([';', '#']))
                .then(|| trimmed.split_once('='))
                .flatten()
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .filter(|(key, _)| !key.is_empty());
            Line {
                section: section.clone(),
                setting,
            }
        })
        .collect()
}

/// Every single-valued setting. Array operations and repeated keys are left
/// out, because they have no one value.
fn settings(text: &str) -> BTreeMap<Slot, String> {
    let mut seen: BTreeMap<Slot, usize> = BTreeMap::new();
    let mut values = BTreeMap::new();
    for line in lines(text) {
        let Some((key, value)) = line.setting else {
            continue;
        };
        if key.starts_with(['+', '-', '.', '!']) {
            continue;
        }
        let slot = (line.section.to_lowercase(), key.to_lowercase());
        *seen.entry(slot.clone()).or_default() += 1;
        values.insert(slot, value);
    }
    values.retain(|slot, _| seen.get(slot) == Some(&1));
    values
}

/// The outcome of [`merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub text: String,
    /// How many of the user's settings were carried into the new file.
    pub carried: usize,
}

/// `fresh` with the user's changes from `user` put back. `pristine` is the
/// file `user` started as; without it every value in `user` counts as the
/// user's choice.
pub fn merge(pristine: Option<&str>, user: &str, fresh: &str) -> Merged {
    let original = pristine.map(settings);
    let ours = settings(user);
    let theirs = settings(fresh);

    // What the user set themselves.
    let changed: BTreeMap<&Slot, &String> = ours
        .iter()
        .filter(|(slot, value)| match &original {
            Some(original) => original.get(*slot) != Some(*value),
            None => true,
        })
        .collect();

    // Keys the user added that the original never had are kept too; keys
    // the new version dropped are not, being obsolete. Without the original
    // there is no telling the two apart, so neither is kept.
    let mut additions: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    if let Some(original) = &original {
        for (raw, line) in user.lines().zip(lines(user)) {
            let Some((key, _)) = &line.setting else {
                continue;
            };
            let slot = (line.section.to_lowercase(), key.to_lowercase());
            if changed.contains_key(&slot)
                && !original.contains_key(&slot)
                && !theirs.contains_key(&slot)
            {
                additions
                    .entry(slot.0)
                    .or_insert_with(|| (line.section.clone(), Vec::new()))
                    .1
                    .push(raw.trim().to_string());
            }
        }
    }

    let fresh_lines = lines(fresh);
    // Where each section's last setting or comment is, so additions go
    // before the blank lines that separate it from the next.
    let mut section_end: BTreeMap<String, usize> = BTreeMap::new();
    for (index, (raw, line)) in fresh.lines().zip(&fresh_lines).enumerate() {
        if !raw.trim().is_empty() {
            section_end.insert(line.section.to_lowercase(), index);
        }
    }

    let newline = if fresh.contains("\r\n") { "\r\n" } else { "\n" };
    let mut carried = 0;
    let mut out: Vec<String> = Vec::new();
    for (index, (raw, line)) in fresh.lines().zip(&fresh_lines).enumerate() {
        let replacement = line.setting.as_ref().and_then(|(key, value)| {
            let slot = (line.section.to_lowercase(), key.to_lowercase());
            let wanted = changed.get(&slot)?;
            (theirs.contains_key(&slot) && *wanted != value).then_some(*wanted)
        });
        match replacement {
            Some(value) => {
                carried += 1;
                let eq = raw.find('=').unwrap_or(raw.len());
                let after = &raw[eq + 1..];
                let gap = &after[..after.len() - after.trim_start().len()];
                out.push(format!("{}={gap}{value}", &raw[..eq]));
            }
            None => out.push(raw.to_string()),
        }
        let section = line.section.to_lowercase();
        if section_end.get(&section) == Some(&index) {
            if let Some((_, added)) = additions.remove(&section) {
                carried += added.len();
                out.extend(added);
            }
        }
    }
    // Sections the new file does not have at all.
    for (_, (section, added)) in additions {
        if !section.is_empty() {
            out.push(format!("[{section}]"));
        }
        carried += added.len();
        out.extend(added);
    }

    let mut text = out.join(newline);
    if fresh.ends_with('\n') || out.is_empty() {
        text.push_str(newline);
    }
    Merged { text, carried }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "; Frame generation\n[General]\nEnabled = 1\nScale=1.0\n\n[Debug]\nLog=0\n";

    #[test]
    fn the_users_edits_survive_and_new_defaults_win_where_untouched() {
        let user = "; Frame generation\n[General]\nEnabled = 0\nScale=1.0\n\n[Debug]\nLog=0\n";
        // v2 changes Scale's default, adds a key and reorders a section.
        let v2 = "[Debug]\nLog=0\n\n; Frame generation\n[General]\nEnabled = 1\nScale=0.5\nSharpen=0.2\n";

        let merged = merge(Some(V1), user, v2);
        assert_eq!(
            merged.text,
            "[Debug]\nLog=0\n\n; Frame generation\n[General]\nEnabled = 0\nScale=0.5\nSharpen=0.2\n"
        );
        assert_eq!(merged.carried, 1);
    }

    #[test]
    fn without_the_original_every_value_counts_as_the_users() {
        let user = "[General]\nEnabled=0\nScale=1.0\n";
        let v2 = "[General]\nEnabled=1\nScale=0.5\n";
        assert_eq!(
            merge(None, user, v2).text,
            "[General]\nEnabled=0\nScale=1.0\n"
        );
    }

    #[test]
    fn keys_the_user_added_are_kept_and_dropped_ones_are_not() {
        let user = "[General]\nEnabled=1\nScale=1.0\nMyTweak=7\n\n[Debug]\nLog=0\n";
        let v2 = "[General]\nEnabled=1\n\n[Debug]\nLog=0\n";
        let merged = merge(Some(V1), user, v2);
        assert_eq!(
            merged.text,
            "[General]\nEnabled=1\nMyTweak=7\n\n[Debug]\nLog=0\n"
        );

        let user = "[General]\nA=1\nB=2\n[Mine]\nC=3\n";
        let merged = merge(Some(V1), user, v2);
        assert_eq!(
            merged.text,
            "[General]\nEnabled=1\nA=1\nB=2\n\n[Debug]\nLog=0\n[Mine]\nC=3\n"
        );
    }

    #[test]
    fn sections_and_keys_match_without_case_and_line_endings_are_kept() {
        let user = "[general]\nENABLED=0\n";
        let v2 = "[General]\r\nEnabled=1\r\n";
        assert_eq!(merge(None, user, v2).text, "[General]\r\nEnabled=0\r\n");
    }

    #[test]
    fn unreal_arrays_and_repeated_keys_are_left_to_the_new_file() {
        let user = "[Paths]\n+Paths=../Mine\nKey=a\nKey=b\n";
        let v2 = "[Paths]\n+Paths=../Theirs\nKey=c\nKey=d\n";
        assert_eq!(merge(None, user, v2).text, v2);
    }
}
