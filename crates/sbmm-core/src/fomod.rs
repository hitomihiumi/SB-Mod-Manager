//! FOMOD installers: the scripted install wizards many mods ship with.
//!
//! An archive with `fomod/ModuleConfig.xml` describes its own install: steps
//! of option groups, each option with a description, a preview image, the
//! files it brings and the flags it sets, and later steps or extra files that
//! appear only for some combinations of earlier answers. Mod Organizer 2 runs
//! these as a wizard; so does the manager, and this module is everything
//! about it that is not drawing the wizard — reading the XML, working out
//! which steps and options apply, checking a selection, and listing the files
//! it installs.
//!
//! What the files *are* is not decided here. The wizard's output is laid out
//! as the script says, and that folder then goes through ordinary detection,
//! so a script written against the game root (`SB/Content/Paks/~mods/…`) and
//! one written against the mods folder (`Outfit_P.pak`) both land correctly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FomodError {
    #[error("the installer script is not valid XML: {0}")]
    Xml(String),
    #[error("the installer script is not a FOMOD module configuration")]
    NotAConfig,
}

/// A whole installer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fomod {
    pub name: String,
    /// The header image, relative to the installer's root.
    pub image: Option<String>,
    /// Installed whatever is chosen.
    pub required: Vec<FileItem>,
    pub steps: Vec<Step>,
    /// Extra files for particular combinations of flags.
    pub conditional: Vec<ConditionalInstall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    /// Shown only when this holds; always shown when absent.
    pub visible: Option<Condition>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<Plugin>,
}

/// How many options of a group may be picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    SelectExactlyOne,
    SelectAtMostOne,
    SelectAtLeastOne,
    SelectAll,
    SelectAny,
}

/// One option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub name: String,
    pub description: String,
    /// Preview image, relative to the installer's root.
    pub image: Option<String>,
    pub files: Vec<FileItem>,
    /// Flags picking this option sets, for later steps and conditional files.
    pub flags: Vec<(String, String)>,
    /// What kind of option it is when no pattern below matches.
    pub default_type: PluginType,
    /// Conditions that change its kind, first match wins.
    pub patterns: Vec<(Condition, PluginType)>,
}

/// What an option is in the current state of the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginType {
    /// Always installed; cannot be unticked.
    Required,
    /// Ticked to begin with.
    Recommended,
    Optional,
    /// Allowed, but the author warns it may not work.
    CouldBeUsable,
    /// Cannot be picked.
    NotUsable,
}

/// A file or folder an option, or the installer itself, installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    /// Relative to the installer's root.
    pub source: String,
    /// Relative to the install root. `None` means "the same as the source".
    pub destination: Option<String>,
    pub is_folder: bool,
    /// Higher wins where two items install the same file.
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalInstall {
    pub when: Condition,
    pub files: Vec<FileItem>,
}

/// A condition on the wizard's answers or on the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Condition {
    /// A flag set by a picked option has this value.
    Flag {
        name: String,
        value: String,
    },
    /// A file in the game folder is `Active`, `Inactive` or `Missing`.
    File {
        path: String,
        state: String,
    },
    All {
        of: Vec<Condition>,
    },
    Any {
        of: Vec<Condition>,
    },
    /// Game and tool version checks, which no longer mean anything here.
    Always,
}

/// Picked options, as `(step, group, plugin)` indexes.
pub type Selection = std::collections::BTreeSet<(usize, usize, usize)>;

/// Answers whether a game file is present, for `fileDependency` conditions.
pub type FileCheck<'a> = &'a dyn Fn(&str) -> bool;

// -- reading -----------------------------------------------------------------

/// Read `ModuleConfig.xml`, in whatever encoding it was saved in.
///
/// Installers written with the old FOMM tools are UTF-16 with a byte order
/// mark; newer ones UTF-8, sometimes with one too. The XML declaration's own
/// encoding attribute is not trusted, since the files often lie in it.
pub fn parse(bytes: &[u8]) -> Result<Fomod, FomodError> {
    let text = decode(bytes);
    // roxmltree rejects a declaration that names an encoding it did not
    // decode itself; the text is already decoded, so drop the declaration.
    let text = strip_declaration(&text);
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| FomodError::Xml(e.to_string()))?;
    let root = doc.root_element();
    if !root.has_tag_name("config") {
        return Err(FomodError::NotAConfig);
    }

    let name = child(root, "moduleName")
        .and_then(|n| n.text())
        .unwrap_or_default()
        .trim()
        .to_string();
    let image = child(root, "moduleImage")
        .and_then(|n| attr(n, "path"))
        .map(|p| p.replace('\\', "/"));
    let required = child(root, "requiredInstallFiles")
        .map(file_list)
        .unwrap_or_default();

    let mut steps = Vec::new();
    if let Some(list) = child(root, "installSteps") {
        for step in ordered(list, "installStep") {
            let visible = child(step, "visible").map(composite);
            let mut groups = Vec::new();
            if let Some(group_list) = child(step, "optionalFileGroups") {
                for group in ordered(group_list, "group") {
                    let plugins = child(group, "plugins")
                        .map(|p| ordered(p, "plugin").into_iter().map(plugin).collect())
                        .unwrap_or_default();
                    groups.push(Group {
                        name: attr(group, "name").unwrap_or_default(),
                        kind: group_kind(attr(group, "type").as_deref()),
                        plugins,
                    });
                }
            }
            steps.push(Step {
                name: attr(step, "name").unwrap_or_default(),
                visible,
                groups,
            });
        }
    }

    let mut conditional = Vec::new();
    if let Some(patterns) =
        child(root, "conditionalFileInstalls").and_then(|c| child(c, "patterns"))
    {
        for pattern in children(patterns, "pattern") {
            let when = child(pattern, "dependencies")
                .map(composite)
                .unwrap_or(Condition::Always);
            let files = child(pattern, "files").map(file_list).unwrap_or_default();
            conditional.push(ConditionalInstall { when, files });
        }
    }

    Ok(Fomod {
        name,
        image,
        required,
        steps,
        conditional,
    })
}

fn decode(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], little: bool| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| {
                if little {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        // UTF-16 without a mark still starts `<\0` or `\0<`.
        [b'<', 0, ..] => utf16(bytes, true),
        [0, b'<', ..] => utf16(bytes, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn strip_declaration(text: &str) -> &str {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<?xml") {
        if let Some(end) = trimmed.find("?>") {
            return &trimmed[end + 2..];
        }
    }
    trimmed
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(name))
}

fn children<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    node.children().filter(|n| n.has_tag_name(name)).collect()
}

fn attr(node: roxmltree::Node, name: &str) -> Option<String> {
    node.attribute(name).map(str::to_string)
}

/// Children in the order the list's `order` attribute asks for. The schema
/// makes `Ascending` the default, so a list without the attribute is sorted
/// by name, as Mod Organizer 2 does.
fn ordered<'a, 'i>(list: roxmltree::Node<'a, 'i>, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    let mut items = children(list, name);
    let key = |n: &roxmltree::Node| n.attribute("name").unwrap_or_default().to_lowercase();
    match list.attribute("order").unwrap_or("Ascending") {
        "Explicit" => {}
        "Descending" => items.sort_by_key(|n| std::cmp::Reverse(key(n))),
        _ => items.sort_by_key(key),
    }
    items
}

fn group_kind(kind: Option<&str>) -> GroupKind {
    match kind {
        Some("SelectExactlyOne") => GroupKind::SelectExactlyOne,
        Some("SelectAtMostOne") => GroupKind::SelectAtMostOne,
        Some("SelectAtLeastOne") => GroupKind::SelectAtLeastOne,
        Some("SelectAll") => GroupKind::SelectAll,
        _ => GroupKind::SelectAny,
    }
}

fn plugin_type(name: Option<&str>) -> PluginType {
    match name {
        Some("Required") => PluginType::Required,
        Some("Recommended") => PluginType::Recommended,
        Some("CouldBeUsable") => PluginType::CouldBeUsable,
        Some("NotUsable") => PluginType::NotUsable,
        _ => PluginType::Optional,
    }
}

fn plugin(node: roxmltree::Node) -> Plugin {
    let description = child(node, "description")
        .and_then(|d| d.text())
        .unwrap_or_default()
        .trim()
        .to_string();
    let image = child(node, "image")
        .and_then(|i| attr(i, "path"))
        .map(|p| p.replace('\\', "/"));
    let files = child(node, "files").map(file_list).unwrap_or_default();
    let flags = child(node, "conditionFlags")
        .map(|f| {
            children(f, "flag")
                .into_iter()
                .map(|flag| {
                    (
                        attr(flag, "name").unwrap_or_default(),
                        flag.text().unwrap_or_default().trim().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let descriptor = child(node, "typeDescriptor");
    let (default_type, patterns) = match descriptor {
        Some(d) => match (child(d, "type"), child(d, "dependencyType")) {
            (Some(t), _) => (plugin_type(t.attribute("name")), Vec::new()),
            (None, Some(dep)) => {
                let default = child(dep, "defaultType")
                    .map(|t| plugin_type(t.attribute("name")))
                    .unwrap_or(PluginType::Optional);
                let patterns = child(dep, "patterns")
                    .map(|p| {
                        children(p, "pattern")
                            .into_iter()
                            .map(|pattern| {
                                let when = child(pattern, "dependencies")
                                    .map(composite)
                                    .unwrap_or(Condition::Always);
                                let kind = child(pattern, "type")
                                    .map(|t| plugin_type(t.attribute("name")))
                                    .unwrap_or(default);
                                (when, kind)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (default, patterns)
            }
            (None, None) => (PluginType::Optional, Vec::new()),
        },
        None => (PluginType::Optional, Vec::new()),
    };

    Plugin {
        name: attr(node, "name").unwrap_or_default(),
        description,
        image,
        files,
        flags,
        default_type,
        patterns,
    }
}

fn file_list(node: roxmltree::Node) -> Vec<FileItem> {
    node.children()
        .filter(|n| n.has_tag_name("file") || n.has_tag_name("folder"))
        .map(|n| FileItem {
            source: attr(n, "source").unwrap_or_default().replace('\\', "/"),
            destination: attr(n, "destination").map(|d| d.replace('\\', "/")),
            is_folder: n.has_tag_name("folder"),
            priority: n
                .attribute("priority")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
        })
        .collect()
}

/// A `compositeDependency`: its children, joined by its `operator`.
fn composite(node: roxmltree::Node) -> Condition {
    let parts: Vec<Condition> = node
        .children()
        .filter(|n| n.is_element())
        .map(|n| match n.tag_name().name() {
            "flagDependency" => Condition::Flag {
                name: attr(n, "flag").unwrap_or_default(),
                value: attr(n, "value").unwrap_or_default(),
            },
            "fileDependency" => Condition::File {
                path: attr(n, "file").unwrap_or_default().replace('\\', "/"),
                state: attr(n, "state").unwrap_or_else(|| "Active".into()),
            },
            "dependencies" => composite(n),
            _ => Condition::Always,
        })
        .collect();
    match node.attribute("operator") {
        Some("Or") => Condition::Any { of: parts },
        _ => Condition::All { of: parts },
    }
}

// -- running -------------------------------------------------------------------

impl Condition {
    pub fn holds(&self, flags: &BTreeMap<String, String>, file_exists: FileCheck) -> bool {
        match self {
            Condition::Flag { name, value } => {
                flags.get(name).map(String::as_str).unwrap_or("") == value
            }
            // There are no plugin files to activate in this game, so a file
            // that is present counts as active and inactive alike.
            Condition::File { path, state } => {
                let present = file_exists(path);
                if state == "Missing" {
                    !present
                } else {
                    present
                }
            }
            Condition::All { of } => of.iter().all(|c| c.holds(flags, file_exists)),
            Condition::Any { of } => of.iter().any(|c| c.holds(flags, file_exists)),
            Condition::Always => true,
        }
    }
}

impl Plugin {
    /// What this option is, given the flags set so far.
    pub fn kind(&self, flags: &BTreeMap<String, String>, file_exists: FileCheck) -> PluginType {
        self.patterns
            .iter()
            .find(|(when, _)| when.holds(flags, file_exists))
            .map(|(_, kind)| *kind)
            .unwrap_or(self.default_type)
    }
}

/// The wizard's state for a selection: which steps show, what each option
/// is, and the flags set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardState {
    pub visible: Vec<bool>,
    /// `[step][group][plugin]`.
    pub kinds: Vec<Vec<Vec<PluginType>>>,
    pub flags: BTreeMap<String, String>,
    /// What stops the install, in words, e.g. "Body: choose one option".
    pub problems: Vec<String>,
}

/// Walk the steps in order, as the wizard does: a step's visibility and its
/// options' kinds depend on the flags the visible steps before it set.
pub fn evaluate(fomod: &Fomod, selection: &Selection, file_exists: FileCheck) -> WizardState {
    let mut flags = BTreeMap::new();
    let mut visible = Vec::new();
    let mut kinds = Vec::new();
    let mut problems = Vec::new();

    for (s, step) in fomod.steps.iter().enumerate() {
        let shown = step
            .visible
            .as_ref()
            .is_none_or(|c| c.holds(&flags, file_exists));
        visible.push(shown);
        let mut step_kinds = Vec::new();
        for (g, group) in step.groups.iter().enumerate() {
            let group_kinds: Vec<PluginType> = group
                .plugins
                .iter()
                .map(|p| p.kind(&flags, file_exists))
                .collect();
            if shown {
                let picked = (0..group.plugins.len())
                    .filter(|p| selection.contains(&(s, g, *p)))
                    .count();
                let problem = match group.kind {
                    GroupKind::SelectExactlyOne if picked != 1 => Some("choose one option"),
                    GroupKind::SelectAtMostOne if picked > 1 => Some("choose at most one option"),
                    GroupKind::SelectAtLeastOne if picked == 0 => {
                        Some("choose at least one option")
                    }
                    _ => None,
                };
                if let Some(problem) = problem {
                    problems.push(format!("{}: {problem}", group.name));
                }
                for (p, kind) in group_kinds.iter().enumerate() {
                    let on = selection.contains(&(s, g, p));
                    if on && *kind == PluginType::NotUsable {
                        problems.push(format!(
                            "{}: “{}” cannot be used",
                            group.name, group.plugins[p].name
                        ));
                    }
                    if on {
                        for (name, value) in &group.plugins[p].flags {
                            flags.insert(name.clone(), value.clone());
                        }
                    }
                }
            }
            step_kinds.push(group_kinds);
        }
        kinds.push(step_kinds);
    }

    WizardState {
        visible,
        kinds,
        flags,
        problems,
    }
}

/// The selection the wizard opens with: required and recommended options
/// ticked, every option of a select-all group, and the first usable option
/// of a group that needs one and has none.
pub fn defaults(fomod: &Fomod, file_exists: FileCheck) -> Selection {
    let mut selection = Selection::new();
    // Kinds can depend on flags from earlier steps, so each step's defaults
    // are taken with the defaults before it already in place.
    for (s, step) in fomod.steps.iter().enumerate() {
        let state = evaluate(fomod, &selection, file_exists);
        for (g, group) in step.groups.iter().enumerate() {
            let kinds = &state.kinds[s][g];
            let usable = |p: usize| kinds[p] != PluginType::NotUsable;
            for (p, kind) in kinds.iter().enumerate() {
                let pick = match group.kind {
                    GroupKind::SelectAll => true,
                    _ => matches!(kind, PluginType::Required | PluginType::Recommended),
                };
                if pick && *kind != PluginType::NotUsable {
                    selection.insert((s, g, p));
                }
            }
            let picked: Vec<usize> = (0..group.plugins.len())
                .filter(|p| selection.contains(&(s, g, *p)))
                .collect();
            match group.kind {
                GroupKind::SelectExactlyOne | GroupKind::SelectAtLeastOne if picked.is_empty() => {
                    if let Some(first) = (0..group.plugins.len()).find(|p| usable(*p)) {
                        selection.insert((s, g, first));
                    }
                }
                // One recommended too many: keep the first.
                GroupKind::SelectExactlyOne | GroupKind::SelectAtMostOne if picked.len() > 1 => {
                    for p in &picked[1..] {
                        selection.remove(&(s, g, *p));
                    }
                }
                _ => {}
            }
        }
    }
    selection
}

/// Everything a selection installs, lowest priority first, so laying the
/// items down in order lets the higher priority win a clash. Options on
/// hidden steps install nothing.
pub fn resolve(fomod: &Fomod, selection: &Selection, file_exists: FileCheck) -> Vec<FileItem> {
    let state = evaluate(fomod, selection, file_exists);
    let mut items: Vec<(usize, FileItem)> = Vec::new();
    let mut push = |item: &FileItem| items.push((items.len(), item.clone()));

    for item in &fomod.required {
        push(item);
    }
    for (s, g, p) in selection {
        if !state.visible.get(*s).copied().unwrap_or(false) {
            continue;
        }
        if let Some(plugin) = fomod
            .steps
            .get(*s)
            .and_then(|step| step.groups.get(*g))
            .and_then(|group| group.plugins.get(*p))
        {
            for item in &plugin.files {
                push(item);
            }
        }
    }
    for conditional in &fomod.conditional {
        if conditional.when.holds(&state.flags, file_exists) {
            for item in &conditional.files {
                push(item);
            }
        }
    }
    // Stable on equal priority, so document order decides between equals.
    items.sort_by_key(|(order, item)| (item.priority, *order));
    items.into_iter().map(|(_, item)| item).collect()
}

/// Where a file item lands, relative to the install root. A `destination`
/// that is missing means "where the source is"; an empty one means the root.
pub fn destination_of(item: &FileItem) -> String {
    match &item.destination {
        None => item.source.trim_matches('/').to_string(),
        Some(d) if item.is_folder => d.trim_matches('/').to_string(),
        Some(d) if d.trim_matches('/').is_empty() || d.ends_with('/') => {
            let name = item.source.rsplit('/').next().unwrap_or(&item.source);
            let dir = d.trim_matches('/');
            if dir.is_empty() {
                name.to_string()
            } else {
                format!("{dir}/{name}")
            }
        }
        Some(d) => d.trim_matches('/').to_string(),
    }
}

/// Picked options named rather than numbered, so a choice survives into the
/// next version of the installer even if options were added or reordered.
pub fn selection_names(fomod: &Fomod, selection: &Selection) -> Vec<[String; 3]> {
    selection
        .iter()
        .filter_map(|(s, g, p)| {
            let step = fomod.steps.get(*s)?;
            let group = step.groups.get(*g)?;
            let plugin = group.plugins.get(*p)?;
            Some([step.name.clone(), group.name.clone(), plugin.name.clone()])
        })
        .collect()
}

/// The indexes for remembered names, skipping any that no longer exist.
pub fn selection_from_names(fomod: &Fomod, names: &[[String; 3]]) -> Selection {
    let mut selection = Selection::new();
    for [step_name, group_name, plugin_name] in names {
        for (s, step) in fomod.steps.iter().enumerate() {
            if &step.name != step_name {
                continue;
            }
            for (g, group) in step.groups.iter().enumerate() {
                if &group.name != group_name {
                    continue;
                }
                if let Some(p) = group.plugins.iter().position(|p| &p.name == plugin_name) {
                    selection.insert((s, g, p));
                }
            }
        }
    }
    selection
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <moduleName>Planet Diving Suit</moduleName>
  <moduleImage path="fomod\images\header.png"/>
  <requiredInstallFiles>
    <file source="Core\Shared_P.pak" destination="Shared_P.pak"/>
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Colour">
      <optionalFileGroups order="Explicit">
        <group name="Colour" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="Black">
              <description>The original colour.</description>
              <image path="fomod\images\black.jpg"/>
              <files><folder source="Black" destination=""/></files>
              <conditionFlags><flag name="colour">black</flag></conditionFlags>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
            <plugin name="White">
              <description>Snow white.</description>
              <files><folder source="White" destination=""/></files>
              <conditionFlags><flag name="colour">white</flag></conditionFlags>
              <typeDescriptor><type name="Recommended"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Extras for white">
      <visible><flagDependency flag="colour" value="white"/></visible>
      <optionalFileGroups order="Explicit">
        <group name="Extras" type="SelectAny">
          <plugins order="Explicit">
            <plugin name="Gloves">
              <description>Matching gloves.</description>
              <files><file source="Extras\Gloves_P.pak" destination="" priority="1"/></files>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
            <plugin name="Broken">
              <description>Does not work.</description>
              <typeDescriptor>
                <dependencyType>
                  <defaultType name="Optional"/>
                  <patterns><pattern>
                    <dependencies operator="And"><flagDependency flag="colour" value="white"/></dependencies>
                    <type name="NotUsable"/>
                  </pattern></patterns>
                </dependencyType>
              </typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls><patterns>
    <pattern>
      <dependencies operator="Or"><flagDependency flag="colour" value="black"/></dependencies>
      <files><file source="Patches\Black_Patch_P.pak"/></files>
    </pattern>
  </patterns></conditionalFileInstalls>
</config>"#;

    fn utf16(text: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            out.extend(unit.to_le_bytes());
        }
        out
    }

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn a_utf16_installer_is_read_whole() {
        let fomod = parse(&utf16(CONFIG)).unwrap();
        assert_eq!(fomod.name, "Planet Diving Suit");
        assert_eq!(fomod.image.as_deref(), Some("fomod/images/header.png"));
        assert_eq!(fomod.required.len(), 1);
        assert_eq!(fomod.steps.len(), 2);
        let colour = &fomod.steps[0].groups[0];
        assert_eq!(colour.kind, GroupKind::SelectExactlyOne);
        assert_eq!(colour.plugins[0].name, "Black");
        assert_eq!(
            colour.plugins[0].image.as_deref(),
            Some("fomod/images/black.jpg")
        );
        assert_eq!(colour.plugins[0].files[0].source, "Black");
        assert_eq!(fomod.conditional.len(), 1);
    }

    #[test]
    fn utf8_with_or_without_a_mark_reads_the_same() {
        let plain = parse(CONFIG.as_bytes()).unwrap();
        let mut marked = vec![0xEF, 0xBB, 0xBF];
        marked.extend(CONFIG.as_bytes());
        assert_eq!(parse(&marked).unwrap(), plain);
        assert_eq!(parse(&utf16(CONFIG)).unwrap(), plain);
    }

    #[test]
    fn rubbish_is_refused_rather_than_guessed_at() {
        assert!(matches!(parse(b"not xml <"), Err(FomodError::Xml(_))));
        assert_eq!(parse(b"<other/>"), Err(FomodError::NotAConfig));
    }

    #[test]
    fn defaults_take_the_recommended_option() {
        let fomod = parse(CONFIG.as_bytes()).unwrap();
        let selection = defaults(&fomod, &none);
        assert!(selection.contains(&(0, 0, 1)), "White is recommended");
        assert!(!selection.contains(&(0, 0, 0)));
    }

    #[test]
    fn a_later_step_shows_only_for_the_answer_it_depends_on() {
        let fomod = parse(CONFIG.as_bytes()).unwrap();
        let white: Selection = [(0, 0, 1)].into();
        let black: Selection = [(0, 0, 0)].into();
        assert_eq!(evaluate(&fomod, &white, &none).visible, vec![true, true]);
        assert_eq!(evaluate(&fomod, &black, &none).visible, vec![true, false]);

        let state = evaluate(&fomod, &white, &none);
        assert_eq!(state.kinds[1][0][1], PluginType::NotUsable);
        assert_eq!(state.flags.get("colour").map(String::as_str), Some("white"));
    }

    #[test]
    fn group_rules_are_checked() {
        let fomod = parse(CONFIG.as_bytes()).unwrap();
        let both: Selection = [(0, 0, 0), (0, 0, 1)].into();
        assert_eq!(
            evaluate(&fomod, &both, &none).problems,
            vec!["Colour: choose one option".to_string()]
        );
        assert!(!evaluate(&fomod, &Selection::new(), &none)
            .problems
            .is_empty());
        let unusable: Selection = [(0, 0, 1), (1, 0, 1)].into();
        assert_eq!(
            evaluate(&fomod, &unusable, &none).problems,
            vec!["Extras: “Broken” cannot be used".to_string()]
        );
    }

    #[test]
    fn the_install_follows_the_answers() {
        let fomod = parse(CONFIG.as_bytes()).unwrap();
        let sources = |selection: &Selection| -> Vec<String> {
            resolve(&fomod, selection, &none)
                .into_iter()
                .map(|i| i.source)
                .collect()
        };
        // White with gloves: required, white, then gloves (priority 1, last).
        assert_eq!(
            sources(&[(0, 0, 1), (1, 0, 0)].into()),
            vec!["Core/Shared_P.pak", "White", "Extras/Gloves_P.pak"]
        );
        // Black: the gloves step is hidden, so a stale tick there installs
        // nothing, and the black patch comes in through its condition.
        assert_eq!(
            sources(&[(0, 0, 0), (1, 0, 0)].into()),
            vec!["Core/Shared_P.pak", "Black", "Patches/Black_Patch_P.pak"]
        );
    }

    #[test]
    fn destinations_follow_the_fomod_rules() {
        let item = |source: &str, destination: Option<&str>, is_folder| FileItem {
            source: source.into(),
            destination: destination.map(str::to_string),
            is_folder,
            priority: 0,
        };
        assert_eq!(destination_of(&item("a/b.pak", None, false)), "a/b.pak");
        assert_eq!(destination_of(&item("a/b.pak", Some(""), false)), "b.pak");
        assert_eq!(
            destination_of(&item("a/b.pak", Some("x/"), false)),
            "x/b.pak"
        );
        assert_eq!(
            destination_of(&item("a/b.pak", Some("x/c.pak"), false)),
            "x/c.pak"
        );
        assert_eq!(destination_of(&item("Black", Some(""), true)), "");
        assert_eq!(
            destination_of(&item("Black", Some("SB/Content"), true)),
            "SB/Content"
        );
    }

    #[test]
    fn a_remembered_choice_survives_reordering() {
        let fomod = parse(CONFIG.as_bytes()).unwrap();
        let picked: Selection = [(0, 0, 1), (1, 0, 0)].into();
        let names = selection_names(&fomod, &picked);
        assert_eq!(
            names[0],
            ["Colour".to_string(), "Colour".into(), "White".into()]
        );
        assert_eq!(selection_from_names(&fomod, &names), picked);
        let missing = [["Colour".to_string(), "Colour".into(), "Purple".into()]];
        assert!(selection_from_names(&fomod, &missing).is_empty());
    }

    #[test]
    fn lists_without_an_order_are_sorted_by_name_as_the_schema_says() {
        let xml = r#"<config><moduleName>M</moduleName><installSteps>
            <installStep name="Two"/><installStep name="one"/></installSteps></config>"#;
        let names: Vec<String> = parse(xml.as_bytes())
            .unwrap()
            .steps
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["one", "Two"]);
    }
}
