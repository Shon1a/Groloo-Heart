//! Merging a CDN-served official collection over the inline defaults.
//!
//! This is the portable heart of the "official add-on list" upgrade layer: the
//! inline defaults are the source of truth for behaviour-critical ids and their
//! install state; the CDN may only **refine display metadata** of known ids and
//! **append** brand-new curated official cards. It can never drop, lock,
//! re-section, or flip the toggle of a known id, and it can never introduce a
//! stream source (neutral-conduit stance).
//!
//! VENDORED from `Stredio-Heart@0.1.0 src/collection.rs`, behaviour-frozen. Every
//! guard, every allow-list and all six of its tests come across intact. The one
//! structural change is that [`MergeReport`] now records *why* a record was
//! skipped and *that* an icon was dropped, instead of only *which* id it was.
//! Heart threw that information away because its only consumer was a repaint
//! decision; the envelope needs it, because "the CDN sent us a stream-bearing
//! official card" and "the CDN sent us a community card" are the same
//! observation until someone writes down which one happened.
//!
//! Note also the un-guarded twin this replaces: `official.ts`'s `loadViaJs()`
//! fallback hands raw CDN descriptors straight to the UI, so a record carrying a
//! `transportUrl` is rejected on the Rust path and accepted on the JS one. The
//! guards below are only worth having if they are the only path.

use crate::envelope::{Warning, WarningCode};
use crate::salvage;
use crate::types::{AddonCollection, AddonDescriptor, Section};
use serde_json::Value;

/// Why a CDN record never made it into the merged list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No `id` — there is nothing to key it by, so it cannot be merged or updated.
    NoId,
    /// Not `section: "official"`. The official collection may only carry official
    /// cards; a community card arriving here is a data error, not a new install.
    NotOfficial,
    /// Carries a transport or a `stream` resource. Curated official cards are
    /// metadata-only by policy — this is the neutral-conduit stance, compiled in.
    HasStream,
}

impl SkipReason {
    pub fn warning_code(self) -> WarningCode {
        match self {
            SkipReason::NoId => WarningCode::SkippedNoId,
            SkipReason::NotOfficial => WarningCode::SkippedNotOfficial,
            SkipReason::HasStream => WarningCode::SkippedHasStream,
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            SkipReason::NoId => "record has no id",
            SkipReason::NotOfficial => "record is not in the official section",
            SkipReason::HasStream => "curated official cards may not carry a stream source",
        }
    }
}

/// One rejected CDN record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The record's id, or `<empty-id>` when it had none.
    pub id: String,
    pub reason: SkipReason,
}

/// What a merge did — lets a caller repaint only when something actually changed,
/// and lets the boundary explain itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// `true` iff any field was refined or any new card appended.
    pub changed: bool,
    /// Ids of brand-new cards appended.
    pub added: Vec<String>,
    /// Ids of known cards whose display metadata was refined.
    pub refined: Vec<String>,
    /// Records rejected by a guard, with the guard that rejected them.
    pub skipped: Vec<Skipped>,
    /// Ids whose CDN-supplied `icon_cls` failed the allow-list and was ignored.
    pub dropped_icons: Vec<String>,
}

impl MergeReport {
    /// Just the ids, in order — what Heart's `skipped` used to be.
    pub fn skipped_ids(&self) -> Vec<&str> {
        self.skipped.iter().map(|s| s.id.as_str()).collect()
    }

    /// The report as boundary warnings, ready to hang off an envelope.
    pub fn warnings(&self) -> Vec<Warning> {
        let mut out: Vec<Warning> = self
            .skipped
            .iter()
            .map(|s| Warning::new(s.reason.warning_code(), s.id.clone(), s.reason.detail()))
            .collect();
        out.extend(self.dropped_icons.iter().map(|id| {
            Warning::new(
                WarningCode::DroppedUnsafeIcon,
                id.clone(),
                "iconCls failed the class-token allow-list and was ignored",
            )
        }));
        out
    }
}

/// Max length + charset for a CDN-supplied `icon_cls`. It is emitted into an HTML
/// `class` attribute by UI shells, so anything outside `[A-Za-z0-9 _-]` is dropped.
const ICON_MAX: usize = 40;

/// Returns the value only if it is a safe class token (else `None`).
pub fn safe_icon(v: &str) -> Option<String> {
    if v.len() <= ICON_MAX
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-')
    {
        Some(v.to_string())
    } else {
        None
    }
}

/// Does this descriptor carry a stream source? Curated official cards must not.
pub fn has_stream(a: &AddonDescriptor) -> bool {
    a.transport_url.is_some() || a.resources.iter().any(|r| r == "stream")
}

/// UI version string: prefer explicit `ver`, else derive `v{version}`.
fn ver_of(raw: &AddonDescriptor) -> Option<String> {
    if let Some(v) = &raw.ver {
        return Some(v.clone());
    }
    raw.version.as_ref().map(|v| format!("v{v}"))
}

/// Take the CDN's `icon_cls` if it is a safe class token; note the id when it is
/// not, so the caller can say so out loud instead of silently keeping the old one.
fn take_icon(raw: &AddonDescriptor, dropped: &mut Vec<String>) -> Option<String> {
    let candidate = raw.icon_cls.as_deref()?;
    match safe_icon(candidate) {
        Some(ic) => Some(ic),
        None => {
            dropped.push(raw.id.clone());
            None
        }
    }
}

/// Refine an existing (known) descriptor with display-only fields. Returns `true`
/// iff a field actually changed. `id`, `section`, `installed`, `locked`,
/// `no_config`, and `preview` are never copied onto a known entry.
fn upsert_known(
    cur: &mut AddonDescriptor,
    raw: &AddonDescriptor,
    dropped: &mut Vec<String>,
) -> bool {
    let mut changed = false;

    if !raw.name.is_empty() && raw.name != cur.name {
        cur.name = raw.name.clone();
        changed = true;
    }
    if let Some(v) = ver_of(raw) {
        if Some(&v) != cur.ver.as_ref() {
            cur.ver = Some(v);
            changed = true;
        }
    }
    if let Some(ic) = take_icon(raw, dropped) {
        if Some(&ic) != cur.icon_cls.as_ref() {
            cur.icon_cls = Some(ic);
            changed = true;
        }
    }
    if let Some(g) = &raw.glyph {
        if Some(g) != cur.glyph.as_ref() {
            cur.glyph = Some(g.clone());
            changed = true;
        }
    }
    if let Some(img) = &raw.img {
        if Some(img) != cur.img.as_ref() {
            cur.img = Some(img.clone());
            changed = true;
        }
    }
    // Only replace tags when the CDN actually supplies some — never wipe to empty.
    if !raw.tags.is_empty() && raw.tags != cur.tags {
        cur.tags = raw.tags.clone();
        changed = true;
    }

    changed
}

/// Build a brand-new curated official card from a CDN record, sanitising it.
fn coerce_new(raw: &AddonDescriptor, dropped: &mut Vec<String>) -> AddonDescriptor {
    let installed = raw.default_installed == Some(true) || raw.installed == Some(true);
    AddonDescriptor {
        id: raw.id.clone(),
        section: Section::Official,
        name: if raw.name.is_empty() {
            raw.id.clone()
        } else {
            raw.name.clone()
        },
        version: raw.version.clone(),
        ver: Some(ver_of(raw).unwrap_or_default()),
        icon_cls: Some(take_icon(raw, dropped).unwrap_or_else(|| "puzzle".to_string())),
        glyph: Some(raw.glyph.clone().unwrap_or_default()),
        img: raw.img.clone(),
        tags: raw.tags.clone(),
        default_installed: raw.default_installed,
        installed: Some(installed),
        no_config: Some(raw.no_config == Some(true)),
        preview: Some(raw.preview == Some(true)),
        locked: Some(raw.locked == Some(true)),
        kind: raw.kind.clone(),
        types: raw.types.clone(),
        resources: raw.resources.clone(),
        // Never carry a transport into a curated UI card (neutral-conduit).
        transport_url: None,
        config_ref: raw.config_ref.clone(),
        flags: raw.flags.clone(),
    }
}

/// Parse an `addons.json` payload document, reporting every record the leniency
/// cost.
///
/// **This is what makes [`SkipReason::NoId`] reachable.** A `Vec<AddonDescriptor>`
/// deserialized in one go dies on its worst element, so a single CDN record with a
/// malformed field failed the ENTIRE payload and the inline defaults stood — the
/// exact opposite of this module's own "one bad row costs that row" rule, and a
/// guard branch that could not be reached by any input. (The other half of that
/// fix is [`AddonDescriptor::id`] gaining a `#[serde(default)]`: a record *missing*
/// an id now deserializes to `""`, which is what the `NoId` guard was written to
/// catch and had never once seen.)
///
/// The blast radius is worth naming: the payload is a file the CDN serves to every
/// client, so "one record kills the collection" is one bad deploy away from every
/// television falling back to the four inline cards at once. Dropping the record
/// and warning is strictly smaller — and unlike the merge guards, which are policy,
/// this one is only ever "we could not read it".
///
/// The `Err` case is document-level only: not JSON, not an object, or a `schema`
/// the core cannot read as a number. Those keep the caller's inline list, which is
/// the contract `merge_official` at the boundary depends on.
pub fn parse_payload(json: &str) -> Result<(AddonCollection, Vec<Warning>), serde_json::Error> {
    let mut obj = match serde_json::from_str::<Value>(json)? {
        Value::Object(o) => o,
        // A bare array is the shape `official.ts`'s deleted `loadViaJs()` fallback
        // used to hand around. It is not a document, it carries no `schema`, and
        // accepting it here would put the schema gate back on the optional path.
        other => return serde_json::from_value::<AddonCollection>(other).map(|c| (c, Vec::new())),
    };

    // The addons are lifted out first so the rest of the document — `schema` above
    // all — is still validated by the derive rather than re-implemented here.
    let raw_addons = obj.remove("addons");
    let mut collection: AddonCollection = serde_json::from_value(Value::Object(obj))?;
    let mut warnings = Vec::new();
    collection.addons = salvage::items("addons", raw_addons, &mut warnings)?;
    Ok((collection, warnings))
}

/// Merge a CDN-served official collection into `inline` (the boot/fallback set).
///
/// Mutates `inline` in place and reports what happened. Guards skip records with
/// no id, non-official records, and any record that carries a stream source.
pub fn merge_official(inline: &mut Vec<AddonDescriptor>, cdn: &[AddonDescriptor]) -> MergeReport {
    let mut report = MergeReport::default();
    for raw in cdn {
        if raw.id.is_empty() {
            report.skipped.push(Skipped {
                id: "<empty-id>".to_string(),
                reason: SkipReason::NoId,
            });
            continue;
        }
        if raw.section != Section::Official {
            report.skipped.push(Skipped {
                id: raw.id.clone(),
                reason: SkipReason::NotOfficial,
            });
            continue;
        }
        if has_stream(raw) {
            report.skipped.push(Skipped {
                id: raw.id.clone(),
                reason: SkipReason::HasStream,
            });
            continue;
        }
        match inline.iter().position(|x| x.id == raw.id) {
            // `get_mut` rather than `inline[pos]`: the index is one we just
            // computed and cannot be stale, but a crate that denies
            // `indexing_slicing` denies it everywhere or it is not a rule.
            Some(pos) => {
                if let Some(cur) = inline.get_mut(pos) {
                    if upsert_known(cur, raw, &mut report.dropped_icons) {
                        report.changed = true;
                        report.refined.push(raw.id.clone());
                    }
                }
            }
            None => {
                let card = coerce_new(raw, &mut report.dropped_icons);
                inline.push(card);
                report.changed = true;
                report.added.push(raw.id.clone());
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(json: &str) -> AddonDescriptor {
        serde_json::from_str(json).unwrap()
    }

    fn four() -> Vec<AddonDescriptor> {
        serde_json::from_str(
            r#"[
          {"id":"upcoming","section":"official","name":"Upcoming","ver":"v1.3.0","iconCls":"puzzle","glyph":"A","tags":["catalog","metadata"],"installed":true,"noConfig":true,"preview":true},
          {"id":"studios","section":"official","name":"Studios","ver":"v1.0.0","iconCls":"puzzle","glyph":"B","tags":["catalog","metadata"],"installed":true,"noConfig":true,"preview":true},
          {"id":"catalog","section":"official","name":"Catalog","ver":"v1.0.0","iconCls":"puzzle","glyph":"C","tags":["catalog","metadata"],"installed":true},
          {"id":"providers","section":"official","name":"Providers","ver":"v1.0.0","iconCls":"puzzle","glyph":"D","tags":["catalog","metadata"],"installed":false}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn identical_display_is_noop() {
        let mut inline = four();
        // CDN payload uses defaultInstalled + version instead of installed, same display.
        let cdn: Vec<AddonDescriptor> = serde_json::from_str(
            r#"[
          {"id":"upcoming","section":"official","name":"Upcoming","version":"1.3.0","ver":"v1.3.0","iconCls":"puzzle","glyph":"A","tags":["catalog","metadata"],"defaultInstalled":true,"noConfig":true,"preview":true,"kind":"discovery"},
          {"id":"studios","section":"official","name":"Studios","version":"1.0.0","ver":"v1.0.0","iconCls":"puzzle","glyph":"B","tags":["catalog","metadata"],"defaultInstalled":true,"noConfig":true,"preview":true,"kind":"discovery"},
          {"id":"catalog","section":"official","name":"Catalog","version":"1.0.0","ver":"v1.0.0","iconCls":"puzzle","glyph":"C","tags":["catalog","metadata"],"defaultInstalled":true,"kind":"discovery"},
          {"id":"providers","section":"official","name":"Providers","version":"1.0.0","ver":"v1.0.0","iconCls":"puzzle","glyph":"D","tags":["catalog","metadata"],"defaultInstalled":false,"kind":"discovery"}
        ]"#,
        )
        .unwrap();
        let report = merge_official(&mut inline, &cdn);
        assert!(!report.changed, "identical CDN data must be a no-op");
        assert_eq!(inline.len(), 4);
    }

    #[test]
    fn refines_display_but_never_behaviour() {
        let mut inline = four();
        let cdn = vec![desc(
            r#"{"id":"catalog","section":"official","name":"Trending & Top","ver":"v2.0.0","installed":false,"locked":true,"noConfig":true}"#,
        )];
        let report = merge_official(&mut inline, &cdn);
        let cat = inline.iter().find(|a| a.id == "catalog").unwrap();
        assert!(report.changed);
        assert_eq!(cat.name, "Trending & Top"); // display refined
        assert_eq!(cat.ver.as_deref(), Some("v2.0.0"));
        assert_eq!(cat.installed, Some(true)); // behaviour untouched
        assert_eq!(cat.locked, None); // never locked by CDN
        assert_eq!(cat.no_config, None); // Configure button preserved
    }

    #[test]
    fn rejects_xss_iconcls_on_known_id() {
        let mut inline = four();
        let cdn = vec![desc(
            r#"{"id":"catalog","section":"official","iconCls":"x\"><img src=y onerror=alert(1)>"}"#,
        )];
        let report = merge_official(&mut inline, &cdn);
        let cat = inline.iter().find(|a| a.id == "catalog").unwrap();
        assert_eq!(cat.icon_cls.as_deref(), Some("puzzle")); // malicious value dropped
        assert_eq!(report.dropped_icons, vec!["catalog"]); // …and said so
    }

    #[test]
    fn skips_stream_sources() {
        let mut inline = four();
        let cdn: Vec<AddonDescriptor> = serde_json::from_str(
            r#"[
          {"id":"pirate","section":"official","name":"P","transportUrl":"http://x/manifest.json"},
          {"id":"pirate2","section":"official","name":"P2","resources":["stream"]}
        ]"#,
        )
        .unwrap();
        let report = merge_official(&mut inline, &cdn);
        assert!(!inline.iter().any(|a| a.id == "pirate" || a.id == "pirate2"));
        assert_eq!(report.skipped_ids(), vec!["pirate", "pirate2"]);
        assert!(report
            .skipped
            .iter()
            .all(|s| s.reason == SkipReason::HasStream));
        assert!(!report.changed);
    }

    #[test]
    fn appends_new_curated_card_and_derives_ver() {
        let mut inline = four();
        let cdn = vec![desc(
            r#"{"id":"nebula","section":"official","name":"Nebula","version":"2.1.0","tags":["catalog"],"defaultInstalled":true}"#,
        )];
        let report = merge_official(&mut inline, &cdn);
        let n = inline.iter().find(|a| a.id == "nebula").unwrap();
        assert_eq!(report.added, vec!["nebula"]);
        assert_eq!(n.ver.as_deref(), Some("v2.1.0"));
        assert_eq!(n.installed, Some(true));
        assert_eq!(n.transport_url, None);
    }

    #[test]
    fn skips_community_and_idless() {
        let mut inline = four();
        let cdn: Vec<AddonDescriptor> = serde_json::from_str(
            r#"[
          {"id":"","section":"official","name":"x"},
          {"id":"c1","section":"community","name":"community card"}
        ]"#,
        )
        .unwrap();
        let report = merge_official(&mut inline, &cdn);
        assert_eq!(inline.len(), 4);
        assert!(!report.changed);
        assert_eq!(report.skipped_ids(), vec!["<empty-id>", "c1"]);
        assert_eq!(report.skipped[0].reason, SkipReason::NoId);
        assert_eq!(report.skipped[1].reason, SkipReason::NotOfficial);
    }

    /// A new card whose icon is hostile still lands — with the safe default, and
    /// with the drop recorded. Rejecting the whole card would let a CDN typo
    /// remove an add-on, which is exactly the power the merge is meant to deny it.
    #[test]
    fn new_card_with_unsafe_icon_falls_back_and_reports() {
        let mut inline = four();
        let cdn = vec![desc(
            r#"{"id":"nebula","section":"official","name":"Nebula","iconCls":"<script>"}"#,
        )];
        let report = merge_official(&mut inline, &cdn);
        let n = inline.iter().find(|a| a.id == "nebula").unwrap();
        assert_eq!(n.icon_cls.as_deref(), Some("puzzle"));
        assert_eq!(report.dropped_icons, vec!["nebula"]);
    }

    /// The guard that could not be reached. A record with NO `id` key — not an
    /// empty one, an absent one — used to fail the payload deserialize, so every
    /// other record in the file was lost and `SkipReason::NoId` never fired once.
    #[test]
    fn a_record_with_no_id_costs_that_record_and_nothing_else() {
        let (payload, warnings) = parse_payload(
            r#"{"schema":1,"addons":[
                 {"section":"official","name":"idless"},
                 {"id":"nebula","section":"official","name":"Nebula","version":"2.1.0"}]}"#,
        )
        .unwrap();
        assert!(
            warnings.is_empty(),
            "a missing id is a merge guard, not a parse failure"
        );

        let mut inline = four();
        let report = merge_official(&mut inline, &payload.addons);
        assert_eq!(report.added, vec!["nebula"], "the good record still merges");
        assert_eq!(report.skipped_ids(), vec!["<empty-id>"]);
        assert_eq!(report.skipped[0].reason, SkipReason::NoId);
    }

    /// One unreadable record costs that record. Before, its `types: 42` failed the
    /// whole file and every client on the CDN fell back to its inline four.
    #[test]
    fn parse_payload_drops_only_the_unreadable_record() {
        let (payload, warnings) = parse_payload(
            r#"{"schema":1,"version":"6","addons":[
                 {"id":"good","section":"official"},
                 {"id":"bad","section":"official","types":42},
                 42]}"#,
        )
        .unwrap();
        let ids: Vec<&str> = payload.addons.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["good"]);
        assert_eq!(payload.version, "6", "the document is still read normally");
        let seen: Vec<(&str, &str)> = warnings
            .iter()
            .map(|w| (w.code.as_str(), w.subject.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("dropped.bad_item", "addons[1]"),
                ("dropped.bad_item", "addons[2]"),
            ]
        );
    }

    /// Document-level failures stay failures: the caller's answer to these is "keep
    /// the inline list", and it cannot be if the parse quietly succeeds with none.
    #[test]
    fn parse_payload_still_refuses_a_non_document() {
        for doc in [
            "}{ not json",
            r#"[{"id":"nebula","section":"official"}]"#,
            r#"{"addons":[]}"#,
            r#"{"schema":"one","addons":[]}"#,
        ] {
            assert!(parse_payload(doc).is_err(), "{doc}");
        }
    }

    /// The report has to survive the trip to the boundary intact — one warning
    /// per rejected record, carrying the id as its subject.
    #[test]
    fn report_converts_to_boundary_warnings() {
        let mut inline = four();
        let cdn: Vec<AddonDescriptor> = serde_json::from_str(
            r#"[
          {"id":"","section":"official"},
          {"id":"c1","section":"community"},
          {"id":"pirate","section":"official","resources":["stream"]},
          {"id":"catalog","section":"official","iconCls":"<script>"}
        ]"#,
        )
        .unwrap();
        let ws = merge_official(&mut inline, &cdn).warnings();
        let codes: Vec<&str> = ws.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "skipped.no_id",
                "skipped.not_official",
                "skipped.has_stream",
                "dropped.unsafe_icon"
            ]
        );
        assert_eq!(ws[2].subject, "pirate");
        assert_eq!(ws[3].subject, "catalog");
    }
}
