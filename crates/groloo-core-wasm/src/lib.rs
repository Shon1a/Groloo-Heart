//! The browser binding for [`groloo_core`] — and nothing else.
//!
//! Every function here is a one-line forward into [`groloo_core::api`]. That is a
//! hard rule, not a style preference: this crate compiles to a `cdylib` a
//! television loads, and anything with a *decision* in it is code that ships
//! untested by the path that matters. Heart learned this the expensive way — its
//! `wasm.rs` grew to 304 lines of hand-written bindings, one per message, and every
//! one of them would have had to be written again for JNI and again for webOS.
//!
//! Because the boundary is `&str -> String`, the entire marshalling story is
//! "wasm-bindgen copies a UTF-8 string". No `JsValue`, no `Option`, no `Result`,
//! no `serde-wasm-bindgen`, nothing whose ABI can drift. `null` is expressed
//! inside the JSON, never by the return type. No exported struct holds state, so
//! there is no handle to leak and no lifetime to marshal — that shape is the whole
//! point, and it is chosen because it is how the shell already drives the core.
//!
//! ## Why there is no logic here, and nothing to test
//!
//! The 22 boundary functions and the home-row rule live in `groloo_core::api` and
//! `groloo_core::rows`, in the domain **rlib**, because a boundary only one
//! binding can reach is not a boundary. This crate is a `cdylib` built for
//! `wasm32-unknown-unknown`, so `groloo-core-ffi` (Phase 5, Android) could not
//! depend on it to reach the very functions it exists to expose — it would have had
//! to write all 22 again, per platform, which is the cost that killed Heart's `Msg`
//! boundary and the single reason this one is shaped the way it is.
//!
//! So: **every binding crate is forwards only.** The domain crate denies
//! `unwrap`/`expect`/`panic`/indexing/overflowing arithmetic at its own root and
//! carries the tests, including `tests/boundary_is_reachable.rs`, which links it as
//! an external rlib and calls all 22 exactly the way the FFI crate will. There is
//! deliberately no `#[deny(...)]` and no `#[cfg(test)]` module below: lints hung
//! here would police `#[wasm_bindgen]`'s own expansion, which is not ours to fix
//! and would eventually force a blanket `#[allow]` — the exact escape hatch the
//! rule is trying to deny — and a test here could only prove that a one-expression
//! forward forwards.
//!
//! ## Panic posture
//!
//! `wasm32-unknown-unknown` has no unwinding on stable Rust: `catch_unwind`
//! catches nothing and a panic traps the instance *permanently*. So the defence is
//! layered, and none of the layers is "catch it":
//!
//! 1. `groloo-core` denies `unwrap`/`expect`/`panic`/indexing/overflowing
//!    arithmetic at its crate root — a panicking path fails the build, in the crate
//!    where the code actually is.
//! 2. [`start`] installs `console_error_panic_hook`, so a panic that escapes (1)
//!    is legible in a TV devtools console instead of `RuntimeError: unreachable`.
//! 3. The shell treats any *thrown* value from a core call as terminal: flip
//!    `coreStatus.state` to `'panicked'` and stop calling this instance. A trapped
//!    module is poisoned; retrying it is the silent-death path this phase exists to
//!    close.

use wasm_bindgen::prelude::*;

/// Runs on module instantiation, before the shell calls anything.
///
/// The hook is the only thing standing between a core panic and a completely
/// opaque `RuntimeError: unreachable executed` in a living-room devtools console.
/// It cannot make the failure recoverable — nothing can, on this target — but it
/// makes it *reportable*, which is the difference between "the app silently
/// stopped syncing" and a bug report with a line number.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
}

// --- official collection ----------------------------------------------------

/// See [`groloo_core::api::official_payload_file`].
#[wasm_bindgen]
pub fn official_payload_file(index_json: &str) -> String {
    groloo_core::api::official_payload_file(index_json)
}

/// See [`groloo_core::api::merge_official`].
#[wasm_bindgen]
pub fn merge_official(inline_json: &str, payload_json: &str) -> String {
    groloo_core::api::merge_official(inline_json, payload_json)
}

// --- install state ----------------------------------------------------------

/// See [`groloo_core::api::reconcile_install_state`].
#[wasm_bindgen]
pub fn reconcile_install_state(request_json: &str) -> String {
    groloo_core::api::reconcile_install_state(request_json)
}

// --- library ----------------------------------------------------------------

/// See [`groloo_core::api::merge_library`]. `now` crosses as `f64` because
/// wasm-bindgen has no `u64` in the JS ABI; it is clamped, never trusted.
#[wasm_bindgen]
pub fn merge_library(local_json: &str, remote_json: &str, now: f64) -> String {
    groloo_core::api::merge_library(local_json, remote_json, now)
}

/// See [`groloo_core::api::library_record_watch`].
#[wasm_bindgen]
pub fn library_record_watch(library_json: &str, item_json: &str) -> String {
    groloo_core::api::library_record_watch(library_json, item_json)
}

/// See [`groloo_core::api::library_remove`].
#[wasm_bindgen]
pub fn library_remove(library_json: &str, id: &str, now: f64) -> String {
    groloo_core::api::library_remove(library_json, id, now)
}

/// See [`groloo_core::api::mylist_toggle`].
#[wasm_bindgen]
pub fn mylist_toggle(library_json: &str, item_json: &str, now: f64) -> String {
    groloo_core::api::mylist_toggle(library_json, item_json, now)
}

/// See [`groloo_core::api::continue_watching`].
#[wasm_bindgen]
pub fn continue_watching(library_json: &str, options_json: &str) -> String {
    groloo_core::api::continue_watching(library_json, options_json)
}

/// See [`groloo_core::api::resume_position`].
#[wasm_bindgen]
pub fn resume_position(library_json: &str, key: &str) -> String {
    groloo_core::api::resume_position(library_json, key)
}

// There is deliberately no `set_progress` binding. It would fire every ~5s of
// playback and copy the whole library across linear memory twice to write one
// key; the shell keeps that write. See the note in `groloo_core::api`.

// --- add-on protocol --------------------------------------------------------

/// See [`groloo_core::api::normalize_manifest_url`].
#[wasm_bindgen]
pub fn normalize_manifest_url(raw: &str) -> String {
    groloo_core::api::normalize_manifest_url(raw)
}

/// See [`groloo_core::api::addon_base_url`].
#[wasm_bindgen]
pub fn addon_base_url(manifest_url: &str) -> String {
    groloo_core::api::addon_base_url(manifest_url)
}

/// See [`groloo_core::api::validate_manifest`].
#[wasm_bindgen]
pub fn validate_manifest(manifest_json: &str) -> String {
    groloo_core::api::validate_manifest(manifest_json)
}

/// See [`groloo_core::api::manifest_has_resource`]. `typ` is `""` for "any type"
/// — an empty string rather than `null`, so the signature stays plain `&str`.
#[wasm_bindgen]
pub fn manifest_has_resource(manifest_json: &str, resource: &str, typ: &str) -> String {
    groloo_core::api::manifest_has_resource(manifest_json, resource, typ)
}

/// See [`groloo_core::api::addon_catalogs`].
#[wasm_bindgen]
pub fn addon_catalogs(records_json: &str) -> String {
    groloo_core::api::addon_catalogs(records_json)
}

/// See [`groloo_core::api::stream_parse`]. `addon_name` is the label the shell
/// wants shown as the source; the core neither knows it nor guesses it.
#[wasm_bindgen]
pub fn stream_parse(response_json: &str, addon_name: &str) -> String {
    groloo_core::api::stream_parse(response_json, addon_name)
}

/// See [`groloo_core::api::catalog_metas`].
#[wasm_bindgen]
pub fn catalog_metas(response_json: &str) -> String {
    groloo_core::api::catalog_metas(response_json)
}

/// See [`groloo_core::api::addon_resource_path`]. `media_type` is the **wire**
/// vocabulary (`movie` / `series`), not the `/api/` one (`movie` / `tv`).
#[wasm_bindgen]
pub fn addon_resource_path(resource: &str, media_type: &str, id: &str) -> String {
    groloo_core::api::addon_resource_path(resource, media_type, id)
}

/// See [`groloo_core::api::order_langs`].
#[wasm_bindgen]
pub fn order_langs(langs_json: &str) -> String {
    groloo_core::api::order_langs(langs_json)
}

// --- stream ranking ---------------------------------------------------------

/// See [`groloo_core::api::rank_streams`]. `caps_json` is the device profile the
/// shell probed; `"{}"` is the fully permissive one, and it is what a shell that
/// has not probed yet should send — an *empty* profile means "no constraint on
/// that axis", never "allow nothing".
#[wasm_bindgen]
pub fn rank_streams(streams_json: &str, caps_json: &str) -> String {
    groloo_core::api::rank_streams(streams_json, caps_json)
}

// --- home rows --------------------------------------------------------------

/// See [`groloo_core::api::visible_rows`].
#[wasm_bindgen]
pub fn visible_rows(rows_json: &str, gating_json: &str, config_json: &str) -> String {
    groloo_core::api::visible_rows(rows_json, gating_json, config_json)
}

// --- meta -------------------------------------------------------------------

/// See [`groloo_core::api::core_version`]. Returns a bare string, never an
/// envelope — it is the probe everything else is trusted relative to.
///
/// NOTE for whoever reads this expecting it to be the load-time pin: it is not,
/// any more. The shell verifies the artifact by the **sha-256 of these bytes**,
/// recorded in the `manifest.json` beside the vendored `.wasm` (see
/// `scripts/build-wasm.mjs`), because a version string cannot detect the failure
/// it was guarding against — a stale or truncated artifact reports the version it
/// was compiled with quite happily. This stays exported as human-readable
/// provenance and as the value stamped into every envelope's `core` field.
#[wasm_bindgen]
pub fn core_version() -> String {
    groloo_core::api::core_version()
}

/// See [`groloo_core::api::core_constants`].
#[wasm_bindgen]
pub fn core_constants() -> String {
    groloo_core::api::core_constants()
}
