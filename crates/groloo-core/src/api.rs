//! THE BOUNDARY — all 22 functions, `&str` in, `String` out, nothing else.
//!
//! ## Why this file is in the domain rlib and not in a binding crate
//!
//! Because a boundary that only one binding can reach is not a boundary. The whole
//! justification for this shape — free functions, JSON string in, JSON string out
//! — is that it survives wasm-bindgen, UniFFI and JNI *without ceremony*, and a
//! module housed in `groloo-core-wasm` survives none of them: that crate is a
//! `cdylib` built for `wasm32-unknown-unknown`, so `groloo-core-ffi` (Phase 5,
//! Android) could not depend on it to reach the very functions it exists to
//! expose. It would have had to write them again, which is the 304-hand-written-
//! lines-per-platform cost that killed Heart's `Msg` boundary and the single
//! reason this one is shaped the way it is.
//!
//! So the rule is: **every binding crate is forwards only.** Each of the functions
//! below is called from exactly one one-line wrapper per platform —
//! `#[wasm_bindgen]` today, `#[uniffi::export]` next — and nothing in this module
//! or in [`crate::rows`] names a binding framework or a target. A `cdylib` a
//! television loads is the one artifact in the tree nobody can swap out, so it
//! holds nothing worth reading and nothing worth testing.
//!
//! Everything here is therefore reachable from an ordinary `cargo test` on the
//! host, which is how the tests at the bottom of this file run. "Untestable by
//! design" is never the reason something went untested.
//!
//! ## The shape, and why it is this shape
//!
//! Free functions. No exported struct holds state, so there is no handle to leak,
//! no lifetime to marshal and no `Msg` enum to hand-bind per platform — which is
//! what made Heart's `wasm.rs` 304 lines and would have made its JNI twin 304
//! more. `null` is expressed *inside* the JSON, never by the return type, so the
//! ABI is "wasm-bindgen copies a UTF-8 string" and cannot drift.
//!
//! Every function except [`core_version`] answers with an [`Envelope`]: `data` is
//! always present and is always the graceful-degradation value, so a shell that
//! destructures `data` and ignores `ok` behaves bit-identically to today, and a
//! shell that reads `ok` can finally tell "nothing to show" from "broken".
//!
//! Unknown input fields are ignored, never an error — and never round-tripped. A
//! field the core does not model is a field the core drops, so anything the shell
//! must keep stays shell-side.

use crate::envelope::{CoreError, Envelope, ErrorCode, Warning, WarningCode};
use crate::library::Library;
use crate::rows::{Gating, RowDecl};
use crate::state::{InstallMap, SyncDecision};
use crate::types::{
    AddonDescriptor, AddonManifest, CollectionManifest, LibraryItem, MyListItem, Progress,
    COLLECTION_SCHEMA,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// shared plumbing
// ---------------------------------------------------------------------------

/// Every clock crosses the boundary as `f64`, because JS has no `u64` in the
/// wasm-bindgen ABI — and it keeps crossing as `f64` on every other binding too,
/// since one signature per function is the entire point of this shape.
///
/// `Date.now()` is an integral millisecond count well inside `f64`'s exact range,
/// so the only inputs this has to defend against are the pathological ones: a
/// negative clock (a device with its date set before the epoch), `NaN` from an
/// uninitialised variable, and `±Infinity`. All of them clamp to **zero**, because
/// a boundary that trusts a bad clock is a boundary that takes the user's library
/// with it.
///
/// **`Infinity` used to reach `u64::MAX`**, and that was the whole bug. The `as`
/// cast on floats saturates rather than wraps (Rust 1.45+), which reads like a
/// defence and is the opposite of one: `merge_libraries` prunes a tombstone when
/// `now - at > TOMB_TTL_MS`, so a clock of `u64::MAX` prunes EVERY tombstone the
/// device has ever recorded. One `Infinity` — an uninitialised `Date.now()`
/// wrapper, a division by zero in a shim — and every title the user had removed on
/// any device silently comes back on the next sync, permanently, because the
/// evidence of the removal is what was deleted. `is_nan()` did not catch it, and
/// the comment claiming all three inputs were handled was simply wrong.
///
/// Zero is the honest answer: a clock the core cannot believe is not a clock, and
/// `now = 0` prunes nothing and expires nothing. It is not free — a removal
/// recorded under a broken clock gets `at: 0`, which loses to any history entry on
/// the next merge, so the removal does not stick. That is recoverable by removing
/// the title again. Wiping the tombstone map is not recoverable by anything.
///
/// The bound is `2^53`, not a guess about calendars: past `Number.MAX_SAFE_INTEGER`
/// an `f64` cannot hold consecutive integers, so whatever JS *meant* is already
/// unrecoverable and `as` would saturate to `u64::MAX` all the same. Tying the
/// limit to the ABI rather than to a plausible year is what makes it a rule
/// instead of a magic number — and it closes the whole saturation class rather
/// than the one value that was noticed.
fn clock(now: f64) -> u64 {
    /// 2^53 — the first millisecond an `f64` cannot count to exactly.
    const MAX_EXACT_MS: f64 = 9_007_199_254_740_992.0;

    if !now.is_finite() || now <= 0.0 || now >= MAX_EXACT_MS {
        0
    } else {
        // Finite, positive and exactly representable: the cast truncates toward
        // zero and has nothing to saturate against.
        now as u64
    }
}

/// `parse.input` with the serde message attached.
///
/// The message is included verbatim on purpose: "expected value at line 1 column
/// 1" is the difference between a five-minute diagnosis and an afternoon, and
/// nothing in it is user data — serde reports positions and expected types, not
/// contents.
fn parse_input(e: &serde_json::Error) -> CoreError {
    CoreError::new(ErrorCode::ParseInput, e.to_string())
}

/// One `truncated.*` warning, or none when nothing was dropped.
///
/// Truncation used to be wordless: a user syncing a 90-entry history onto a
/// device saw 60 and had no way to learn the other 30 were not lost, merely
/// capped. Reporting the count is what makes the cap a policy rather than a bug.
fn truncated(code: WarningCode, subject: &str, dropped: usize) -> Option<Warning> {
    if dropped == 0 {
        return None;
    }
    Some(Warning::new(
        code,
        subject,
        format!("dropped {dropped} record(s) over the cap"),
    ))
}

/// Re-scope a document's row warnings to the side of a merge they came from.
///
/// [`merge_library`] reads two documents and both of them number their rows from
/// zero, so an unqualified `history[3]` names two different records and identifies
/// neither. The subject becomes `local.history[3]`, which is the only form a
/// caller can act on — it is the document they would have to go and look at.
fn scoped(side: &str, warnings: Vec<Warning>) -> Vec<Warning> {
    warnings
        .into_iter()
        .map(|mut w| {
            w.subject = format!("{side}.{}", w.subject);
            w
        })
        .collect()
}

/// One `pruned.tombstone` warning per id whose removal aged out.
fn pruned(ids: &[String]) -> Vec<Warning> {
    ids.iter()
        .map(|id| {
            Warning::new(
                WarningCode::PrunedTombstone,
                id.clone(),
                "tombstone older than the 30-day TTL and forgotten",
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1-2. official collection
// ---------------------------------------------------------------------------

/// **#1** — which payload file the official collection lives in.
///
/// `data`: `string | null` (e.g. `"addons.json"`).
///
/// Replaces `official.ts:62-64` in full: `rt.load_official()`,
/// `official_manifest_fetched()`, and the `parse<Array<{FetchOfficialPayload}>>`
/// walk over Heart's *effect array* — three calls plus an effect parser whose
/// only purpose was to read one filename out of a data structure invented to
/// describe performing I/O the shell was already performing itself.
///
/// The schema gate is checked here rather than deferred to
/// [`CollectionManifest::official`], which folds "wrong schema" and "no official
/// collection" into the same `None`. They are different problems with different
/// fixes and the shell deserves to be told which one it has.
pub fn official_payload_file(index_json: &str) -> String {
    let manifest: CollectionManifest = match serde_json::from_str(index_json) {
        Ok(m) => m,
        Err(e) => return Envelope::failed(None::<String>, parse_input(&e)).into_json(),
    };
    if manifest.schema != COLLECTION_SCHEMA {
        return Envelope::failed(
            None::<String>,
            CoreError::new(
                ErrorCode::SchemaUnsupported,
                format!(
                    "collection schema {} is not {COLLECTION_SCHEMA}",
                    manifest.schema
                ),
            ),
        )
        .into_json();
    }
    match manifest.official() {
        Some(c) => Envelope::ok(Some(c.file.clone())).into_json(),
        None => Envelope::failed(
            None::<String>,
            CoreError::new(
                ErrorCode::NotFound,
                "manifest declares no official collection with a payload file",
            ),
        )
        .into_json(),
    }
}

/// **#2** — merge the CDN-served official collection over the inline defaults.
///
/// `inline_json` is `AddonDescriptor[]` (the four cards `official.ts` ships);
/// `payload_json` is the whole `AddonCollection` **document** `{schema, version,
/// addons}`, not a bare array, so the schema-1 gate lives inside the merge. That
/// gate is the one thing Heart's `Msg::OfficialPayloadFetched` arm did which its
/// `ffi::merge_official_json` did not, and splitting them is how a shell ends up
/// with one guarded path and one unguarded one.
///
/// Which is exactly what happened: `official.ts:74-80`'s `loadViaJs()` fallback
/// hands raw CDN descriptors straight to the UI, so a record carrying a
/// `transportUrl` is rejected on the Rust path and accepted on the JS one. There
/// is no version of that which is not a hole. Deleting `loadViaJs` closes it, and
/// this function is what makes deleting it possible.
///
/// On any failure `data` is the inline list **unchanged** — never `[]`. A CDN
/// outage must cost the user nothing.
///
/// `MergeReport.changed` is deliberately not surfaced: its only consumer was
/// Heart's repaint suppression, and React does that itself.
pub fn merge_official(inline_json: &str, payload_json: &str) -> String {
    let mut inline: Vec<AddonDescriptor> = match serde_json::from_str(inline_json) {
        Ok(v) => v,
        // Nothing to degrade *to*: the fallback set is the thing that failed to
        // parse. `[]` is the only honest answer, and `ok:false` says so.
        Err(e) => {
            return Envelope::failed(Vec::<AddonDescriptor>::new(), parse_input(&e)).into_json()
        }
    };

    // Row-level, so one malformed CDN record costs that record rather than the
    // whole file — and says which one. See `collection::parse_payload`.
    let (payload, mut warnings) = match crate::collection::parse_payload(payload_json) {
        Ok(p) => p,
        Err(e) => return Envelope::failed(inline, parse_input(&e)).into_json(),
    };
    if payload.schema != COLLECTION_SCHEMA {
        return Envelope::failed(
            inline,
            CoreError::new(
                ErrorCode::SchemaUnsupported,
                format!(
                    "collection schema {} is not {COLLECTION_SCHEMA}",
                    payload.schema
                ),
            ),
        )
        .into_json();
    }

    let report = crate::collection::merge_official(&mut inline, &payload.addons);
    warnings.extend(report.warnings());
    Envelope::ok(inline).with_warnings(warnings).into_json()
}

// ---------------------------------------------------------------------------
// 3. install state
// ---------------------------------------------------------------------------

/// One side of the install-state comparison: the map and when it was written.
#[derive(Debug, Clone, Default, Deserialize)]
struct StateSnapshot {
    #[serde(default)]
    map: InstallMap,
    #[serde(default)]
    at: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReconcileRequest {
    #[serde(default)]
    addons: Vec<AddonDescriptor>,
    #[serde(default)]
    local: StateSnapshot,
    #[serde(default)]
    remote: Option<StateSnapshot>,
    #[serde(default)]
    owner_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReconcileResponse {
    decision: &'static str,
    addons: Vec<AddonDescriptor>,
    map: InstallMap,
    at: u64,
}

/// **#3** — decide whose install state wins, and hand back everything that
/// follows from that decision.
///
/// This one replaces nothing in the shell today, and saying so plainly matters
/// more than the function does. `/api/addon-state` exists server-side and has
/// **zero clients** — there is not one occurrence of `addon-state` under
/// `Stredio-Web/src`. Install state is `groloo.homeconfig` in localStorage,
/// device-local, never synced. This is the net-new piece that would wire it, and
/// it is specified now because a shared living-room TV is precisely the device
/// `ownerChanged` was written for.
///
/// It subsumes Heart's `install_map` + `apply_install_map` + `reconcile` into one
/// call because no caller ever wants one without the others, and it absorbs
/// `Msg::InstallStatePulled` / `Msg::LocalStateLoaded` along with them.
///
/// The returned `map` is **recomputed** from the descriptors after the winning
/// map is overlaid, rather than echoed. That is what folds `install_map` in: the
/// result is normalised (locked ids excluded, every known id present), so it is
/// simultaneously the thing to persist and the thing to push, and the two cannot
/// drift apart.
///
/// Ship the function and its tests. **Do not ship a UI for it in this phase.**
pub fn reconcile_install_state(request_json: &str) -> String {
    let req: ReconcileRequest = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => {
            // Best effort: if the document is JSON at all, echo the descriptors we
            // can still read so the caller's list survives its own bad request.
            let mut warnings = Vec::new();
            let addons = salvage_addons(request_json, &mut warnings);
            return Envelope::failed(
                ReconcileResponse {
                    decision: "noop",
                    addons,
                    map: InstallMap::new(),
                    at: 0,
                },
                parse_input(&e),
            )
            .with_warnings(warnings)
            .into_json();
        }
    };

    let remote_at = req.remote.as_ref().map(|r| r.at).unwrap_or(0);
    let decision = crate::state::reconcile(
        req.local.at,
        req.remote.is_some(),
        remote_at,
        req.owner_changed,
    );

    let (winning_map, at) = match decision {
        SyncDecision::AdoptRemote => match &req.remote {
            Some(r) => (r.map.clone(), r.at),
            None => (req.local.map.clone(), req.local.at),
        },
        SyncDecision::UploadLocal | SyncDecision::Noop => (req.local.map.clone(), req.local.at),
    };

    let mut addons = req.addons;
    crate::state::apply_install_map(&mut addons, &winning_map);
    let map = crate::state::install_map(&addons);

    Envelope::ok(ReconcileResponse {
        decision: match decision {
            SyncDecision::AdoptRemote => "adoptRemote",
            SyncDecision::UploadLocal => "uploadLocal",
            SyncDecision::Noop => "noop",
        },
        addons,
        map,
        at,
    })
    .into_json()
}

/// Pull whatever descriptors are still readable out of a request that did not
/// deserialize. Used only on the failure path, where "echo the input" is the
/// contract and a typed parse is by definition unavailable.
///
/// It reports what it could not salvage, for the same reason everything else here
/// does: this is a list the caller is about to render, and "your request was
/// malformed" plus a list that is quietly two cards shorter is a worse answer than
/// either half alone.
fn salvage_addons(request_json: &str, warnings: &mut Vec<Warning>) -> Vec<AddonDescriptor> {
    let raw = serde_json::from_str::<serde_json::Value>(request_json)
        .ok()
        .and_then(|v| v.get("addons").cloned());
    // The container being unreadable is not a second failure worth naming — the
    // call has already failed and said why, and "there were no descriptors to give
    // back" is what an empty list means here.
    crate::salvage::items("addons", raw, warnings).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 4-9. library
// ---------------------------------------------------------------------------

/// **#4** — merge two whole `/api/library-state` documents.
///
/// Called on PULL and on boot. **Not** on a progress tick — see the note on
/// `set_progress`'s deliberate absence at the bottom of this module.
///
/// The failure contract is the load-bearing part, and it is not the envelope. On
/// a local document the core cannot read, `data` is the **remote** document
/// as-is; on an unreadable remote, `data` is the **local** one. It is never `{}`.
/// Heart's `hydrate()` ends in `unwrap_or_default()` (`wasm.rs:231`), so under
/// `heartLibrary.ts:46`'s hydrate → op → snapshot round trip a single history
/// entry the core could not deserialize turned the user's entire history,
/// progress and tombstone map into an empty object which the shell then wrote
/// back as truth. That is the single most damaging instance of problem 7, and an
/// envelope alone does not fix it: the fallback *value* had to change too.
///
/// (The row-level defence is [`crate::salvage`] — one unreadable record costs that
/// record, and now says so. This is the document-level backstop for when the
/// string is not JSON at all.)
///
/// ***Recorded divergence D8 — the removal survives the sync.*** 0.1.0 unions the
/// two progress maps and stops there, so a device that has not yet seen a removal
/// hands every `id:S#E#` key back on the next pull: the user deletes a series and
/// it returns, counting against `PROGRESS_CAP` again. The merge now re-applies
/// [`crate::library::sweep_removed_progress`] against the merged tombstone map,
/// under the same at-or-after rule the history entry already obeys, so a re-watch
/// *newer* than the removal still keeps its position. No wire change: the
/// tombstone that decides was already being exchanged.
///
/// ***Recorded divergence D7 — a same-id, equal-`at` collision converges.*** Both
/// list merges resolved `prev.at >= it.at` by keeping whichever record the loop
/// reached first, which is `local` — so two devices holding different content for
/// one id in the same millisecond each kept their own row forever. The row's
/// *order* was already commutative (that is D2); its *content* was not.
/// `library::content_order` settles it on canonical bytes, greater wins.
/// The old core's answer is therefore "whichever document was hydrated first",
/// which on the harness's `local`-then-`pulled` driver means local.
pub fn merge_library(local_json: &str, remote_json: &str, now: f64) -> String {
    let local = crate::library::parse_library(local_json);
    let remote = crate::library::parse_library(remote_json);

    // The `data` on a failure path is the side that DID read, and it is returned
    // as given — so its own row warnings still have to travel with it, or a
    // half-readable document degrades silently inside a call that already failed.
    let ((local, local_rows), (remote, remote_rows)) = match (local, remote) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), Ok((r, w))) => {
            return Envelope::failed(r, parse_input(&e))
                .with_warnings(scoped("remote", w))
                .into_json()
        }
        (Ok((l, w)), Err(e)) => {
            return Envelope::failed(l, parse_input(&e))
                .with_warnings(scoped("local", w))
                .into_json()
        }
        (Err(e), Err(_)) => {
            return Envelope::failed(Library::default(), parse_input(&e)).into_json()
        }
    };

    let merged = crate::library::merge_libraries(&local, &remote, clock(now));
    let r = &merged.report;
    let mut warnings = scoped("local", local_rows);
    warnings.extend(scoped("remote", remote_rows));
    warnings.extend(pruned(&r.pruned_tombstones));
    warnings.extend(truncated(
        WarningCode::TruncatedHistory,
        "history",
        r.truncated_history,
    ));
    warnings.extend(truncated(
        WarningCode::TruncatedProgress,
        "progress",
        r.truncated_progress,
    ));
    warnings.extend(truncated(
        WarningCode::TruncatedMylist,
        "mylist",
        r.truncated_mylist,
    ));

    Envelope::ok(merged.library)
        .with_warnings(warnings)
        .into_json()
}

/// **#5** — record that a title was watched or opened.
///
/// `data` is the updated [`Library`]. The item's `at` is authoritative; the core
/// does not own a clock.
///
/// On an unreadable item the library is echoed **unchanged** with `ok:false`.
/// Heart returned `"[]"` here (`wasm.rs:245`) and `heartLibrary.ts:46` then read
/// `snapshot_json()` off the un-updated runtime and handed it back as the new
/// state — the write was dropped and the return value was success-shaped, which
/// is the worst of both.
///
/// Fires once per playback start (`VideoPlayer.tsx:533`). Not hot.
pub fn library_record_watch(library_json: &str, item_json: &str) -> String {
    let (mut lib, mut warnings) = match crate::library::parse_library(library_json) {
        Ok(l) => l,
        Err(e) => return Envelope::failed(Library::default(), parse_input(&e)).into_json(),
    };
    let item: LibraryItem = match serde_json::from_str(item_json) {
        Ok(i) => i,
        Err(e) => {
            return Envelope::failed(lib, CoreError::new(ErrorCode::ParseField, e.to_string()))
                .with_warnings(warnings)
                .into_json()
        }
    };

    // `record_watch` caps internally and swallows the count, so derive it: an
    // upsert replaces, a new id grows the list by one, and anything the cap ate
    // is the difference. Saturating throughout — this crate denies overflowing
    // arithmetic and a length subtraction is exactly where one would hide.
    let replaced = lib.history.iter().any(|it| it.id() == item.id());
    let before = lib.history.len();
    lib.record_watch(item);
    let projected = if replaced {
        before
    } else {
        before.saturating_add(1)
    };
    let dropped = projected.saturating_sub(lib.history.len());
    warnings.extend(truncated(WarningCode::TruncatedHistory, "history", dropped));

    Envelope::ok(lib).with_warnings(warnings).into_json()
}

/// **#6** — remove a title from the library.
///
/// Tombstones `id`, drops its history entry, and drops the resume position for
/// the bare key **and every `id:` prefixed episode key**.
///
/// ***Recorded divergence D3.*** The shell keys progress by media key
/// (`player.ts:16` — `id` for a film, `${id}:S#E#` for an episode) while Heart's
/// `LibMsg::Remove` did `progress.remove(&id)` and nothing else. Removing a
/// series from Continue Watching therefore left every episode's resume position
/// behind forever, counting against `PROGRESS_CAP` and resurrecting the moment
/// the title was re-added. The prefix sweep is the fix, and it is precisely why
/// the media-key format has to be a core concept rather than a shell one.
pub fn library_remove(library_json: &str, id: &str, now: f64) -> String {
    let (mut lib, warnings) = match crate::library::parse_library(library_json) {
        Ok(l) => l,
        Err(e) => return Envelope::failed(Library::default(), parse_input(&e)).into_json(),
    };
    if id.is_empty() {
        // An empty id would tombstone the empty string and sweep every key
        // beginning `":"`. Refusing is cheaper than explaining.
        return Envelope::failed(
            lib,
            CoreError::new(ErrorCode::ParseField, "id must not be empty"),
        )
        .with_warnings(warnings)
        .into_json();
    }
    lib.remove(id, clock(now));
    Envelope::ok(lib).with_warnings(warnings).into_json()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MyListToggleResponse {
    library: Library,
    in_list: bool,
}

/// **#7** — add or remove a title from My List.
///
/// `data` is `{ library, inList }`. `inList` is the **new** membership, because
/// `stores/library.ts:103-118`'s `toggle` returns `!has` and its call sites read
/// it — keeping the return value means the call site does not have to change
/// shape to adopt the core.
///
/// Present → removed and tombstoned. Absent → prepended with `at: now` and any
/// prior tombstone cleared, so re-adding on one device beats a stale removal on
/// another. Capped at 200.
pub fn mylist_toggle(library_json: &str, item_json: &str, now: f64) -> String {
    let (mut lib, warnings) = match crate::library::parse_library(library_json) {
        Ok(l) => l,
        Err(e) => {
            return Envelope::failed(
                MyListToggleResponse {
                    library: Library::default(),
                    in_list: false,
                },
                parse_input(&e),
            )
            .into_json()
        }
    };
    let item: MyListItem = match serde_json::from_str(item_json) {
        Ok(i) => i,
        Err(e) => {
            // No id means no membership question to answer; the library is
            // untouched and `inList` reports the only truthful thing available.
            return Envelope::failed(
                MyListToggleResponse {
                    library: lib,
                    in_list: false,
                },
                CoreError::new(ErrorCode::ParseField, e.to_string()),
            )
            .with_warnings(warnings)
            .into_json();
        }
    };

    let in_list = lib.mylist_toggle(item, clock(now));
    Envelope::ok(MyListToggleResponse {
        library: lib,
        in_list,
    })
    .with_warnings(warnings)
    .into_json()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinueOptions {
    #[serde(default)]
    hide_finished: bool,
}

/// One Continue Watching rail entry, borrowed from the library it came out of.
#[derive(Debug, Serialize)]
struct ContinueEntry<'a> {
    item: &'a LibraryItem,
    key: &'a str,
    fraction: f64,
    resume: Option<Progress>,
}

/// **#8** — the Continue Watching rail, derived once.
///
/// `options_json` is `{ "hideFinished": bool }`, **default false**. That default
/// is the whole reason this function is adoptable: today's rail maps every
/// history entry regardless of completion (`ContinueRow.tsx:38`), while Heart's
/// `Library::continue_watching()` filters at 0.9 unconditionally and is therefore
/// unusable by this shell — which is exactly why `heartLibrary.ts:10-12`
/// documents that the rail is still derived in the store. Shipping the flag
/// default-false makes this a drop-in; flipping it is a separate, reviewable UI
/// decision.
///
/// `key` resolves `item.key || item.id` — the rule `ContinueRow.tsx:41`
/// open-codes. `fraction` is the raw watched fraction clamped to `[0,1]`, so a
/// finished title still reports `1.0`; `resume` is the *offer*, and is `null`
/// outside the `[0.01, 0.94]` window. Splitting the two is deliberate: the
/// progress bar and the "resume or restart?" question are different questions.
///
/// An unreadable options document does not fail the call — the rail is computed
/// with defaults and `ok:false` explains why the flag was ignored.
pub fn continue_watching(library_json: &str, options_json: &str) -> String {
    let (lib, warnings) = match crate::library::parse_library(library_json) {
        Ok(l) => l,
        Err(e) => {
            return Envelope::failed(Vec::<ContinueEntry>::new(), parse_input(&e)).into_json()
        }
    };

    let mut options_error = None;
    let options = if options_json.trim().is_empty() {
        ContinueOptions::default()
    } else {
        match serde_json::from_str(options_json) {
            Ok(o) => o,
            Err(e) => {
                options_error = Some(parse_input(&e));
                ContinueOptions::default()
            }
        }
    };

    let entries: Vec<ContinueEntry> = lib
        .continue_watching(options.hide_finished)
        .into_iter()
        .map(|item| {
            let key = item.media_key();
            ContinueEntry {
                item,
                key,
                fraction: lib.resume(key).map(|p| p.fraction()).unwrap_or(0.0),
                resume: lib.resumable(key),
            }
        })
        .collect();

    match options_error {
        Some(e) => Envelope::failed(entries, e)
            .with_warnings(warnings)
            .into_json(),
        None => Envelope::ok(entries).with_warnings(warnings).into_json(),
    }
}

/// **#9** — the resume offer for one media key.
///
/// `data` is `Progress | null`. Null when there is no record, when `dur <= 0`,
/// when the fraction is below 0.01 ("you just started"), or above 0.94 ("you
/// finished it").
///
/// ***Recorded divergence D5.*** The shell used `PROGRESS_DONE = 0.94`
/// (`history.ts:26`) and Heart's `Progress::is_finished` used 0.9
/// (`types.rs:273`). 0.94 wins: it is the number users actually experience and
/// the 0.9 had no consumer. `is_finished` is deleted and the constant now lives
/// exactly once, reachable through [`core_constants`].
pub fn resume_position(library_json: &str, key: &str) -> String {
    let (lib, warnings) = match crate::library::parse_library(library_json) {
        Ok(l) => l,
        Err(e) => return Envelope::failed(None::<Progress>, parse_input(&e)).into_json(),
    };
    // A `null` answer with a `dropped.bad_item` beside it is a different thing
    // from a `null` answer alone: the first says "the record for this key was
    // unreadable", the second says "you have not watched it".
    Envelope::ok(lib.resumable(key))
        .with_warnings(warnings)
        .into_json()
}

// *** DELIBERATELY NOT A FUNCTION: set_progress. ***
//
// `heartLibrary.ts:54` → `history.ts:134` fires on every ~5s playback tick
// (`VideoPlayer.tsx:520-525`), and each call would serialise up to 60 history
// entries, 240 progress records and 200 My List entries THROUGH LINEAR MEMORY
// TWICE — hydrate in, snapshot out — to write one key. That is problem 4 in its
// purest form, and widening `Library` to five fields makes it strictly worse.
//
// The shell keeps the single-key write: `map[key] = {pos, dur, at, lang}` plus a
// cap against `core_constants().progressCap`. Six lines of trivial JS with no
// rule worth compiling in and no divergence risk, because once the *merge* is
// single-sourced there is nothing left to diverge from.
//
// The rule itself still lives somewhere testable — `Library::set_progress` in the
// domain crate, which `merge_library` has to agree with.

// ---------------------------------------------------------------------------
// 10-14. add-on protocol
// ---------------------------------------------------------------------------

/// **#10** — normalise a user-pasted add-on URL into a canonical manifest URL.
///
/// `data` is `string | null`; `error.code` is `invalid.url`.
///
/// ***Recorded divergence D6, and the strongest single argument for exposing
/// `addon.rs` at all.*** `stores/addons.ts:203`'s twin tests `/manifest\.json$/`
/// against the **whole string including the query**, so it never matches a
/// query-bearing URL and appends a second segment:
///
/// ```text
///   "https://a.co/x/manifest.json?y=2"
///     TS   → "https://a.co/x/manifest.json?y=2/manifest.json"   broken
///     here → "https://a.co/x/manifest.json?y=2"
///   "https://a.co/addon?x=1"
///     TS   → "https://a.co/addon?x=1/manifest.json"             broken
///     here → "https://a.co/addon/manifest.json?x=1"
///   "not-an-id"
///     TS   → "https://not-an-id/manifest.json"  (a typo becomes a URL)
///     here → null                               (rejects)
/// ```
///
/// By the Stremio convention documented in that same file's header, a
/// *configured* add-on packs its credentials into the URL — so the add-ons that
/// break are exactly the credentialed ones. This divergence is only observable
/// once the TS twin is deleted; until then old-vs-new is Rust-vs-Rust and passes
/// trivially, so the differential harness must assert it against the TypeScript
/// function directly.
///
/// **Amended when the three copies were unified.** This used to reject every
/// scheme but `http(s)://` and its own `groloo://` alias; it now rewrites any
/// foreign `scheme://` to `https://`, as **both** TypeScript copies always have,
/// because `stremio://…` is the commonest form an add-on link is shared in and
/// rejecting it was a regression against shipping behaviour. The full
/// three-way ledger — which copy won on each axis and why — is in the
/// [`crate::addon`] module doc, and D6's corpus should assert against
/// `server.js:1992` as well as `addons.ts:203`.
///
/// **Amended again for U-slash, and it is worth being exact about which copy this
/// now agrees with.** On the query axis, the server. On the trailing-slash axis,
/// the *client*: `…/manifest.json/` normalises to `…/manifest.json`, where the
/// server appends a second segment and 404s. So this is no longer "the server's
/// copy, exported" — it is the better half of each, and the corpus row for
/// `https://a.co/manifest.json/` moves from an unplanned divergence against the
/// client to a **declared** one against the server:
///
/// ```text
///   "https://a.co/manifest.json/"
///     server.js:1992 → "https://a.co/manifest.json/manifest.json"   404s
///     addons.ts:203  → "https://a.co/manifest.json"
///     here           → "https://a.co/manifest.json"
/// ```
pub fn normalize_manifest_url(raw: &str) -> String {
    match crate::addon::normalize_manifest_url(raw) {
        Some(u) => Envelope::ok(Some(u)).into_json(),
        None => Envelope::failed(
            None::<String>,
            CoreError::new(ErrorCode::InvalidUrl, "not a usable URL or hostname"),
        )
        .into_json(),
    }
}

/// **#11** — the directory an add-on's resource paths resolve against.
///
/// `data` is the URL up to and including the final `/`, or `""` when there is
/// none. Cannot fail.
///
/// Replaces `addonClient.ts:45-47` —
/// `String(manifestUrl||'').replace(/[^/]*$/,'')` — which is `addon.rs:15`
/// re-derived character for character in another language. It is the specific
/// duplication plan 06 §2.5 cites as proof that "write it once" erodes without a
/// gate, and it is worth exporting a three-line function purely to close it.
pub fn addon_base_url(manifest_url: &str) -> String {
    Envelope::ok(crate::addon::addon_base_url(manifest_url)).into_json()
}

/// **#12** — validate an add-on's `manifest.json`.
///
/// `data` is the normalised manifest on success and the **best-effort parse** on
/// failure, so a caller can still show the user which add-on it was talking
/// about. `error.detail` carries `server.js:1947`'s message verbatim — all five
/// of them, `Manifest is not a JSON object` included.
///
/// Replaces `stores/addons.ts:233` — `if (!manifest || !manifest.id ||
/// !manifest.name) throw new Error('Not a valid add-on manifest')` — which checks
/// two of the four rules and gives one undifferentiated message for every
/// failure, so a user with a malformed id and a user with no `types` get
/// identical, useless feedback.
///
/// ***U-notobject, fixed here rather than in [`crate::addon::validate_manifest`].***
/// The server's FIRST rule is `!m || typeof m !== 'object'`, and a typed function
/// taking an `&AddonManifest` cannot express it — by the time it is called, serde
/// has already failed the document and the user is looking at `invalid type: null,
/// expected struct AddonManifest at line 1 column 4` where they used to read a
/// sentence. That is a real regression for one very ordinary input: a manifest URL
/// that answers `null`, or HTML, or `[]`, is what a mis-configured add-on host
/// returns, and this string goes straight onto the add-ons screen. So the shape
/// check happens on the untyped [`serde_json::Value`], before the typed parse,
/// which is the only layer that can see it — and the doc above is now true rather
/// than nearly true.
///
/// One wording difference survives, stated so nothing here claims otherwise: a
/// JSON **array** is `typeof 'object'` in JavaScript, so the server admits it to
/// its `id` rule and answers `Manifest "id" is missing or malformed`, where this
/// answers `Manifest is not a JSON object`. Same code, same verdict, better
/// sentence; reproducing the server's would mean re-implementing all four rules
/// against an untyped value to preserve a message about a manifest nobody sends.
pub fn validate_manifest(manifest_json: &str) -> String {
    let value: serde_json::Value = match serde_json::from_str(manifest_json) {
        Ok(v) => v,
        Err(e) => return Envelope::failed(AddonManifest::default(), parse_input(&e)).into_json(),
    };
    if !value.is_object() {
        return Envelope::failed(
            AddonManifest::default(),
            CoreError::new(ErrorCode::InvalidManifest, "Manifest is not a JSON object"),
        )
        .into_json();
    }
    let manifest: AddonManifest = match serde_json::from_value(value) {
        Ok(m) => m,
        Err(e) => return Envelope::failed(AddonManifest::default(), parse_input(&e)).into_json(),
    };
    match crate::addon::validate_manifest(&manifest) {
        Ok(()) => Envelope::ok(manifest).into_json(),
        Err(msg) => {
            Envelope::failed(manifest, CoreError::new(ErrorCode::InvalidManifest, msg)).into_json()
        }
    }
}

/// **#13** — does this manifest advertise `resource`, optionally for `typ`?
///
/// `typ` is `""` for "any type" — an empty string rather than `null`, so the
/// wasm-bindgen signature stays plain `&str` and there is no `Option<String>` to
/// marshal. `api` is where that maps back to `None`; the domain function keeps
/// its honest `Option<&str>`.
///
/// Replaces `addonClient.ts:49-53`, consumed at `:116` (stream collection) and
/// `:150` (catalog enumeration). Short (`"stream"`) and full
/// (`{"name":"stream",…}`) resource forms are handled identically; the TS twin's
/// `typeof r === 'string' ? r : r?.name` map is that same logic re-derived.
pub fn manifest_has_resource(manifest_json: &str, resource: &str, typ: &str) -> String {
    let manifest: AddonManifest = match serde_json::from_str(manifest_json) {
        Ok(m) => m,
        Err(e) => return Envelope::failed(false, parse_input(&e)).into_json(),
    };
    let typ = if typ.is_empty() { None } else { Some(typ) };
    Envelope::ok(crate::addon::manifest_has_resource(
        &manifest, resource, typ,
    ))
    .into_json()
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AddonRecord {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    manifest: AddonManifest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    addon_id: String,
    addon_name: String,
    #[serde(rename = "type")]
    catalog_type: String,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    base: String,
}

/// **#14** — flatten installed add-ons into a list of the catalogs they declare.
///
/// `records_json` is the shell's `AddonRecord` list minus the fields the core
/// must not see: `[{ "id", "url", "manifest" }]`. Records that advertise no
/// `catalog` resource are skipped; `base` is [`addon_base_url`] of the record's
/// URL. Replaces `addonClient.ts:147-156`.
///
/// Everything downstream of this — `mapAddonStream`, `parseStreamLangs`,
/// `detectQuality`, `extractSize`, `qualityRank`, `FLAG_LANG`
/// (`addonClient.ts:65-126`) — was reserved for a later increment when this
/// function was written, and is now #18-#22 below.
///
/// A record the core cannot read costs that record and warns `dropped.bad_item`,
/// rather than costing the user their whole catalog picker.
pub fn addon_catalogs(records_json: &str) -> String {
    let raw: Vec<serde_json::Value> = match serde_json::from_str(records_json) {
        Ok(v) => v,
        Err(e) => return Envelope::failed(Vec::<CatalogEntry>::new(), parse_input(&e)).into_json(),
    };

    let mut warnings: Vec<Warning> = Vec::new();
    let mut out: Vec<CatalogEntry> = Vec::new();

    for (i, value) in raw.into_iter().enumerate() {
        let record: AddonRecord = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                warnings.push(Warning::new(
                    WarningCode::DroppedBadItem,
                    format!("records[{i}]"),
                    e.to_string(),
                ));
                continue;
            }
        };
        if !crate::addon::manifest_has_resource(&record.manifest, "catalog", None) {
            continue;
        }
        let base = crate::addon::addon_base_url(&record.url);
        for decl in &record.manifest.catalogs {
            out.push(CatalogEntry {
                addon_id: record.id.clone(),
                addon_name: record.manifest.name.clone(),
                catalog_type: decl.catalog_type.clone(),
                id: decl.id.clone(),
                name: decl.name.clone(),
                base: base.clone(),
            });
        }
    }

    Envelope::ok(out).with_warnings(warnings).into_json()
}

// ---------------------------------------------------------------------------
// 15. home rows
// ---------------------------------------------------------------------------

/// **#15** — which home rows currently render, in the table's own order.
///
/// `rows_json` is `[{ "cat", "kind" }]` — **the table is an argument**;
/// `gating_json` is `{ "catalog", "providers", "studios" }`; `config_json` is
/// `{ cat: bool }` where absent means on.
///
/// Replaces `heartCatalog.ts:27-38` in full: `hydrate_row_config`, `set_gating`
/// and `visible_rows_json`, plus the `{...catalogRows, ...providerRows}`
/// flattening and the defensive `typeof x === 'string' ? x : x?.cat` map — three
/// calls and two workarounds collapse to one. Also replaces `Home.tsx:68-73`'s
/// `rowVisible` / `studiosVisible`.
///
/// On a parse failure `data` is `[]` **and `ok` is false**, and the distinction
/// matters at the call site: `Home.tsx:68` must read `ok:false` as "use my JS
/// gating", never read `[]` as "hide every row". An empty array is a legitimate
/// answer (nothing installed); a broken call is not.
pub fn visible_rows(rows_json: &str, gating_json: &str, config_json: &str) -> String {
    let fail =
        |e: &serde_json::Error| Envelope::failed(Vec::<String>::new(), parse_input(e)).into_json();

    let rows: Vec<RowDecl> = match serde_json::from_str(rows_json) {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let gating: Gating = match serde_json::from_str(gating_json) {
        Ok(g) => g,
        Err(e) => return fail(&e),
    };
    let config: BTreeMap<String, bool> = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };

    Envelope::ok(crate::rows::visible_rows(&rows, gating, &config)).into_json()
}

// ---------------------------------------------------------------------------
// 16-17. meta
// ---------------------------------------------------------------------------

/// **#16** — this build's identity, as a **bare string**, not an envelope.
///
/// It is the bootstrap probe: the shell calls it immediately after instantiate,
/// before it has any reason to trust the envelope format itself, and compares the
/// answer to the pinned vendored folder name it loaded from
/// (`public/assets/heart/<version>/`). A stale or mispointed pin is otherwise
/// completely invisible — it presents as "the app behaves like an old build",
/// never as an error. Being the thing everything else is checked against, it must
/// never fail and must never depend on anything that can.
pub fn core_version() -> String {
    crate::core_version()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Constants {
    history_cap: usize,
    progress_cap: usize,
    mylist_cap: usize,
    tombstone_ttl_ms: u64,
    resume_min_fraction: f64,
    resume_done_fraction: f64,
    collection_schema: u32,
    media_key_separator: &'static str,
}

/// **#17** — the numbers the shell must agree with the core about.
///
/// Four copies of 60 / 240 / 200 / 30d exist today — `history.ts:25-26`,
/// `library.ts:17`, `server.js:2109-2112` and the core — and nothing tests that
/// they agree. Publishing them makes the shell's copies derived rather than
/// parallel, which is the only version of "single source of truth" that survives
/// someone editing one of them.
///
/// `mediaKeySeparator` is here for the same reason: the shell builds `id:S#E#`
/// keys and [`library_remove`] sweeps by that prefix. If those two ever spell it
/// differently, removals stop cleaning up and nothing anywhere fails loudly.
pub fn core_constants() -> String {
    Envelope::ok(Constants {
        history_cap: crate::library::HISTORY_CAP,
        progress_cap: crate::library::PROGRESS_CAP,
        mylist_cap: crate::library::MYLIST_CAP,
        tombstone_ttl_ms: crate::library::TOMB_TTL_MS,
        resume_min_fraction: crate::library::RESUME_MIN_FRACTION,
        resume_done_fraction: crate::library::RESUME_DONE_FRACTION,
        collection_schema: COLLECTION_SCHEMA,
        media_key_separator: crate::library::MEDIA_KEY_SEPARATOR,
    })
    .into_json()
}

// ---------------------------------------------------------------------------
// 18-22. the add-on protocol proper: streams, catalogs, paths, ranking
// ---------------------------------------------------------------------------
//
// These five replace the half of `addonClient.ts` that is protocol rather than
// I/O. What stays in the shell stays for a stated reason and not because it was
// awkward: `fetchAddonJSON` (an AbortController and a 20-second timer),
// `collectAddonStreams` and `fetchAddonCatalog` (a `Promise.all` fan-out with a
// per-add-on catch), `toVttBlobUrl` (fetch + DecompressionStream +
// createObjectURL) and `langName` (i18n resolution, which plan 06 §3 keeps out of
// the core so a rendered string is never an FFI crossing).

/// **#18** — map an add-on's `stream/{type}/{id}.json` response into the flat
/// records the UI renders.
///
/// `data` is `AddonStream[]`:
/// `[{ source, label, quality, size, kind, url, langs, subtitles? }]`.
/// `addon_name` is what the shell shows as the source — `manifest.name`, or its
/// own fallback; the core has no opinion and no access to one.
///
/// Replaces `addonClient.ts:93-111` (`mapAddonStream`) together with the three
/// detectors only it calls — `detectQuality` (`:65`), `extractSize` (`:73`) and
/// `parseStreamLangs` (`:79`) — and the `.map(...).filter(Boolean)` at `:122`.
///
/// A stream the core cannot read costs that stream and warns `dropped.bad_item`;
/// a stream with no playable URL is dropped **silently**, because that is a
/// routine editorial decision (a torrent-only source in a browser shell) rather
/// than a fault, and warning on it would make every debrid add-on look broken.
///
/// ***U-coerce, fixed in [`crate::stream::Stream`] rather than here.*** The set of
/// streams the core "cannot read" used to include any whose `name` was a number,
/// and a good deal more besides — one `fileIdx: "2"` or `videoSize: "9.2 GB"` and
/// a perfectly playable source vanished out of this list. It was counted and
/// warned about, which is better than silence and still not something a user can
/// act on. The typing rule on that type says which fields may cost a stream
/// (`url`, and only `url`) and which may only cost themselves; `unreadable` here
/// now means what it says.
pub fn stream_parse(response_json: &str, addon_name: &str) -> String {
    let raw: serde_json::Value = match serde_json::from_str(response_json) {
        Ok(v) => v,
        Err(e) => {
            return Envelope::failed(Vec::<crate::stream::AddonStream>::new(), parse_input(&e))
                .into_json()
        }
    };

    // Count what the lenient parse dropped, so "the add-on sent 9 and you see 7"
    // is answerable. The alternative is a silent difference no log records.
    let sent = raw
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let response: crate::stream::StreamsResponse = serde_json::from_value(raw).unwrap_or_default();
    let unreadable = sent.saturating_sub(response.streams.len());

    let out = crate::stream::map_addon_streams(&response, addon_name);
    let mut env = Envelope::ok(out);
    if unreadable > 0 {
        env = env.with_warning(Warning::new(
            WarningCode::DroppedBadItem,
            addon_name,
            format!("{unreadable} stream(s) could not be read and were skipped"),
        ));
    }
    env.into_json()
}

/// **#19** — map an add-on's `catalog/{type}/{id}.json` response into poster cards.
///
/// `data` is `MediaItem[]`: `[{ id, type, title, year, rating, genre, poster }]`,
/// with cards that have no poster already removed — a card with no artwork is a
/// hole in a row, and the shell has always dropped them.
///
/// Replaces `addonClient.ts:130-141` (`mapCatalogMeta`) and the
/// `.map(mapCatalogMeta).filter((m) => m.poster)` at `:162`.
///
/// ***Recorded divergence.*** `type` goes through the core's `movie | series`
/// vocabulary, so an add-on that labels a show `"tv"` yields `"series"` here and
/// `"movie"` in the twin (`m.type === 'series' ? 'series' : 'movie'`, a strict
/// equality against one token). That mislabelling is not cosmetic: the card
/// renders with a film's chrome and its streams are then requested from
/// `stream/movie/…`, which returns nothing. This is the `movie|tv` vs
/// `movie|series` split the increment exists to close — see [`crate::media`].
pub fn catalog_metas(response_json: &str) -> String {
    let response: crate::catalog::CatalogResponse = match serde_json::from_str(response_json) {
        Ok(r) => r,
        Err(e) => {
            return Envelope::failed(Vec::<crate::catalog::CatalogItem>::new(), parse_input(&e))
                .into_json()
        }
    };
    Envelope::ok(crate::catalog::map_catalog_metas(&response.metas)).into_json()
}

/// **#20** — the path an add-on resource lives at, relative to [`addon_base_url`].
///
/// `data` is a `string`: `"stream/series/tt0903747%3A1%3A4.json"`,
/// `"catalog/movie/top.json"`. Cannot fail.
///
/// Replaces the two hand-built path expressions in `addonClient.ts` — `:118`'s
/// `'stream/' + type + '/' + encodeURIComponent(videoId) + '.json'` and `:161`'s
/// catalog equivalent — which are the same expression written twice. The
/// percent-encoding is `encodeURIComponent`'s, exactly, including its unreserved
/// set (`crate::media::encode_uri_component`): add-ons accept both the raw and the
/// encoded form, so this is a byte-identity requirement rather than a correctness
/// one, and byte identity is what the twin rule needs.
///
/// `media_type` is the **wire** vocabulary (`movie` / `series`) — pass
/// `crate::media::MediaKind::from_wire(x).as_wire()` if what you hold is a stored
/// `"tv"`.
pub fn addon_resource_path(resource: &str, media_type: &str, id: &str) -> String {
    Envelope::ok(crate::media::addon_resource_path(resource, media_type, id)).into_json()
}

/// **#21** — deduplicate and order language codes for the language tabs.
///
/// `langs_json` is `string[]`; `data` is `string[]`. Replaces
/// `addonClient.ts:32-38` (`orderLangs`), called at `DetailModal.tsx:182`, `:269`
/// and `:279`.
///
/// This survives [`rank_streams`] rather than being subsumed by it: ranking
/// answers "which source should play", and this answers "which tabs exist", which
/// is a different question about the same list.
///
/// ***Divergence risk, declared:*** the twin tie-breaks with `localeCompare` and
/// this uses byte order. They agree on every code the app can produce today (all
/// `[a-z]{2}`) and disagree on anything else — see [`crate::rank::order_langs`],
/// where the assumption is pinned by a test rather than assumed.
pub fn order_langs(langs_json: &str) -> String {
    let langs: Vec<String> = match serde_json::from_str(langs_json) {
        Ok(l) => l,
        Err(e) => return Envelope::failed(Vec::<String>::new(), parse_input(&e)).into_json(),
    };
    Envelope::ok(crate::rank::order_langs(&langs)).into_json()
}

/// **#22** — rank streams for *this* device. The shell probes, the core decides.
///
/// `streams_json` is the `AddonStream[]` from [`stream_parse`] (the raw wire shape
/// is accepted too — see [`crate::rank::RankCandidate`]); `caps_json` is the
/// device profile, `{}` for fully permissive. `data` is
/// `{ ranked: [...], summary: { playable, blocked, bestIndex } }`, where each
/// entry carries `index` back into the input array and the streams themselves are
/// never re-emitted.
///
/// Replaces both copies of `DetailModal.tsx`'s
/// `filter(langs.includes(want)).sort(qualityRank(b) - qualityRank(a))` (`:271`
/// and `:282`) — and, more to the point, it is the fix for `VideoPlayer.tsx:190`
/// declaring "Source unavailable" on a 4K HEVC stream that the panel's decoder
/// handles fine, because MSE answers for the browser's software pipeline and not
/// for the television. See [`crate::rank`] for the rule, which lives there and
/// only there so that LG and Android cannot disagree about it.
///
/// **Nothing is ever dropped.** A stream the device cannot play comes back with
/// `blocked: true` and the tokens that blocked it, so the UI can say *why* rather
/// than render an empty list. A profile that failed to probe an axis leaves it
/// empty, which means "no constraint" — never "allow nothing".
///
/// A malformed profile is `ok: false` with the streams ranked **permissively**
/// rather than an empty list: the shell must be able to tell "this device can play
/// none of these" from "the ranking call broke", and defaulting to permissive
/// keeps a bad profile from emptying the source list.
pub fn rank_streams(streams_json: &str, caps_json: &str) -> String {
    let empty = || crate::rank::Ranking {
        ranked: Vec::new(),
        summary: crate::rank::RankSummary {
            playable: 0,
            blocked: 0,
            best_index: None,
        },
    };

    let candidates: Vec<crate::rank::RankCandidate> = match serde_json::from_str(streams_json) {
        Ok(c) => c,
        Err(e) => return Envelope::failed(empty(), parse_input(&e)).into_json(),
    };

    match serde_json::from_str::<crate::rank::Capabilities>(caps_json) {
        Ok(caps) => Envelope::ok(crate::rank::rank_streams(&candidates, &caps)).into_json(),
        Err(e) => Envelope::failed(
            crate::rank::rank_streams(&candidates, &crate::rank::Capabilities::default()),
            parse_input(&e),
        )
        .into_json(),
    }
}

// ===========================================================================
// tests — driven through the boundary, against the exact strings JS receives
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Parse a boundary response. Every assertion below goes through this, so a
    /// function that returned something unparseable fails every one of its tests
    /// rather than silently passing a `contains()` check.
    fn env(s: &str) -> Value {
        serde_json::from_str(s).unwrap_or_else(|e| panic!("not JSON: {e}\n{s}"))
    }

    /// The five-key invariant, asserted on every single response in this file.
    fn assert_envelope_shape(v: &Value) {
        let o = v.as_object().expect("envelope must be an object");
        for k in ["ok", "core", "data", "warnings", "error"] {
            assert!(o.contains_key(k), "envelope is missing `{k}`: {v}");
        }
        assert!(o["ok"].is_boolean());
        assert!(o["core"].is_string());
        assert!(o["warnings"].is_array());
        // `data` is checked for PRESENCE above and never for a type: `null` is a
        // legitimate value (no resume, no payload file) and the whole point of
        // `ok` is that it, not `data`, carries the success/failure signal.
        assert!(o["error"].is_null() || o["error"]["code"].is_string());
    }

    fn ok_data(s: &str) -> Value {
        let v = env(s);
        assert_envelope_shape(&v);
        assert_eq!(v["ok"], Value::Bool(true), "expected ok:true — {s}");
        assert_eq!(v["error"], Value::Null);
        v["data"].clone()
    }

    fn err_of(s: &str) -> (String, Value) {
        let v = env(s);
        assert_envelope_shape(&v);
        assert_eq!(v["ok"], Value::Bool(false), "expected ok:false — {s}");
        let code = v["error"]["code"].as_str().unwrap().to_string();
        (code, v["data"].clone())
    }

    fn warning_codes(s: &str) -> Vec<String> {
        env(s)["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["code"].as_str().unwrap().to_string())
            .collect()
    }

    // ---- #1 official_payload_file ----

    const INDEX: &str = r#"{"schema":1,"version":"6","collections":[
        {"id":"community","section":"community","file":"community.json"},
        {"id":"official","section":"official","file":"addons.json"}]}"#;

    #[test]
    fn official_payload_file_finds_the_official_collection() {
        assert_eq!(
            ok_data(&official_payload_file(INDEX)),
            Value::from("addons.json")
        );
    }

    #[test]
    fn official_payload_file_rejects_a_future_schema() {
        let (code, data) = err_of(&official_payload_file(r#"{"schema":2,"collections":[]}"#));
        assert_eq!(code, "schema.unsupported");
        assert_eq!(data, Value::Null);
    }

    /// "Wrong schema" and "no official collection" must not collapse into one
    /// answer — they have different fixes.
    #[test]
    fn official_payload_file_distinguishes_missing_from_unsupported() {
        let (code, _) = err_of(&official_payload_file(
            r#"{"schema":1,"collections":[{"id":"c","section":"community","file":"c.json"}]}"#,
        ));
        assert_eq!(code, "not_found");
    }

    #[test]
    fn official_payload_file_rejects_garbage() {
        let (code, _) = err_of(&official_payload_file("not json at all"));
        assert_eq!(code, "parse.input");
    }

    // ---- #2 merge_official ----

    const INLINE: &str = r#"[
        {"id":"upcoming","section":"official","name":"Upcoming","ver":"v1.3.0","iconCls":"puzzle","installed":true},
        {"id":"catalog","section":"official","name":"Catalog","ver":"v1.0.0","iconCls":"puzzle","installed":true}]"#;

    fn ids(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn merge_official_appends_new_curated_cards() {
        let payload = r#"{"schema":1,"version":"6","addons":[
            {"id":"nebula","section":"official","name":"Nebula","version":"2.1.0"}]}"#;
        let data = ok_data(&merge_official(INLINE, payload));
        assert_eq!(ids(&data), vec!["upcoming", "catalog", "nebula"]);
        assert_eq!(data[2]["ver"], Value::from("v2.1.0"));
    }

    /// The guard `official.ts`'s `loadViaJs()` fallback does not have. A CDN
    /// record carrying a transport is rejected and *said so*, in the same call.
    #[test]
    fn merge_official_rejects_a_stream_bearing_card_and_warns() {
        let payload = r#"{"schema":1,"addons":[
            {"id":"pirate","section":"official","name":"P","transportUrl":"http://x/manifest.json"}]}"#;
        let s = merge_official(INLINE, payload);
        let data = ok_data(&s);
        assert_eq!(ids(&data), vec!["upcoming", "catalog"]);
        assert_eq!(warning_codes(&s), vec!["skipped.has_stream"]);
        assert_eq!(env(&s)["warnings"][0]["subject"], Value::from("pirate"));
    }

    #[test]
    fn merge_official_warns_on_an_unsafe_icon() {
        let payload =
            r#"{"schema":1,"addons":[{"id":"catalog","section":"official","iconCls":"<script>"}]}"#;
        let s = merge_official(INLINE, payload);
        assert_eq!(warning_codes(&s), vec!["dropped.unsafe_icon"]);
        assert_eq!(ok_data(&s)[1]["iconCls"], Value::from("puzzle"));
    }

    /// `skipped.no_id` was unreachable: a record with no `id` key failed the whole
    /// payload deserialize, so the guard written for it had never once fired and
    /// one bad CDN row cost every client the entire official collection.
    #[test]
    fn merge_official_costs_a_bad_record_only_that_record() {
        let payload = r#"{"schema":1,"addons":[
            {"section":"official","name":"idless"},
            {"id":"broken","section":"official","types":42},
            {"id":"nebula","section":"official","name":"Nebula","version":"2.1.0"}]}"#;
        let s = merge_official(INLINE, payload);
        let data = ok_data(&s);
        assert_eq!(
            ids(&data),
            vec!["upcoming", "catalog", "nebula"],
            "the good record must still land: {s}"
        );
        assert_eq!(warning_codes(&s), vec!["dropped.bad_item", "skipped.no_id"]);
        assert_eq!(
            env(&s)["warnings"][0]["subject"],
            Value::from("addons[1]"),
            "an unreadable record is named by position — it has no id to name it by"
        );
    }

    /// The contract that makes a CDN outage free: `data` is the inline list, byte
    /// for byte, on every failure path.
    #[test]
    fn merge_official_keeps_inline_on_every_failure() {
        for (payload, expected) in [
            ("}{ not json", "parse.input"),
            (r#"{"schema":2,"addons":[]}"#, "schema.unsupported"),
        ] {
            let (code, data) = err_of(&merge_official(INLINE, payload));
            assert_eq!(code, expected);
            assert_eq!(ids(&data), vec!["upcoming", "catalog"]);
        }
    }

    #[test]
    fn merge_official_with_unreadable_inline_is_empty_and_says_so() {
        let (code, data) = err_of(&merge_official("nope", r#"{"schema":1,"addons":[]}"#));
        assert_eq!(code, "parse.input");
        assert_eq!(data, serde_json::json!([]));
    }

    /// The payload is a DOCUMENT, not a bare array — that is where the schema gate
    /// lives, and passing an array must fail rather than quietly skip the gate.
    #[test]
    fn merge_official_requires_a_document_not_an_array() {
        let (code, _) = err_of(&merge_official(
            INLINE,
            r#"[{"id":"x","section":"official"}]"#,
        ));
        assert_eq!(code, "parse.input");
    }

    // ---- #3 reconcile_install_state ----

    const ADDONS: &str = r#"[{"id":"a","installed":true},{"id":"b","installed":false},
                             {"id":"c","installed":true,"locked":true}]"#;

    fn reconcile_req(local_at: u64, remote: Option<(&str, u64)>, owner_changed: bool) -> String {
        let remote = match remote {
            Some((map, at)) => format!(r#"{{"map":{map},"at":{at}}}"#),
            None => "null".to_string(),
        };
        format!(
            r#"{{"addons":{ADDONS},"local":{{"map":{{"a":true,"b":false}},"at":{local_at}}},
                 "remote":{remote},"ownerChanged":{owner_changed}}}"#
        )
    }

    #[test]
    fn reconcile_adopts_a_newer_remote() {
        let s = reconcile_install_state(&reconcile_req(
            10,
            Some((r#"{"a":false,"b":true}"#, 20)),
            false,
        ));
        let data = ok_data(&s);
        assert_eq!(data["decision"], Value::from("adoptRemote"));
        assert_eq!(data["at"], Value::from(20));
        assert_eq!(data["map"]["a"], Value::Bool(false));
        assert_eq!(data["addons"][0]["installed"], Value::Bool(false));
    }

    #[test]
    fn reconcile_uploads_when_local_is_newer() {
        let s = reconcile_install_state(&reconcile_req(30, Some((r#"{"a":false}"#, 20)), false));
        let data = ok_data(&s);
        assert_eq!(data["decision"], Value::from("uploadLocal"));
        assert_eq!(data["at"], Value::from(30));
        assert_eq!(data["addons"][0]["installed"], Value::Bool(true));
    }

    /// The shared-living-room-TV case: a different account owned the local copy,
    /// so remote wins even though it is older.
    #[test]
    fn reconcile_adopts_remote_when_the_owner_changed() {
        let s = reconcile_install_state(&reconcile_req(30, Some((r#"{"a":false}"#, 20)), true));
        assert_eq!(ok_data(&s)["decision"], Value::from("adoptRemote"));
    }

    #[test]
    fn reconcile_noops_for_a_new_owner_with_nothing_to_adopt() {
        let s = reconcile_install_state(&reconcile_req(0, None, true));
        assert_eq!(ok_data(&s)["decision"], Value::from("noop"));
    }

    #[test]
    fn reconcile_uploads_when_there_is_no_remote() {
        let s = reconcile_install_state(&reconcile_req(0, None, false));
        assert_eq!(ok_data(&s)["decision"], Value::from("uploadLocal"));
    }

    /// A locked id is never toggled by a map and never appears in the map that is
    /// persisted or pushed — it is behaviour, not preference.
    #[test]
    fn reconcile_never_touches_a_locked_addon() {
        let s = reconcile_install_state(&reconcile_req(10, Some((r#"{"c":false}"#, 20)), false));
        let data = ok_data(&s);
        let c = data["addons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "c")
            .unwrap();
        assert_eq!(c["installed"], Value::Bool(true));
        assert!(data["map"].get("c").is_none());
    }

    #[test]
    fn reconcile_echoes_addons_it_can_still_read_when_the_request_is_broken() {
        // `local` is a string where an object belongs — the typed parse fails,
        // but the descriptors are still there and must come back.
        let s = reconcile_install_state(r#"{"addons":[{"id":"a"}],"local":"nonsense"}"#);
        let (code, data) = err_of(&s);
        assert_eq!(code, "parse.input");
        assert_eq!(data["decision"], Value::from("noop"));
        assert_eq!(ids(&data["addons"]), vec!["a"]);
    }

    // ---- #4 merge_library ----

    #[test]
    fn merge_library_merges_by_recency() {
        let local = r#"{"history":[{"id":"a","at":100}]}"#;
        let remote = r#"{"history":[{"id":"a","at":300},{"id":"b","at":200}]}"#;
        let data = ok_data(&merge_library(local, remote, 1000.0));
        assert_eq!(ids(&data["history"]), vec!["a", "b"]);
        assert_eq!(data["history"][0]["at"], Value::from(300));
    }

    /// THE regression this whole phase exists to close: an unreadable local
    /// document must never come back as `{}`.
    #[test]
    fn merge_library_never_wipes_on_an_unreadable_side() {
        let good = r#"{"history":[{"id":"a","at":100}],"removed":{"z":5}}"#;

        let (code, data) = err_of(&merge_library("<<<not json>>>", good, 1000.0));
        assert_eq!(code, "parse.input");
        assert_eq!(ids(&data["history"]), vec!["a"], "remote must be preserved");

        let (code, data) = err_of(&merge_library(good, "<<<not json>>>", 1000.0));
        assert_eq!(code, "parse.input");
        assert_eq!(ids(&data["history"]), vec!["a"], "local must be preserved");
        assert_eq!(data["removed"]["z"], Value::from(5));
    }

    /// A row the core cannot read costs that row, not the document — **and it is
    /// never free.** The leniency was right; returning `ok:true` with an empty
    /// `warnings` array while the shell persisted the shortened library was not.
    #[test]
    fn merge_library_drops_only_the_bad_row_and_says_which() {
        let local = r#"{"history":[{"id":"good","at":2},{"garbage":true},{"id":603,"at":1}],
                        "progress":{"good":{"pos":1,"dur":2,"at":3},"broken":null}}"#;
        let s = merge_library(local, "{}", 1000.0);
        let data = ok_data(&s);
        assert_eq!(ids(&data["history"]), vec!["good", "603"]);
        assert!(data["progress"]["broken"].is_null());
        assert_eq!(data["progress"]["good"]["pos"], Value::from(1.0));

        let ws = env(&s)["warnings"].clone();
        let seen: Vec<(&str, &str)> = ws
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["code"].as_str().unwrap(), w["subject"].as_str().unwrap()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("dropped.bad_item", "local.history[1]"),
                ("coerced.field", "local.history[2]"),
                ("dropped.bad_item", "local.progress[broken]"),
            ],
            "{s}"
        );
    }

    /// Which document a lost row came out of is half the information — both sides
    /// number their rows from zero, so a bare `history[0]` names two records.
    #[test]
    fn merge_library_says_which_side_a_lost_row_came_from() {
        let s = merge_library(
            r#"{"history":[{"nope":true}]}"#,
            r#"{"history":[{"also_nope":true}]}"#,
            1000.0,
        );
        let subjects: Vec<String> = env(&s)["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["subject"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(subjects, vec!["local.history[0]", "remote.history[0]"]);
    }

    /// A call that already failed still has to account for the side it COULD read
    /// — that document is what `data` is, and the shell is about to persist it.
    #[test]
    fn merge_library_reports_rows_even_on_the_failure_path() {
        let s = merge_library(
            "<<<not json>>>",
            r#"{"history":[{"id":"a","at":1},{"garbage":true}]}"#,
            1000.0,
        );
        let (code, data) = err_of(&s);
        assert_eq!(code, "parse.input");
        assert_eq!(ids(&data["history"]), vec!["a"]);
        assert_eq!(warning_codes(&s), vec!["dropped.bad_item"]);
        assert_eq!(
            env(&s)["warnings"][0]["subject"],
            Value::from("remote.history[1]")
        );
    }

    /// D1 + D2: an exact tie is resolved by the DATA, so the same result falls out
    /// whichever way round the arguments go — which is the only reason two devices
    /// that pull from each other ever stop disagreeing.
    #[test]
    fn merge_library_is_commutative_on_ties() {
        let a = r#"{"history":[{"id":"zulu","at":100},{"id":"alpha","at":100}],
                    "progress":{"k":{"pos":1,"dur":100,"at":500}}}"#;
        let b = r#"{"history":[{"id":"mike","at":100}],
                    "progress":{"k":{"pos":2,"dur":100,"at":500}}}"#;
        let ab = ok_data(&merge_library(a, b, 1000.0));
        let ba = ok_data(&merge_library(b, a, 1000.0));
        assert_eq!(ids(&ab["history"]), vec!["alpha", "mike", "zulu"]);
        assert_eq!(ids(&ba["history"]), vec!["alpha", "mike", "zulu"]);
        // D1: the tie goes to the further-along position, not to whichever
        // document was passed first. `local wins` was argument order wearing a
        // rule's clothes, and it is why two devices never reconverged.
        assert_eq!(ab["progress"]["k"]["pos"], Value::from(2.0));
        assert_eq!(ab, ba, "the whole document must be order-independent");
    }

    /// D8, through the boundary: the shell deletes a series, the other device has
    /// not synced, and the merge must not hand the episode keys back.
    #[test]
    fn merge_library_does_not_resurrect_a_removed_titles_progress() {
        let local = r#"{"removed":{"tt1":200}}"#;
        let stale = r#"{"history":[{"id":"tt1","at":100}],
                        "progress":{"tt1":{"pos":5,"dur":100,"at":100},
                                    "tt1:S1E1":{"pos":10,"dur":100,"at":100}}}"#;

        let ab = ok_data(&merge_library(local, stale, 1000.0));
        assert_eq!(
            ab["progress"],
            serde_json::json!({}),
            "the delete was undone"
        );
        assert_eq!(ab["history"].as_array().unwrap().len(), 0);

        let ba = ok_data(&merge_library(stale, local, 1000.0));
        assert_eq!(
            ab, ba,
            "which device pulled must not decide what was deleted"
        );
    }

    /// …and the other half of D8: a position recorded AFTER the removal is a
    /// re-watch, and comes back with the history entry rather than being swept.
    #[test]
    fn merge_library_keeps_a_position_newer_than_its_tombstone() {
        let local = r#"{"removed":{"tt1":200}}"#;
        let remote = r#"{"history":[{"id":"tt1","at":300}],
                         "progress":{"tt1:S1E1":{"pos":10,"dur":100,"at":300},
                                     "tt1:S2E9":{"pos":20,"dur":100,"at":100}}}"#;
        let data = ok_data(&merge_library(local, remote, 1000.0));
        let keys: Vec<&str> = data["progress"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["tt1:S1E1"]);
        assert_eq!(ids(&data["history"]), vec!["tt1"]);
    }

    /// D7: one id, one millisecond, two different rows. The old rule kept
    /// whichever document was passed first, so the two devices never agreed on
    /// that row's content however many times they synced.
    #[test]
    fn merge_library_converges_on_a_same_id_equal_at_collision() {
        let a = r#"{"history":[{"id":"tt1","title":"Alpha","at":100}],
                    "mylist":[{"id":"m","title":"Alpha","at":100}]}"#;
        let b = r#"{"history":[{"id":"tt1","title":"Beta","at":100}],
                    "mylist":[{"id":"m","title":"Beta","at":100}]}"#;
        let ab = ok_data(&merge_library(a, b, 1000.0));
        let ba = ok_data(&merge_library(b, a, 1000.0));
        assert_eq!(ab, ba, "the row's content must be order-independent");
        assert_eq!(ab["history"][0]["title"], Value::from("Beta"));
        assert_eq!(ab["mylist"][0]["title"], Value::from("Beta"));
    }

    /// D4: `lang` rides along with the position it belongs to, in both directions.
    /// This is what deletes `history.ts::keepLangs()`.
    #[test]
    fn merge_library_carries_lang_from_the_losing_record() {
        let local = r#"{"progress":{"k":{"pos":10,"dur":100,"at":100,"lang":"jpn"}}}"#;
        let remote = r#"{"progress":{"k":{"pos":80,"dur":100,"at":900}}}"#;
        let p = ok_data(&merge_library(local, remote, 1000.0))["progress"]["k"].clone();
        assert_eq!(p["pos"], Value::from(80.0), "newer position wins");
        assert_eq!(p["lang"], Value::from("jpn"), "…but the language carries");
    }

    #[test]
    fn merge_library_warns_about_caps_and_pruned_tombstones() {
        let mut history = Vec::new();
        for i in 0..70 {
            history.push(format!(r#"{{"id":"h{i:03}","at":{i}}}"#));
        }
        let local = format!(
            r#"{{"history":[{}],"removed":{{"stale":1}}}}"#,
            history.join(",")
        );
        let s = merge_library(&local, "{}", 3_000_000_000.0);
        let codes = warning_codes(&s);
        assert!(codes.contains(&"pruned.tombstone".to_string()), "{codes:?}");
        assert!(
            codes.contains(&"truncated.history".to_string()),
            "{codes:?}"
        );
        assert_eq!(ok_data(&s)["history"].as_array().unwrap().len(), 60);
    }

    /// A partial document — which is the only kind either store ever pushes —
    /// contributes its half and leaves the other half alone.
    #[test]
    fn merge_library_treats_a_partial_document_as_a_contribution() {
        let local = r#"{"history":[{"id":"a","at":100}],"mylist":[{"id":"m","at":100}]}"#;
        let remote = r#"{"mylist":[{"id":"n","at":200}]}"#;
        let data = ok_data(&merge_library(local, remote, 1000.0));
        assert_eq!(ids(&data["history"]), vec!["a"]);
        assert_eq!(ids(&data["mylist"]), vec!["n", "m"]);
    }

    /// A device with a broken clock must not take the library with it.
    ///
    /// The assertion here used to be `data["removed"].is_object()`, which an EMPTY
    /// object satisfies — so it passed against the very wipe it was written to
    /// forbid, and `Infinity` was silently pruning every tombstone on the device.
    /// The tombstones themselves are what has to be checked, by value.
    #[test]
    fn merge_library_clamps_a_hostile_clock() {
        for now in [f64::NAN, -1.0, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            let s = merge_library(r#"{"removed":{"x":50,"y":4000}}"#, "{}", now);
            let data = ok_data(&s);
            assert_eq!(
                data["removed"],
                serde_json::json!({"x": 50, "y": 4000}),
                "now={now} pruned a tombstone it had no clock to age",
            );
            assert!(
                warning_codes(&s).is_empty(),
                "nothing expired, so nothing to report: {s}",
            );
        }
    }

    /// …and a clock the core CAN believe still expires what it should, or the
    /// clamp above would be indistinguishable from switching pruning off.
    #[test]
    fn merge_library_still_prunes_under_a_believable_clock() {
        let ttl = crate::library::TOMB_TTL_MS as f64;
        let s = merge_library(r#"{"removed":{"x":1000}}"#, "{}", 1000.0 + ttl + 1.0);
        assert_eq!(warning_codes(&s), vec!["pruned.tombstone"]);
        assert_eq!(ok_data(&s)["removed"], serde_json::json!({}));
    }

    // ---- #5 library_record_watch ----

    #[test]
    fn record_watch_upserts_and_floats_to_the_front() {
        let lib = r#"{"history":[{"id":"a","at":100},{"id":"b","at":200}]}"#;
        let data = ok_data(&library_record_watch(lib, r#"{"id":"a","at":300}"#));
        assert_eq!(ids(&data["history"]), vec!["a", "b"]);
        assert_eq!(data["history"][0]["at"], Value::from(300));
    }

    /// Re-watching un-removes: the tombstone has to go or the next merge deletes
    /// the entry we just wrote.
    #[test]
    fn record_watch_clears_the_tombstone() {
        let lib = r#"{"history":[],"removed":{"a":999}}"#;
        let data = ok_data(&library_record_watch(lib, r#"{"id":"a","at":300}"#));
        assert!(data["removed"].as_object().unwrap().is_empty());
    }

    /// Heart returned `"[]"` here and the shell wrote it back as the new state.
    /// The write is dropped either way — but now it is dropped *loudly*, and the
    /// library survives.
    #[test]
    fn record_watch_echoes_the_library_when_the_item_is_unreadable() {
        let lib = r#"{"history":[{"id":"a","at":100}]}"#;
        let (code, data) = err_of(&library_record_watch(lib, r#"{"no_id":true}"#));
        assert_eq!(code, "parse.field");
        assert_eq!(ids(&data["history"]), vec!["a"]);
    }

    /// Every function that reads a library reports what reading it cost — this is
    /// the write path, where the shell takes `data` and persists it immediately,
    /// so a row lost here is a row lost for good.
    #[test]
    fn record_watch_reports_the_rows_the_library_lost() {
        let lib = r#"{"history":[{"id":"a","at":100},{"broken":true}]}"#;
        let s = library_record_watch(lib, r#"{"id":"b","at":300}"#);
        assert_eq!(ids(&ok_data(&s)["history"]), vec!["b", "a"]);
        assert_eq!(warning_codes(&s), vec!["dropped.bad_item"]);
        // One document, so no `local.`/`remote.` qualifier to add.
        assert_eq!(env(&s)["warnings"][0]["subject"], Value::from("history[1]"));
    }

    #[test]
    fn record_watch_warns_when_the_cap_ate_an_entry() {
        let mut history = Vec::new();
        for i in 0..60 {
            history.push(format!(r#"{{"id":"h{i:03}","at":{}}}"#, 1000 + i));
        }
        let lib = format!(r#"{{"history":[{}]}}"#, history.join(","));
        let s = library_record_watch(&lib, r#"{"id":"fresh","at":9999}"#);
        assert_eq!(warning_codes(&s), vec!["truncated.history"]);
        assert_eq!(ok_data(&s)["history"].as_array().unwrap().len(), 60);
    }

    // ---- #6 library_remove ----

    /// D3, the behaviour fix. Heart left every `tt1:S#E#` key behind forever.
    #[test]
    fn remove_sweeps_every_episode_progress_key() {
        let lib = r#"{"history":[{"id":"tt1","at":1}],
                      "progress":{"tt1":{"pos":1,"dur":9,"at":1},
                                  "tt1:S1E1":{"pos":2,"dur":9,"at":2},
                                  "tt1:S2E9":{"pos":3,"dur":9,"at":3},
                                  "tt10":{"pos":4,"dur":9,"at":4},
                                  "tt10:S1E1":{"pos":5,"dur":9,"at":5}}}"#;
        let data = ok_data(&library_remove(lib, "tt1", 99.0));
        let keys: Vec<&str> = data["progress"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec!["tt10", "tt10:S1E1"],
            "a sibling id must not be swept"
        );
        assert!(data["history"].as_array().unwrap().is_empty());
        assert_eq!(data["removed"]["tt1"], Value::from(99));
    }

    #[test]
    fn remove_refuses_an_empty_id() {
        let (code, data) = err_of(&library_remove(
            r#"{"history":[{"id":"a","at":1}]}"#,
            "",
            5.0,
        ));
        assert_eq!(code, "parse.field");
        assert_eq!(ids(&data["history"]), vec!["a"]);
    }

    // ---- #7 mylist_toggle ----

    #[test]
    fn mylist_toggle_reports_the_new_membership() {
        let added = ok_data(&mylist_toggle("{}", r#"{"id":"tt1"}"#, 100.0));
        assert_eq!(added["inList"], Value::Bool(true));
        assert_eq!(added["library"]["mylist"][0]["at"], Value::from(100));

        let lib = serde_json::to_string(&added["library"]).unwrap();
        let removed = ok_data(&mylist_toggle(&lib, r#"{"id":"tt1"}"#, 200.0));
        assert_eq!(removed["inList"], Value::Bool(false));
        assert!(removed["library"]["mylist"].as_array().unwrap().is_empty());
        assert_eq!(removed["library"]["mylistRemoved"]["tt1"], Value::from(200));
    }

    /// The persisted key spelling is `mylistRemoved` and must stay that way —
    /// renaming it orphans every installed user's removals.
    #[test]
    fn mylist_toggle_keeps_the_wire_key_spelling() {
        let lib = r#"{"mylist":[{"id":"tt1","at":1}]}"#;
        let s = mylist_toggle(lib, r#"{"id":"tt1"}"#, 200.0);
        assert!(s.contains("\"mylistRemoved\""), "{s}");
    }

    #[test]
    fn mylist_toggle_echoes_the_library_on_a_bad_item() {
        let (code, data) = err_of(&mylist_toggle(
            r#"{"mylist":[{"id":"x","at":1}]}"#,
            "{}",
            5.0,
        ));
        assert_eq!(code, "parse.field");
        assert_eq!(ids(&data["library"]["mylist"]), vec!["x"]);
        assert_eq!(data["inList"], Value::Bool(false));
    }

    // ---- #8 continue_watching ----

    const CW_LIB: &str = r#"{"history":[{"id":"a","at":300},{"id":"b","key":"b:S1E2","at":200}],
                             "progress":{"a":{"pos":99,"dur":100,"at":1},
                                         "b:S1E2":{"pos":30,"dur":100,"at":2}}}"#;

    /// hideFinished defaults FALSE, which is what makes this a drop-in for a rail
    /// that today maps every history entry.
    #[test]
    fn continue_watching_shows_everything_by_default() {
        for opts in ["{}", "", "   "] {
            let data = ok_data(&continue_watching(CW_LIB, opts));
            assert_eq!(data.as_array().unwrap().len(), 2, "opts={opts:?}");
        }
    }

    #[test]
    fn continue_watching_resolves_key_fraction_and_resume() {
        let data = ok_data(&continue_watching(CW_LIB, "{}"));
        // "a" is finished: full bar, but no resume offer.
        assert_eq!(data[0]["key"], Value::from("a"));
        assert_eq!(data[0]["fraction"], Value::from(0.99));
        assert!(data[0]["resume"].is_null());
        // "b" carries an episode media key and is mid-watch.
        assert_eq!(data[1]["key"], Value::from("b:S1E2"));
        assert_eq!(data[1]["fraction"], Value::from(0.3));
        assert_eq!(data[1]["resume"]["pos"], Value::from(30.0));
        assert_eq!(data[1]["item"]["id"], Value::from("b"));
    }

    #[test]
    fn continue_watching_honours_hide_finished() {
        let data = ok_data(&continue_watching(CW_LIB, r#"{"hideFinished":true}"#));
        assert_eq!(data.as_array().unwrap().len(), 1);
        assert_eq!(data[0]["item"]["id"], Value::from("b"));
    }

    /// A broken options document costs the flag, not the rail.
    #[test]
    fn continue_watching_still_returns_the_rail_when_options_are_broken() {
        let (code, data) = err_of(&continue_watching(CW_LIB, "!!!"));
        assert_eq!(code, "parse.input");
        assert_eq!(data.as_array().unwrap().len(), 2);
    }

    #[test]
    fn continue_watching_returns_an_empty_rail_for_an_unreadable_library() {
        let (code, data) = err_of(&continue_watching("!!!", "{}"));
        assert_eq!(code, "parse.input");
        assert_eq!(data, serde_json::json!([]));
    }

    // ---- #9 resume_position ----

    /// D5: the window is [0.01, 0.94]. 0.92 is the case that pins which threshold
    /// actually won — it sits between Heart's 0.9 and the shell's 0.94.
    #[test]
    fn resume_position_window() {
        let lib = r#"{"progress":{
            "just_started":{"pos":0.5,"dur":100,"at":1},
            "mid":{"pos":50,"dur":100,"at":1},
            "old_cutoff":{"pos":92,"dur":100,"at":1},
            "near_end":{"pos":95,"dur":100,"at":1},
            "no_dur":{"pos":10,"dur":0,"at":1}}}"#;
        assert!(ok_data(&resume_position(lib, "just_started")).is_null());
        assert_eq!(
            ok_data(&resume_position(lib, "mid"))["pos"],
            Value::from(50.0)
        );
        assert_eq!(
            ok_data(&resume_position(lib, "old_cutoff"))["pos"],
            Value::from(92.0)
        );
        assert!(ok_data(&resume_position(lib, "near_end")).is_null());
        assert!(ok_data(&resume_position(lib, "no_dur")).is_null());
        assert!(ok_data(&resume_position(lib, "absent")).is_null());
    }

    /// "No resume" and "the core is broken" are the same `null` to a shell that
    /// only reads `data` — which is why `ok` exists.
    #[test]
    fn resume_position_distinguishes_absent_from_broken() {
        assert_eq!(env(&resume_position("{}", "k"))["ok"], Value::Bool(true));
        let (code, data) = err_of(&resume_position("!!!", "k"));
        assert_eq!(code, "parse.input");
        assert!(data.is_null());
    }

    // ---- #10 normalize_manifest_url ----

    /// The five inputs in the divergence ledger, executable rather than prose.
    #[test]
    fn normalize_manifest_url_fixes_the_typescript_twin() {
        let cases = [
            (
                "https://a.co/x/manifest.json?y=2",
                "https://a.co/x/manifest.json?y=2",
            ),
            (
                "https://a.co/addon?x=1",
                "https://a.co/addon/manifest.json?x=1",
            ),
            ("https://a.co/x", "https://a.co/x/manifest.json"),
            ("groloo://a.co/x", "https://a.co/x/manifest.json"),
            (
                "https://a.co/cfg=a|b,c/manifest.json?x=1",
                "https://a.co/cfg=a|b,c/manifest.json?x=1",
            ),
        ];
        for (raw, expect) in cases {
            assert_eq!(
                ok_data(&normalize_manifest_url(raw)),
                Value::from(expect),
                "{raw}"
            );
        }
    }

    /// A foreign scheme is rewritten, not rejected — that is how a shared
    /// `stremio://` link installs. Only a string that names no host at all fails,
    /// and it fails with a code the shell can branch on.
    #[test]
    fn normalize_manifest_url_rewrites_foreign_schemes_and_rejects_non_urls() {
        for raw in ["ftp://a.co/x", "stremio://a.co/x", "a.co/x"] {
            assert_eq!(
                ok_data(&normalize_manifest_url(raw)),
                Value::from("https://a.co/x/manifest.json"),
                "{raw}"
            );
        }
        for raw in ["https://", "", "   ", "javascript:alert(1)", "not-an-id"] {
            let (code, data) = err_of(&normalize_manifest_url(raw));
            assert_eq!(code, "invalid.url", "{raw}");
            assert!(data.is_null());
        }
    }

    /// **U-slash at the boundary.** A canonical manifest URL that arrived with a
    /// trailing slash — which is what an address bar and most link shorteners hand
    /// back — must normalise to itself, not grow a second `/manifest.json` and
    /// 404. The query and the `|`-packed debrid config have to survive the trim;
    /// [`crate::addon`] tests the combinations, this asserts the answer the shell
    /// actually receives.
    #[test]
    fn normalize_manifest_url_strips_a_trailing_slash_before_testing_for_json() {
        for (raw, expect) in [
            ("https://a.co/manifest.json/", "https://a.co/manifest.json"),
            (
                "https://a.co/x/manifest.json/?y=2",
                "https://a.co/x/manifest.json?y=2",
            ),
            (
                "https://a.co/cfg=a|b,c/manifest.json/",
                "https://a.co/cfg=a|b,c/manifest.json",
            ),
        ] {
            assert_eq!(
                ok_data(&normalize_manifest_url(raw)),
                Value::from(expect),
                "{raw}"
            );
        }
    }

    /// Normalisation is a fixpoint. The TS twin's output grows a `/manifest.json`
    /// on every pass, which is the defect stated as a property.
    #[test]
    fn normalize_manifest_url_is_a_fixpoint() {
        for raw in [
            "https://a.co/x",
            "https://a.co/addon?x=1",
            "groloo://a.co/y/",
            "https://a.co/manifest.json/",
        ] {
            let once = ok_data(&normalize_manifest_url(raw));
            let twice = ok_data(&normalize_manifest_url(once.as_str().unwrap()));
            assert_eq!(once, twice, "{raw}");
        }
    }

    // ---- #11 addon_base_url ----

    #[test]
    fn addon_base_url_strips_the_last_segment() {
        for (url, expect) in [
            ("https://a.co/x/manifest.json", "https://a.co/x/"),
            ("https://a.co/", "https://a.co/"),
            ("noslash", ""),
            ("", ""),
        ] {
            assert_eq!(ok_data(&addon_base_url(url)), Value::from(expect), "{url}");
        }
    }

    // ---- #12 validate_manifest ----

    const GOOD_MANIFEST: &str =
        r#"{"id":"com.x.addon","name":"X","resources":["catalog"],"types":["movie"]}"#;

    #[test]
    fn validate_manifest_accepts_a_good_one() {
        let data = ok_data(&validate_manifest(GOOD_MANIFEST));
        assert_eq!(data["id"], Value::from("com.x.addon"));
    }

    /// Four rules, four messages — against the shell's one
    /// "Not a valid add-on manifest" for all of them.
    #[test]
    fn validate_manifest_messages_are_distinct_and_verbatim() {
        let cases = [
            (
                r#"{"id":"-bad","name":"X","resources":["a"],"types":["movie"]}"#,
                "\"id\"",
            ),
            (
                r#"{"id":"x","name":"  ","resources":["a"],"types":["movie"]}"#,
                "\"name\"",
            ),
            (
                r#"{"id":"x","name":"X","resources":[],"types":["movie"]}"#,
                "\"resources\"",
            ),
            (
                r#"{"id":"x","name":"X","resources":["a"],"types":[]}"#,
                "\"types\"",
            ),
        ];
        for (json, needle) in cases {
            let v = env(&validate_manifest(json));
            assert_eq!(
                v["error"]["code"],
                Value::from("invalid.manifest"),
                "{json}"
            );
            let detail = v["error"]["detail"].as_str().unwrap();
            assert!(detail.contains(needle), "{detail} should mention {needle}");
        }
    }

    /// **U-notobject, fixed.** A manifest URL that answers `null`, a bare string or
    /// a number is refused with `server.js:1947`'s own first sentence, not with
    /// serde's `invalid type: null, expected struct AddonManifest`. That string is
    /// rendered on the add-ons screen, and `error.detail` is documented as carrying
    /// the server's message verbatim — so this is the doc and the code being made
    /// to agree, in the direction that leaves the user with a sentence.
    #[test]
    fn validate_manifest_says_a_non_object_is_not_a_json_object() {
        for json in ["null", "5", r#""a string""#, "true", "0"] {
            let v = env(&validate_manifest(json));
            assert_envelope_shape(&v);
            assert_eq!(v["ok"], Value::Bool(false), "{json}");
            assert_eq!(
                v["error"]["code"],
                Value::from("invalid.manifest"),
                "{json}"
            );
            assert_eq!(
                v["error"]["detail"],
                Value::from("Manifest is not a JSON object"),
                "{json}"
            );
        }
        // Still a parse failure when it is not JSON at all — there is no document
        // to have an opinion about.
        let (code, _) = err_of(&validate_manifest("<html>"));
        assert_eq!(code, "parse.input");
        // And an object still reaches the four typed rules.
        let (code, _) = err_of(&validate_manifest(r#"{"id":"-bad"}"#));
        assert_eq!(code, "invalid.manifest");
    }

    /// `data` on failure is the best-effort parse, so a caller can still name the
    /// add-on it just refused.
    #[test]
    fn validate_manifest_returns_the_parse_even_when_it_fails() {
        let (_, data) = err_of(&validate_manifest(
            r#"{"id":"x","name":"Debrid Thing","resources":[],"types":[]}"#,
        ));
        assert_eq!(data["name"], Value::from("Debrid Thing"));
    }

    // ---- #13 manifest_has_resource ----

    #[test]
    fn manifest_has_resource_handles_short_and_full_forms() {
        let m = r#"{"id":"x","name":"X","types":["movie","series"],
                    "resources":["catalog",{"name":"stream","types":["movie"]}]}"#;
        assert_eq!(
            ok_data(&manifest_has_resource(m, "stream", "")),
            Value::Bool(true)
        );
        assert_eq!(
            ok_data(&manifest_has_resource(m, "catalog", "movie")),
            Value::Bool(true)
        );
        assert_eq!(
            ok_data(&manifest_has_resource(m, "catalog", "channel")),
            Value::Bool(false)
        );
        assert_eq!(
            ok_data(&manifest_has_resource(m, "meta", "")),
            Value::Bool(false)
        );
    }

    /// `""` means "any type", not "a type literally called empty string" — the
    /// whole reason the signature can stay `&str`.
    #[test]
    fn manifest_has_resource_treats_empty_type_as_any() {
        let m = r#"{"id":"x","name":"X","types":[],"resources":["catalog"]}"#;
        assert_eq!(
            ok_data(&manifest_has_resource(m, "catalog", "")),
            Value::Bool(true)
        );
    }

    #[test]
    fn manifest_has_resource_is_false_not_broken_on_garbage() {
        let (code, data) = err_of(&manifest_has_resource("!!!", "catalog", ""));
        assert_eq!(code, "parse.input");
        assert_eq!(data, Value::Bool(false));
    }

    // ---- #14 addon_catalogs ----

    #[test]
    fn addon_catalogs_flattens_declarations() {
        let records = r#"[
          {"id":"cinemeta","url":"https://c.co/x/manifest.json","manifest":{
             "id":"cm","name":"Cinemeta","resources":["catalog","meta"],"types":["movie"],
             "catalogs":[{"type":"movie","id":"top","name":"Top"},{"type":"series","id":"trend"}]}},
          {"id":"streamer","url":"https://s.co/manifest.json","manifest":{
             "id":"st","name":"Streamer","resources":["stream"],"types":["movie"],
             "catalogs":[{"type":"movie","id":"nope"}]}}]"#;
        let data = ok_data(&addon_catalogs(records));
        assert_eq!(
            data.as_array().unwrap().len(),
            2,
            "the stream-only add-on is skipped"
        );
        assert_eq!(data[0]["addonId"], Value::from("cinemeta"));
        assert_eq!(data[0]["addonName"], Value::from("Cinemeta"));
        assert_eq!(data[0]["type"], Value::from("movie"));
        assert_eq!(data[0]["id"], Value::from("top"));
        assert_eq!(data[0]["name"], Value::from("Top"));
        assert_eq!(data[0]["base"], Value::from("https://c.co/x/"));
        assert!(
            data[1].get("name").is_none(),
            "an unnamed catalog omits the key"
        );
    }

    #[test]
    fn addon_catalogs_drops_a_bad_record_and_warns() {
        let records = r#"[42,{"id":"a","url":"https://a.co/manifest.json","manifest":{
             "id":"a","name":"A","resources":["catalog"],"types":["movie"],
             "catalogs":[{"type":"movie","id":"c"}]}}]"#;
        let s = addon_catalogs(records);
        assert_eq!(warning_codes(&s), vec!["dropped.bad_item"]);
        assert_eq!(ok_data(&s).as_array().unwrap().len(), 1);
    }

    #[test]
    fn addon_catalogs_is_empty_not_broken_for_an_empty_list() {
        assert_eq!(ok_data(&addon_catalogs("[]")), serde_json::json!([]));
        let (code, data) = err_of(&addon_catalogs("{}"));
        assert_eq!(code, "parse.input");
        assert_eq!(data, serde_json::json!([]));
    }

    // ---- #15 visible_rows ----

    const ROWS: &str = r#"[{"cat":"trending_movie","kind":"catalog"},
                           {"cat":"top_movie","kind":"catalog"},
                           {"cat":"prov_netflix","kind":"provider"},
                           {"cat":"studios","kind":"studio"}]"#;

    #[test]
    fn visible_rows_applies_gating_and_per_row_config() {
        let s = visible_rows(
            ROWS,
            r#"{"catalog":true,"providers":true,"studios":true}"#,
            r#"{"top_movie":false}"#,
        );
        assert_eq!(
            ok_data(&s),
            serde_json::json!(["trending_movie", "prov_netflix", "studios"])
        );
    }

    /// The distinction `Home.tsx:68` has to make: `[]` is a legitimate answer and
    /// `ok:false` is not, and only one of them should fall back to JS gating.
    #[test]
    fn visible_rows_separates_empty_from_broken() {
        let empty = visible_rows(
            ROWS,
            r#"{"catalog":false,"providers":false,"studios":false}"#,
            "{}",
        );
        assert_eq!(env(&empty)["ok"], Value::Bool(true));
        assert_eq!(ok_data(&empty), serde_json::json!([]));

        for (r, g, c) in [
            ("!!!", "{}", "{}"),
            (ROWS, "!!!", "{}"),
            (ROWS, "{}", "!!!"),
        ] {
            let (code, data) = err_of(&visible_rows(r, g, c));
            assert_eq!(code, "parse.input");
            assert_eq!(data, serde_json::json!([]));
        }
    }

    // ---- #16-17 meta ----

    /// Bare string, never an envelope. It is what everything else is checked
    /// against, so it cannot depend on the format being trusted yet.
    #[test]
    fn core_version_is_a_bare_token() {
        let v = core_version();
        assert!(!v.starts_with('{'), "must not be an envelope: {v}");
        assert!(v.contains("+g"), "{v}");
        assert_eq!(v, crate::CORE_VERSION);
    }

    #[test]
    fn core_constants_publishes_every_number_the_shell_hardcodes() {
        let d = ok_data(&core_constants());
        assert_eq!(d["historyCap"], Value::from(60));
        assert_eq!(d["progressCap"], Value::from(240));
        assert_eq!(d["mylistCap"], Value::from(200));
        assert_eq!(d["tombstoneTtlMs"], Value::from(2_592_000_000u64));
        assert_eq!(d["resumeMinFraction"], Value::from(0.01));
        assert_eq!(d["resumeDoneFraction"], Value::from(0.94));
        assert_eq!(d["collectionSchema"], Value::from(1));
        assert_eq!(d["mediaKeySeparator"], Value::from(":"));
    }

    /// The constants are not a second copy — they must be the same symbols the
    /// merge and the caps actually use.
    #[test]
    fn core_constants_are_the_symbols_not_literals() {
        let d = ok_data(&core_constants());
        assert_eq!(d["historyCap"], Value::from(crate::library::HISTORY_CAP));
        assert_eq!(
            d["resumeDoneFraction"],
            Value::from(crate::library::RESUME_DONE_FRACTION)
        );
        assert_eq!(
            d["mediaKeySeparator"],
            Value::from(crate::library::MEDIA_KEY_SEPARATOR)
        );
    }

    // ---- #18 stream_parse ----

    /// The exact bytes the shell receives, against the exact bytes
    /// `mapAddonStream` produces. Everything else about this function is tested in
    /// `crate::stream`; what is asserted here is the boundary itself.
    /// Asserted against the RAW response, not through [`Value`]: `serde_json`'s
    /// value tree sorts object keys, and field order is exactly what the twin
    /// comparison is about.
    fn assert_data_bytes(response: &str, expected: &str) {
        assert_envelope_shape(&env(response));
        assert!(
            response.contains(&format!(r#""data":{expected},"#)),
            "data bytes differ\n  expected: {expected}\n  actual:   {response}"
        );
    }

    #[test]
    fn stream_parse_answers_with_the_mapped_records() {
        assert_data_bytes(
            &stream_parse(
                r#"{"streams":[
                 {"url":"https://a.co/v.m3u8","name":"🇬🇧 Provider","title":"Movie 1080p 2.3 GB"},
                 {"name":"no url, dropped"},
                 {"url":"https://a.co/w.mp4","behaviorHints":{"lang":"ka"}}
               ]}"#,
                "Torrentio",
            ),
            r#"[{"source":"Torrentio","label":"🇬🇧 Provider\nMovie 1080p 2.3 GB","quality":"1080p","size":"2.3 GB","kind":"hls","url":"https://a.co/v.m3u8","langs":["en"]},{"source":"Torrentio","label":"Source","quality":"","size":null,"kind":"url","url":"https://a.co/w.mp4","langs":["ka"]}]"#,
        );
    }

    /// A missing `streams` array is an empty list and NOT a failure — an add-on
    /// with nothing for this title is a normal answer.
    #[test]
    fn stream_parse_treats_an_empty_response_as_empty_not_broken() {
        for json in [r#"{}"#, r#"{"streams":[]}"#, r#"{"streams":null}"#] {
            assert_eq!(
                ok_data(&stream_parse(json, "X")),
                Value::from(Vec::<Value>::new())
            );
        }
    }

    /// An unreadable entry costs that entry and is *reported* — the difference
    /// between nine sources becoming seven and nobody knowing why.
    #[test]
    fn stream_parse_warns_about_what_it_could_not_read() {
        let v = env(&stream_parse(
            r#"{"streams":[{"url":"a"},42,{"url":"b"}]}"#,
            "Torrentio",
        ));
        assert_eq!(v["ok"], Value::Bool(true), "a bad row is not a failed call");
        assert_eq!(v["data"].as_array().map(Vec::len), Some(2));
        assert_eq!(v["warnings"][0]["code"], Value::from("dropped.bad_item"));
        assert_eq!(v["warnings"][0]["subject"], Value::from("Torrentio"));
    }

    /// **U-coerce at the boundary.** The add-on sent two sources; the user must see
    /// two. A numeric `name` is labelled as JavaScript labels it, and a stray type
    /// on a field nothing dereferences costs the field — so `unreadable` stays 0
    /// and no warning is raised, because nothing was in fact unreadable.
    #[test]
    fn stream_parse_keeps_a_source_whose_name_is_a_number() {
        let v = env(&stream_parse(
            r#"{"streams":[{"name":1080,"url":"https://a.co/v.mp4"},
                           {"name":"good","url":"https://a.co/w.mp4","behaviorHints":{"videoSize":"9.2 GB"}}]}"#,
            "Torrentio",
        ));
        assert_eq!(v["ok"], Value::Bool(true));
        assert_eq!(v["data"].as_array().map(Vec::len), Some(2));
        assert_eq!(v["data"][0]["label"], Value::from("1080"));
        assert_eq!(v["data"][0]["quality"], Value::from("1080p"));
        assert_eq!(
            v["warnings"].as_array().map(Vec::len),
            Some(0),
            "nothing was dropped, so nothing should be reported"
        );
    }

    #[test]
    fn stream_parse_reports_a_broken_response_without_losing_the_shape() {
        let (code, data) = err_of(&stream_parse("not json", "X"));
        assert_eq!(code, "parse.input");
        assert_eq!(data, Value::from(Vec::<Value>::new()));
    }

    // ---- #19 catalog_metas ----

    #[test]
    fn catalog_metas_maps_and_drops_the_posterless() {
        assert_data_bytes(
            &catalog_metas(
                r#"{"metas":[
                 {"id":"tt1","name":"A","type":"series","releaseInfo":"2008-2013",
                  "imdbRating":"9.5","genres":["Crime"],"poster":"https://a.co/p.jpg"},
                 {"id":"tt2","name":"No poster"}
               ]}"#,
            ),
            r#"[{"id":"tt1","type":"series","title":"A","year":"2008","rating":9.5,"genre":"Crime","poster":"https://a.co/p.jpg"}]"#,
        );
    }

    /// The declared divergence, asserted at the boundary: `"tv"` is a series here
    /// and a movie in the twin.
    #[test]
    fn catalog_metas_reads_the_tv_dialect_the_twin_mislabels() {
        let d = ok_data(&catalog_metas(
            r#"{"metas":[{"id":"tt1","type":"tv","poster":"p"}]}"#,
        ));
        assert_eq!(d[0]["type"], Value::from("series"));
    }

    // ---- #20 addon_resource_path ----

    #[test]
    fn addon_resource_path_encodes_like_encode_uri_component() {
        assert_eq!(
            ok_data(&addon_resource_path("stream", "series", "tt0903747:1:4")),
            Value::from("stream/series/tt0903747%3A1%3A4.json")
        );
        assert_eq!(
            ok_data(&addon_resource_path("catalog", "movie", "top")),
            Value::from("catalog/movie/top.json")
        );
    }

    // ---- #21 order_langs ----

    #[test]
    fn order_langs_dedupes_and_orders() {
        assert_eq!(
            ok_data(&order_langs(r#"["ru","en","ka","en","fr"]"#)),
            serde_json::json!(["en", "ka", "ru", "fr"])
        );
        assert_eq!(ok_data(&order_langs("[]")), serde_json::json!([]));
        let (code, data) = err_of(&order_langs("nope"));
        assert_eq!(code, "parse.input");
        assert_eq!(data, serde_json::json!([]));
    }

    // ---- #22 rank_streams ----

    /// The whole point, at the boundary: the blocked stream is still in the
    /// answer, it says why, and the unlabelled one is still playable.
    #[test]
    fn rank_streams_returns_the_blocked_ones_with_reasons() {
        let d = ok_data(&rank_streams(
            r#"[{"label":"Movie 2160p HEVC","langs":["en"],"url":"https://a.co/a.mkv"},
                {"label":"Some Release","langs":["en"],"url":"https://a.co/b"},
                {"label":"Movie 1080p x264","langs":["en"],"url":"https://a.co/c.mp4"}]"#,
            r#"{"video":["avc1"],"containers":["mp4"],"preferLangs":["en"]}"#,
        ));
        assert_eq!(d["summary"]["bestIndex"], Value::from(2));
        assert_eq!(d["summary"]["playable"], Value::from(2));
        assert_eq!(d["summary"]["blocked"], Value::from(1));
        assert_eq!(d["ranked"].as_array().map(Vec::len), Some(3));

        let blocked = d["ranked"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["index"] == 0)
            .unwrap()
            .clone();
        assert_eq!(blocked["blocked"], Value::Bool(true));
        assert_eq!(
            blocked["blockedBy"],
            serde_json::json!(["video:hvc1", "container:mkv"])
        );
    }

    /// A device that can play nothing is a *reportable* state, not an empty list —
    /// `bestIndex: null` with the reasons intact.
    #[test]
    fn rank_streams_says_no_compatible_source_rather_than_nothing() {
        let d = ok_data(&rank_streams(
            r#"[{"label":"Movie 2160p HEVC","url":"https://a.co/a.mkv"}]"#,
            r#"{"video":["avc1"]}"#,
        ));
        assert_eq!(d["summary"]["bestIndex"], Value::Null);
        assert_eq!(d["ranked"].as_array().map(Vec::len), Some(1));
    }

    /// `{}` is the permissive profile, and a *broken* profile degrades to it
    /// rather than to an empty source list — with `ok:false`, so the shell can
    /// tell the two apart.
    #[test]
    fn rank_streams_degrades_a_broken_profile_to_permissive() {
        let streams = r#"[{"label":"Movie 2160p HEVC","url":"https://a.co/a.mkv"}]"#;
        assert_eq!(
            ok_data(&rank_streams(streams, "{}"))["summary"]["blocked"],
            Value::from(0)
        );
        let v = env(&rank_streams(streams, "not json"));
        assert_eq!(v["ok"], Value::Bool(false));
        assert_eq!(v["error"]["code"], Value::from("parse.input"));
        assert_eq!(
            v["data"]["summary"]["bestIndex"],
            Value::from(0),
            "a bad profile must not cost the user every source"
        );
    }

    /// Broken *streams* are a different matter: there is nothing to rank, and the
    /// summary says so without pretending the call succeeded.
    #[test]
    fn rank_streams_reports_unreadable_input() {
        let (code, data) = err_of(&rank_streams("not json", "{}"));
        assert_eq!(code, "parse.input");
        assert_eq!(data["summary"]["playable"], Value::from(0));
        assert_eq!(data["ranked"], serde_json::json!([]));
    }

    // ---- cross-cutting ----

    /// Every enveloped function, on a deliberately hostile input, still answers
    /// with all five keys. A shell that destructures `data` can never hit an
    /// exception it has to guess the meaning of.
    #[test]
    fn every_function_answers_with_a_well_formed_envelope() {
        let bad = "\u{0}not json\u{0}";
        let responses = vec![
            official_payload_file(bad),
            merge_official(bad, bad),
            reconcile_install_state(bad),
            merge_library(bad, bad, f64::NAN),
            library_record_watch(bad, bad),
            library_remove(bad, "", f64::NAN),
            mylist_toggle(bad, bad, f64::NAN),
            continue_watching(bad, bad),
            resume_position(bad, ""),
            normalize_manifest_url(bad),
            addon_base_url(bad),
            validate_manifest(bad),
            manifest_has_resource(bad, "", ""),
            addon_catalogs(bad),
            visible_rows(bad, bad, bad),
            core_constants(),
            stream_parse(bad, "Add-on"),
            catalog_metas(bad),
            addon_resource_path(bad, bad, bad),
            order_langs(bad),
            rank_streams(bad, bad),
        ];
        assert_eq!(
            responses.len(),
            21,
            "21 enveloped functions + core_version = 22"
        );
        for s in &responses {
            assert_envelope_shape(&env(s));
            assert_eq!(
                env(s)["core"],
                Value::from(crate::CORE_VERSION),
                "unversioned response: {s}"
            );
        }
    }

    /// Tombstone maps are `BTreeMap`, so the wire form is key-sorted and two
    /// devices that merged the same thing produce the same bytes. The differential
    /// harness canonicalises anyway, but a boundary that is already deterministic
    /// is one less thing the harness has to paper over.
    #[test]
    fn tombstone_output_is_key_sorted() {
        let t: crate::library::Tombstones =
            [("z".to_string(), 1u64), ("a".to_string(), 2u64)].into();
        let lib = serde_json::json!({ "removed": t }).to_string();
        let s = merge_library(&lib, "{}", 3.0);
        let pos_a = s.find("\"a\"").unwrap();
        let pos_z = s.find("\"z\"").unwrap();
        assert!(pos_a < pos_z, "removed map must serialise key-sorted: {s}");
    }
}
