//! The Rust registry mirrors `vivid-protocol-1.5-registry.toml`, the normative assignment source.
//!
//! Every numeric section is compared in both directions, so an assignment added to only one side
//! fails here instead of surfacing as a peer interoperability bug.
use std::collections::{BTreeMap, BTreeSet};

use vivid_protocol::{
    messages::{self, TrackKind},
    registry,
    wire::ConnectionKind,
};

const REGISTRY: &str = include_str!("../vivid-protocol-1.5-registry.toml");
const RUST_REGISTRY: &str = include_str!("../src/registry.rs");
const RUST_MESSAGES: &str = include_str!("../src/messages.rs");

/// One `[[section]]` table of the registry: key to the raw TOML value text.
type Entry = BTreeMap<String, String>;

fn sections(name: &str) -> Vec<Entry> {
    let header = format!("[[{name}]]");
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    for line in REGISTRY.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            entries.extend(current.take());
            if line == header {
                current = Some(Entry::new());
            }
            continue;
        }
        if let (Some(entry), Some((key, value))) = (current.as_mut(), line.split_once(" = ")) {
            entry.insert(key.to_owned(), value.to_owned());
        }
    }
    entries.extend(current);
    assert!(!entries.is_empty(), "registry has no [[{name}]] entries");
    entries
}

fn integer(text: &str) -> u64 {
    let text = text.replace('_', "");
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}

fn string(text: &str) -> &str {
    text.trim_matches('"')
}

fn strings(text: &str) -> Vec<&str> {
    text.trim_matches(['[', ']'])
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(string)
        .collect()
}

/// `name = value` pairs of a registry section.
fn assignments(section: &str) -> BTreeMap<String, u64> {
    sections(section)
        .iter()
        .map(|entry| (string(&entry["name"]).to_owned(), integer(&entry["value"])))
        .collect()
}

/// `pub const NAME: T = VALUE;` items of `mod module` in `src/registry.rs`.
fn rust_constants(module: &str) -> BTreeMap<String, u64> {
    let start = RUST_REGISTRY
        .find(&format!("mod {module} {{"))
        .unwrap_or_else(|| panic!("src/registry.rs has no module {module}"));
    let body = &RUST_REGISTRY[start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    body.lines()
        .filter_map(|line| {
            let item = line.trim().strip_prefix("pub const ")?;
            let (name, rest) = item.split_once(':')?;
            let value = rest.split_once('=')?.1.trim().trim_end_matches(';');
            (!name.contains(' ')).then(|| (name.to_owned(), integer(value)))
        })
        .collect()
}

#[test]
fn numeric_assignments_match_the_registry_in_both_directions() {
    for (section, module) in [
        ("record", "record"),
        ("limit", "limit"),
        ("error", "error"),
        ("channel_advance_reason", "channel_advance_reason"),
    ] {
        assert_eq!(
            rust_constants(module),
            assignments(section),
            "registry::{module} differs from [[{section}]]"
        );
    }
}

/// The record names `messages` re-exports by name from the crate-private registry.
fn reexported_records() -> BTreeSet<String> {
    let list = RUST_MESSAGES
        .split_once("pub use crate::registry::record::{")
        .expect("src/messages.rs re-exports the record types")
        .1;
    let list = &list[..list.find("};").unwrap()];
    list.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// The `ERROR_*` code constants `messages` defines.
fn message_error_codes() -> BTreeSet<String> {
    RUST_MESSAGES
        .lines()
        .filter_map(|line| line.strip_prefix("pub const ERROR_"))
        .map(|rest| format!("ERROR_{}", rest.split(':').next().unwrap()))
        .collect()
}

#[test]
fn messages_names_every_live_record_and_error() {
    let live_records: BTreeSet<_> = rust_constants("record")
        .into_iter()
        .filter(|(_, value)| !messages::is_retired_record(u16::try_from(*value).unwrap()))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(reexported_records(), live_records);
    let errors: BTreeSet<_> = rust_constants("error")
        .into_keys()
        .map(|name| format!("ERROR_{name}"))
        .collect();
    assert_eq!(message_error_codes(), errors);
}

#[test]
fn record_states_and_ranges_match_the_registry() {
    for entry in sections("record") {
        let value = u16::try_from(integer(&entry["value"])).unwrap();
        assert_eq!(
            messages::is_retired_record(value),
            string(&entry["state"]) == "retired",
            "{}",
            entry["name"]
        );
    }
    for range in sections("record_range") {
        let start = u16::try_from(integer(&range["start"])).unwrap();
        let end = u16::try_from(integer(&range["end"])).unwrap();
        let experimental = string(&range["state"]) == "experimental";
        let vendor = string(&range["purpose"]).starts_with("vendor");
        for value in [start, end] {
            assert_eq!(
                messages::is_experimental_record(value),
                experimental,
                "{value:#x}"
            );
            assert_eq!(messages::is_vendor_record(value), vendor, "{value:#x}");
        }
    }
}

#[test]
fn registered_error_membership_matches_the_registry() {
    let assigned: BTreeSet<u64> = assignments("error").into_values().collect();
    for code in 0..=assigned.last().unwrap() + 1 {
        assert_eq!(
            messages::is_registered_error(code),
            assigned.contains(&code),
            "code {code}"
        );
    }
}

#[test]
fn profiles_and_prerequisites_match_the_registry() {
    let mut names = BTreeSet::new();
    for profile in sections("profile") {
        let name = string(&profile["name"]);
        assert_eq!(
            registry::prerequisites(name),
            Some(strings(&profile["prerequisites"]).as_slice()),
            "{name}"
        );
        names.insert(name.to_owned());
    }
    for profile in sections("surface_profile") {
        names.insert(string(&profile["name"]).to_owned());
    }
    // Every profile-name constant at the top of src/registry.rs, before the first module.
    let header = &RUST_REGISTRY[..RUST_REGISTRY.find("pub mod ").unwrap()];
    let rust: BTreeSet<String> = header
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub const "))
        .filter_map(|item| item.split_once(": &str = "))
        .map(|(_, value)| string(value.trim_end_matches(';')).to_owned())
        .collect();
    assert_eq!(rust, names);
}

#[test]
fn connection_and_track_kinds_match_the_registry() {
    let kinds = assignments("connection_kind");
    for value in kinds.values() {
        ConnectionKind::try_from(u8::try_from(*value).unwrap()).unwrap();
    }
    let next = u8::try_from(kinds.values().max().unwrap() + 1).unwrap();
    ConnectionKind::try_from(next).unwrap_err();

    for value in assignments("track_kind").values() {
        TrackKind::try_from(*value).unwrap();
    }
}
