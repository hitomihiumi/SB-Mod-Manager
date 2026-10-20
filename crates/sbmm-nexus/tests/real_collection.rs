//! The parser against a real Stellar Blade collection manifest.
//!
//! The fixture is trimmed from a published 314-mod collection: every entry
//! kept is verbatim, and one of each distinct `details.type` is present, so
//! the shapes are the ones Nexus actually writes rather than ones invented to
//! match the parser.

use sbmm_nexus::collection;

const MANIFEST: &str = include_str!("fixtures/collection.json");

#[test]
fn every_entry_of_a_real_collection_is_understood() {
    let parsed = collection::parse(MANIFEST).expect("a published manifest must parse");

    assert_eq!(parsed.name, "DEGEN: An NSFW AIO pack by dae");
    assert!(
        parsed.manual.is_empty(),
        "every entry names a Nexus mod and file, so none needs doing by hand: {:?}",
        parsed.manual
    );
    assert_eq!(
        parsed.mods.len(),
        9,
        "one entry per distinct mod type in the fixture"
    );

    for entry in &parsed.mods {
        assert!(entry.mod_id > 0, "{} has no mod id", entry.name);
        assert!(entry.file_id > 0, "{} has no file id", entry.name);
        assert!(!entry.name.is_empty());
    }
}

/// Optional entries are a choice the user makes, so mistaking one for required
/// would install things they did not ask for. The real manifest marks 28 of
/// its 314 that way.
#[test]
fn optional_entries_are_kept_apart_from_required_ones() {
    let parsed = collection::parse(MANIFEST).unwrap();
    let optional: Vec<&str> = parsed
        .mods
        .iter()
        .filter(|m| m.optional)
        .map(|m| m.name.as_str())
        .collect();

    assert!(
        !optional.is_empty(),
        "the fixture keeps at least one optional entry"
    );
    assert!(
        parsed.mods.iter().any(|m| !m.optional),
        "and at least one required one"
    );
}

/// Ids arrive as JSON numbers here, but other manifests write them as strings;
/// the value has to come out the same either way.
#[test]
fn ids_are_read_as_numbers_whichever_way_they_are_written() {
    let parsed = collection::parse(MANIFEST).unwrap();
    let first = &parsed.mods[0];

    let as_strings = MANIFEST.replace(
        &format!("\"modId\": {}", first.mod_id),
        &format!("\"modId\": \"{}\"", first.mod_id),
    );
    let restringed = collection::parse(&as_strings).unwrap();
    assert_eq!(restringed.mods[0].mod_id, first.mod_id);
}

/// The version each entry pins, which is what an update check compares
/// against once the mod is installed.
#[test]
fn entries_carry_the_version_the_collection_pins() {
    let parsed = collection::parse(MANIFEST).unwrap();
    assert!(
        parsed.mods.iter().any(|m| m.version.is_some()),
        "a real collection pins versions"
    );
}
