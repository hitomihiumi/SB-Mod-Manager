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

// -- editing one setting -------------------------------------------------
//
// What a settings tweak needs: read, set and remove single keys, and add or
// remove whole lines for Unreal's array entries, leaving every other line —
// comments, order, line endings — as it was.

/// A line that sets something, in file order: `(section, key, value)`.
/// Keys keep Unreal's array prefixes (`+Paths`).
pub fn entries(text: &str) -> Vec<(String, String, String)> {
    lines(text)
        .into_iter()
        .filter_map(|line| line.setting.map(|(k, v)| (line.section, k, v)))
        .collect()
}

/// Whether a key is one of Unreal's array operations rather than a value.
pub fn is_array_op(key: &str) -> bool {
    key.starts_with(['+', '-', '.', '!'])
}

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn join(raw: Vec<String>, newline: &str, trailing: bool) -> String {
    let mut text = raw.join(newline);
    if trailing && !text.is_empty() {
        text.push_str(newline);
    }
    text
}

/// The value of the first `key` in `section`, ignoring case.
pub fn get(text: &str, section: &str, key: &str) -> Option<String> {
    lines(text).into_iter().find_map(|line| {
        let (k, v) = line.setting?;
        (line.section.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key)).then_some(v)
    })
}

/// Index of the line after the last non-blank line of `section`, or `None`
/// when the section is not there.
fn section_end(raw: &[String], parsed: &[Line], section: &str) -> Option<usize> {
    let mut end = None;
    for (index, (text, line)) in raw.iter().zip(parsed).enumerate() {
        if line.section.eq_ignore_ascii_case(section) && !text.trim().is_empty() {
            end = Some(index + 1);
        }
    }
    end
}

fn append_to_section(raw: &mut Vec<String>, parsed: &[Line], section: &str, line: String) {
    match section_end(raw, parsed, section) {
        Some(at) => raw.insert(at, line),
        None => {
            if raw.last().is_some_and(|l| !l.trim().is_empty()) {
                raw.push(String::new());
            }
            if !section.is_empty() {
                raw.push(format!("[{section}]"));
            }
            raw.push(line);
        }
    }
}

/// `text` with `key` in `section` set to `value`: the first such line is
/// changed in place, or the key is added at the end of the section (and the
/// section at the end of the file).
pub fn set(text: &str, section: &str, key: &str, value: &str) -> String {
    let newline = newline_of(text);
    let parsed = lines(text);
    let mut raw: Vec<String> = text.lines().map(str::to_string).collect();
    let existing = parsed.iter().position(|line| {
        line.section.eq_ignore_ascii_case(section)
            && line
                .setting
                .as_ref()
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(key))
    });
    match existing {
        Some(index) => {
            let line = &raw[index];
            let eq = line.find('=').unwrap_or(line.len());
            raw[index] = format!("{}={value}", &line[..eq]);
        }
        None => append_to_section(&mut raw, &parsed, section, format!("{key}={value}")),
    }
    join(raw, newline, true)
}

/// `text` without the first `key` in `section`.
pub fn remove(text: &str, section: &str, key: &str) -> String {
    let newline = newline_of(text);
    let parsed = lines(text);
    let mut raw: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(index) = parsed.iter().position(|line| {
        line.section.eq_ignore_ascii_case(section)
            && line
                .setting
                .as_ref()
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(key))
    }) {
        raw.remove(index);
    }
    join(raw, newline, text.ends_with('\n'))
}

/// Whether `section` has exactly this line (trimmed).
pub fn has_line(text: &str, section: &str, line: &str) -> bool {
    text.lines()
        .zip(lines(text))
        .any(|(raw, parsed)| parsed.section.eq_ignore_ascii_case(section) && raw.trim() == line)
}

/// `text` with `line` added to the end of `section`.
pub fn add_line(text: &str, section: &str, line: &str) -> String {
    let newline = newline_of(text);
    let parsed = lines(text);
    let mut raw: Vec<String> = text.lines().map(str::to_string).collect();
    append_to_section(&mut raw, &parsed, section, line.to_string());
    join(raw, newline, true)
}

/// `text` without the first occurrence of `line` in `section`.
pub fn remove_line(text: &str, section: &str, line: &str) -> String {
    let newline = newline_of(text);
    let parsed = lines(text);
    let mut raw: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(index) = raw
        .iter()
        .zip(&parsed)
        .position(|(r, p)| p.section.eq_ignore_ascii_case(section) && r.trim() == line)
    {
        raw.remove(index);
    }
    join(raw, newline, text.ends_with('\n'))
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
    fn single_settings_are_set_and_removed_in_place() {
        let text = "; mine\r\n[SystemSettings]\r\nr.Shadow=1\r\n\r\n[Other]\r\nA=1\r\n";
        let set_existing = set(text, "systemsettings", "R.SHADOW", "3");
        assert_eq!(
            set_existing,
            "; mine\r\n[SystemSettings]\r\nr.Shadow=3\r\n\r\n[Other]\r\nA=1\r\n"
        );
        let added = set(&set_existing, "SystemSettings", "r.Fog", "0");
        assert_eq!(
            added,
            "; mine\r\n[SystemSettings]\r\nr.Shadow=3\r\nr.Fog=0\r\n\r\n[Other]\r\nA=1\r\n"
        );
        assert_eq!(get(&added, "SystemSettings", "r.fog").as_deref(), Some("0"));
        let new_section = set(
            "",
            "/Script/Engine.RendererSettings",
            "r.Tonemapper.Sharpen",
            "1",
        );
        assert_eq!(
            new_section,
            "[/Script/Engine.RendererSettings]\nr.Tonemapper.Sharpen=1\n"
        );
        assert_eq!(remove(&added, "SystemSettings", "r.Fog"), set_existing);
    }

    #[test]
    fn array_lines_are_added_and_removed_whole() {
        let text = "[Core.Log]\nGlobal=log\n";
        let added = add_line(text, "Core.Log", "+Paths=../Mine");
        assert_eq!(added, "[Core.Log]\nGlobal=log\n+Paths=../Mine\n");
        assert!(has_line(&added, "core.log", "+Paths=../Mine"));
        assert_eq!(remove_line(&added, "Core.Log", "+Paths=../Mine"), text);
        assert!(is_array_op("+Paths") && !is_array_op("Paths"));
        assert_eq!(
            entries(&added)[1],
            ("Core.Log".into(), "+Paths".into(), "../Mine".into())
        );
    }

    #[test]
    fn unreal_arrays_and_repeated_keys_are_left_to_the_new_file() {
        let user = "[Paths]\n+Paths=../Mine\nKey=a\nKey=b\n";
        let v2 = "[Paths]\n+Paths=../Theirs\nKey=c\nKey=d\n";
        assert_eq!(merge(None, user, v2).text, v2);
    }
}
