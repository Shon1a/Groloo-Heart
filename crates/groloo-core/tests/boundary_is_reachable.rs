//! Proof that the boundary is reachable from an ordinary Rust dependent.
//!
//! This file exists because "the boundary shape survives JNI without ceremony" is
//! a claim, and until the Phase 5 crate exists nothing tests it. An integration
//! test links `groloo-core` exactly the way `groloo-core-ffi` will — as an
//! external rlib, from outside the crate, through `pub` paths only — so if the 22
//! functions ever drift back behind a binding crate, a `pub(crate)`, or a Cargo
//! feature, this stops compiling at the moment it happens rather than in whichever
//! month somebody starts the Android shell.
//!
//! It deliberately asserts almost nothing about behaviour. The behaviour tests
//! live next to the functions in `src/api.rs`, where they can see the private
//! helpers; what is being pinned here is *reachability and shape* — free
//! functions, `&str` in, `String` out, no handle, no lifetime, nothing to marshal.

use groloo_core::api;

/// Every enveloped function, called the way a foreign binding would call it:
/// borrowed strings in, an owned `String` out, no state carried between calls.
///
/// The array is the point. Twenty-one `String`s in one collection is only possible
/// because every signature returns the same owned type — the moment one of them
/// needs an `Option`, a `Result` or a struct, this stops compiling, and that is
/// exactly the drift a per-message hand-written binding layer is made of.
#[test]
fn every_boundary_function_is_a_free_function_returning_a_string() {
    let empty = "{}";
    let responses: Vec<String> = vec![
        api::official_payload_file(empty),
        api::merge_official("[]", empty),
        api::reconcile_install_state(empty),
        api::merge_library(empty, empty, 1_700_000_000_000.0),
        api::library_record_watch(empty, r#"{"id":"tt1","at":1}"#),
        api::library_remove(empty, "tt1", 1_700_000_000_000.0),
        api::mylist_toggle(empty, r#"{"id":"tt1"}"#, 1_700_000_000_000.0),
        api::continue_watching(empty, empty),
        api::resume_position(empty, "tt1"),
        api::normalize_manifest_url("https://a.co/x"),
        api::addon_base_url("https://a.co/x/manifest.json"),
        api::validate_manifest(empty),
        api::manifest_has_resource(empty, "catalog", ""),
        api::addon_catalogs("[]"),
        api::visible_rows("[]", empty, empty),
        api::core_constants(),
        api::stream_parse(r#"{"streams":[{"url":"https://a.co/v.mp4"}]}"#, "Add-on"),
        api::catalog_metas(r#"{"metas":[{"id":"tt1","poster":"p"}]}"#),
        api::addon_resource_path("stream", "series", "tt1:1:4"),
        api::order_langs(r#"["ru","en"]"#),
        api::rank_streams(
            r#"[{"label":"Movie 1080p","url":"https://a.co/v.mp4"}]"#,
            "{}",
        ),
    ];
    assert_eq!(
        responses.len(),
        21,
        "21 enveloped functions + core_version = the 22 the contract names"
    );

    // The envelope is the ABI, so it is checked here too: a binding author reading
    // this file should not have to go and find out what the string contains.
    for s in &responses {
        let v: serde_json::Value = serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}"));
        for k in ["ok", "core", "data", "warnings", "error"] {
            assert!(v.get(k).is_some(), "envelope is missing `{k}`: {s}");
        }
        assert_eq!(
            v["core"],
            serde_json::Value::from(groloo_core::CORE_VERSION)
        );
    }
}

/// The 22nd. Bare string, never an envelope — it is the probe every other answer
/// is trusted relative to, so it must not depend on the envelope format being
/// trustworthy yet.
#[test]
fn core_version_is_the_one_unenveloped_answer() {
    let v = api::core_version();
    assert!(!v.starts_with('{'), "must not be an envelope: {v}");
    assert_eq!(v, groloo_core::CORE_VERSION);
}

/// The typed layer a native shell may prefer to the JSON one is reachable too —
/// the same rules, without paying for a round trip through a string. A binding
/// crate uses `api`; a Rust server embedding the core can use these directly.
#[test]
fn the_typed_domain_is_reachable_beside_the_json_boundary() {
    let (local, warnings) =
        groloo_core::parse_library(r#"{"history":[{"id":"a","at":1},{"bad":true}]}"#).unwrap();
    assert_eq!(local.history.len(), 1);
    assert_eq!(
        warnings.len(),
        1,
        "the drop is reported to Rust callers too"
    );

    let merged = groloo_core::merge_libraries(&local, &groloo_core::Library::default(), 1_000);
    assert_eq!(merged.library.history.len(), 1);
}
