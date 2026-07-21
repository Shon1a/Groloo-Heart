#![forbid(unsafe_code)]
// A core that panics is a core that dies silently. `wasm32-unknown-unknown` has
// no unwinding on stable Rust, so a panic there permanently traps the instance
// and no amount of `catch_unwind` in the shell can recover it — the only real
// defence is not to have panicking paths, and the only way to keep that true as
// the crate grows is to make them a build failure. Every rule below is therefore
// about the *shape* of the code, not its correctness; correctness is what the
// tests are for.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
// Tests are the one place where panicking IS the reporting mechanism. This is the
// only sanctioned escape hatch; a bare `#[allow]` anywhere outside `cfg(test)` is
// a review failure.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

//! # groloo-core
//!
//! The domain logic every GROLOO shell shares — web (WASM), webOS, Android TV,
//! and the node server. Pure and I/O-free: bytes in, bytes out. Network, storage,
//! clocks and rendering belong to the shell.
//!
//! ## What this is, and what it replaces
//!
//! This crate is the successor to [`Stredio-Heart`](https://github.com/Shon1a/Stredio-Heart).
//! Heart's ~600 lines of real domain logic are correct and are carried across
//! unchanged; what does not come with them is the Elm runtime that wrapped them.
//! Heart exposed `Model`/`Msg`/`update`/`Effect` and three *disconnected*
//! reducers, but every shell already drove it as a stateless JSON transform —
//! hydrating an entire library, applying one message, and snapshotting it back
//! out on every playback tick. The abstraction was paying rent nobody collected,
//! and it was also the reason the binding layer cost 304 hand-written lines per
//! platform.
//!
//! So the boundary here is deliberately the dumbest thing that works:
//!
//! > **free functions, JSON string in, JSON string out.**
//!
//! No state held in Rust, no handles, no lifetimes, no `Msg` enum, nothing to
//! marshal but `&str` and `String`. That shape survives wasm-bindgen, UniFFI and
//! JNI without ceremony, and it is chosen *because* it is how the shell already
//! uses the core, not in spite of it.
//!
//! ## Crate layout
//!
//! - [`api`] — **the boundary itself**: the 22 free functions, `&str` in, `String`
//!   out. It lives here, in the rlib, and not in a binding crate. That is not
//!   tidiness: `groloo-core-ffi` (Phase 5, Android) cannot depend on a `cdylib`
//!   built for `wasm32` to reach the functions it exists to expose, so a boundary
//!   housed in the wasm crate is a boundary that survives JNI only by being
//!   written a second time — which is the 304-lines-per-platform cost this whole
//!   shape was chosen to avoid. Every binding crate is now forwards only.
//! - [`types`] — the shared data model (descriptors, manifests, collections,
//!   library items, progress, My List). Vendored from Heart with the Phase 1
//!   corrections applied.
//! - [`envelope`] — the `{ok, core, data, warnings, error}` wrapper that replaces
//!   Heart's silent `catch { return null }` degradation. Degradation kept,
//!   silence removed.
//! - [`salvage`] — the other half of that: row-level leniency that reports what it
//!   dropped or rewrote, because a `deserialize_with` hook cannot.
//! - [`collection`] — merging a CDN-served official collection over the inline
//!   defaults, with the neutral-conduit and icon allow-list guards intact.
//! - [`state`] — per-account install state and its last-write-wins
//!   reconciliation. Vendored from Heart, behaviour-frozen.
//! - [`library`] — the history / progress / My List CRDT, its caps and its
//!   tombstone TTL. One implementation of an algorithm that currently exists in
//!   four hand-maintained copies.
//! - [`addon`] — manifest URL normalisation, base-URL resolution, validation and
//!   capability checks. Vendored from Heart, where it shipped with no binding and
//!   was therefore reachable from nothing.
//! - [`rows`] — the seven surviving lines of Heart's `catalog.rs`: which home rows
//!   render. The table is an argument, not a compiled-in const.
//! - [`media`] — `MediaRef` and the two media-key formats (`id`, `id:S#E#`,
//!   `id:1:4`). The cross-platform naming contract: `07-build-plan.md` §2.7 puts
//!   media-key parsing in the core precisely so the Android Engage publisher calls
//!   it instead of re-typing the season/episode split in Kotlin.
//! - [`stream`] — the Stremio `stream`/`subtitles` wire types **and** the mapping
//!   that turns them into what the UI renders. Hand-ported rather than vendored
//!   from stremio-core; the module doc carries the assessment and the three
//!   deliberate divergences.
//! - [`catalog`] — an add-on catalog's `meta` entries → poster cards. Mostly a
//!   translation of what four JavaScript expressions *mean* rather than what they
//!   say (`parseFloat`, `toFixed`, `slice` on UTF-16, `|| undefined`).
//! - [`rank`] — capability-aware stream ranking: the shell probes the device, the
//!   core decides. One rule for LG, Android and the browser, and the fix for a
//!   player that calls a 4K HEVC source unavailable on the hardware built to
//!   decode it.
//!
//! Public behaviour is tested *through* [`api`], against the exact strings a shell
//! receives, because that is the only surface a shell can actually observe; the
//! module tests underneath pin the rules that boundary is made of.
//!
//! The boundary is 22 functions: the original 17 plus `stream_parse`,
//! `catalog_metas`, `addon_resource_path`, `order_langs` and `rank_streams`. The
//! `subtitle_*` namespace is still reserved (`srtToVtt` has no port yet).
//!
//! **Adding a boundary function touches four files and only one of them is here.**
//! `crates/groloo-core/src/api.rs` and `tests/boundary_is_reachable.rs` are in
//! this crate; `scripts/build-wasm.mjs`'s `EXPORTS` list, the forwarding shim in
//! `crates/groloo-core-wasm`, and `Stredio-Web/src/lib/heart.ts`'s `CoreExports`
//! are not. Until those three catch up, a new function is reachable from Rust and
//! from `cargo test` but not from a browser — which is why the wasm build still
//! passes: it checks that the names it knows about exist, not that it knows about
//! every name.

pub mod addon;
pub mod api;
pub mod catalog;
pub mod collection;
pub mod envelope;
pub mod library;
pub mod media;
pub mod rank;
pub mod rows;
pub mod salvage;
pub mod state;
pub mod stream;
pub mod types;

// Convenience re-exports — a shell binding should never need to know which module
// a type came from, and moving one later should not be a breaking change.
pub use addon::{
    addon_base_url, manifest_has_resource, manifest_provides_stream, normalize_manifest_url,
    validate_manifest,
};
pub use catalog::{map_catalog_meta, map_catalog_metas, CatalogItem, CatalogMeta, CatalogResponse};
pub use collection::{
    has_stream, merge_official, parse_payload, safe_icon, MergeReport, SkipReason, Skipped,
};
pub use envelope::{CoreError, Envelope, ErrorCode, Warning, WarningCode};
pub use library::{
    merge_history, merge_libraries, merge_mylist, merge_progress, merge_tombstones, parse_library,
    prune_tombstones, union_tombstones, Library, LibraryMerge, LibraryMergeReport, Tombstones,
    HISTORY_CAP, MEDIA_KEY_SEPARATOR, MYLIST_CAP, PROGRESS_CAP, RESUME_DONE_FRACTION,
    RESUME_MIN_FRACTION, TOMB_TTL_MS,
};
pub use media::{addon_resource_path, encode_uri_component, MediaKind, MediaRef, MediaSource};
pub use rank::{
    classify, order_langs, quality_rank, rank_streams, Capabilities, RankCandidate, RankSummary,
    RankedStream, Ranking, StreamClass,
};
pub use rows::{visible_rows, Gating, RowDecl};
pub use state::{apply_install_map, install_map, reconcile, InstallMap, SyncDecision};
pub use stream::{
    detect_quality, detect_resolution, extract_size, map_addon_stream, map_addon_streams,
    parse_stream_langs, AddonStream, Stream, StreamBehaviorHints, StreamProxyHeaders,
    StreamSourceKind, StreamsResponse, Subtitle,
};
pub use types::{
    AddonCatalogDecl, AddonCollection, AddonDescriptor, AddonManifest, CollectionManifest,
    CollectionRef, Flags, LibraryItem, MetaItem, MyListItem, Progress, Resource, Section,
    COLLECTION_SCHEMA,
};

/// This build's identity: the crate version plus the git revision it was compiled
/// from, e.g. `0.2.0+g1a2b3c4`. `+gunknown` when the source tree had no revision
/// to report (a tarball, or a repo with no commit yet).
///
/// It is a `const` built by `concat!` rather than a function that formats at call
/// time so there is exactly one place a version can come from and no path that
/// forgets to stamp it. Every envelope carries it by construction.
pub const CORE_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+g",
    env!("GROLOO_CORE_GIT_SHA")
);

/// The bootstrap probe — deliberately NOT enveloped.
///
/// The shell calls this immediately after instantiating the module, before it has
/// any reason to trust the envelope format, and compares the answer to the
/// pinned vendored folder name it loaded from. A mismatch means Stredio-Web is
/// running a core that is not the core it thinks it is, which is otherwise
/// completely invisible. Being trustworthy before anything else is, this function
/// must never fail and must never depend on anything that can.
pub fn core_version() -> String {
    CORE_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the shape the shell parses, not the value — the sha changes on every
    /// commit, but `<semver>+g<rev>` is a contract `heart.ts` compares against.
    #[test]
    fn core_version_is_semver_plus_git() {
        let v = core_version();
        let (semver, rev) = v.split_once("+g").expect("version must carry a +g suffix");
        assert_eq!(semver, env!("CARGO_PKG_VERSION"));
        assert!(!rev.is_empty(), "revision must never be blank");
        assert!(!v.contains(char::is_whitespace), "must be a single token");
    }
}
