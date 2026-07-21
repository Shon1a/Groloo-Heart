//! The numbers the JavaScript differential harness writes down by hand.
//!
//! `tests/differential/` compares this crate against the vendored
//! `0.1.0-5ecaa78` artifact by driving both through node. Its fixtures state the
//! caps, the TTL and the resume window as LITERALS — `bigHistory(70)` to overflow
//! a cap of 60, `1000 + TTL + 1` to age out a tombstone — because a differential
//! fixture that imported the constant from the implementation it is testing would
//! move whenever the implementation moved and would therefore never catch it
//! moving. That is the right call, and it buys a specific hole: raise
//! `HISTORY_CAP` to 100 and every one of those fixtures still passes, having
//! quietly stopped testing the boundary it names.
//!
//! This file is the other half of that trade. It is the one place where the
//! literals and the constants are compared, so a change to either fails here
//! rather than silently hollowing out the harness.
//!
//! Each value below is also 0.1.0's value — Heart's `library.rs:14-18` — with two
//! deliberate exceptions, marked as such. That is what makes them safe for the
//! harness to assume on BOTH sides of the comparison.

use groloo_core::library::{
    HISTORY_CAP, MEDIA_KEY_SEPARATOR, MYLIST_CAP, PROGRESS_CAP, RESUME_DONE_FRACTION,
    RESUME_MIN_FRACTION, TOMB_TTL_MS,
};
use groloo_core::types::COLLECTION_SCHEMA;

/// Frozen from `Stredio-Heart@0.1.0 library.rs:14-16`. The harness overflows each
/// of these by hand (`bigHistory(70)`, `bigProgress(250)`) and asserts the survivor
/// count, so a change here turns those fixtures into no-ops rather than failures.
#[test]
fn caps_are_the_ones_the_harness_overflows() {
    assert_eq!(HISTORY_CAP, 60, "differential fixtures overflow 60 by hand");
    assert_eq!(
        PROGRESS_CAP, 240,
        "differential fixtures overflow 240 by hand"
    );
    // New in this crate — 0.1.0's Library had no My List, so there is nothing on
    // the old side to compare against. It is pinned anyway because
    // `stores/library.ts` has been enforcing the same 200 in TypeScript, and the
    // whole point of publishing it through `core_constants` is that the shell's
    // copy becomes derived rather than parallel.
    assert_eq!(MYLIST_CAP, 200);
}

/// 30 days in ms. The harness straddles this boundary by exactly one millisecond
/// in both directions (`now = 1000 + TTL` survives, `+ 1` expires), which only
/// tests anything if the number it adds is the number the core subtracts.
#[test]
fn tombstone_ttl_is_thirty_days() {
    assert_eq!(TOMB_TTL_MS, 2_592_000_000);
    assert_eq!(TOMB_TTL_MS, 30 * 24 * 3600 * 1000, "…and that is 30 days");
}

/// D5, pinned. The harness's `between-the-two-finished-thresholds` fixture uses a
/// watched fraction of 0.92 SPECIFICALLY because it falls between 0.1.0's
/// `Progress::is_finished` cutoff of 0.9 and this crate's 0.94. Move
/// `RESUME_DONE_FRACTION` to either side of 0.92 and that fixture stops
/// demonstrating the divergence while still reporting that it did.
#[test]
fn the_resume_window_still_straddles_the_fixtures_probe() {
    assert_eq!(RESUME_MIN_FRACTION, 0.01);
    assert_eq!(RESUME_DONE_FRACTION, 0.94);
    let probe = 0.92_f64;
    assert!(
        probe > 0.9 && probe < RESUME_DONE_FRACTION,
        "0.92 must sit strictly between Heart's retired 0.9 and this crate's \
         {RESUME_DONE_FRACTION}, or the D5 fixture proves nothing",
    );
}

/// D3, pinned. `library_remove` sweeps `{id}{SEP}` and the harness feeds it keys
/// spelled `tt1:S1E1`. If the separator ever stopped being `":"`, the sweep would
/// match nothing, the fixture would agree with 0.1.0, and the run would report the
/// divergence as stale — which is the right alarm, but this is the clearer one.
#[test]
fn the_media_key_separator_is_what_the_fixtures_spell() {
    assert_eq!(MEDIA_KEY_SEPARATOR, ":");
    assert!("tt1:S1E1".starts_with(&format!("tt1{MEDIA_KEY_SEPARATOR}")));
    // The near-miss the fixture exists to protect: a prefix sweep must not eat a
    // different title whose id merely starts with the same characters.
    assert!(!"tt10:S1E1".starts_with(&format!("tt1{MEDIA_KEY_SEPARATOR}")));
}

/// The gate `merge_official` applies to a payload document and
/// `official_payload_file` applies to an index. The harness has `schema: 2`
/// fixtures on both, which only exercise the gate while 2 is still unsupported.
#[test]
fn the_collection_schema_gate_is_still_one() {
    assert_eq!(COLLECTION_SCHEMA, 1);
}
