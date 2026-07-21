//! The library CRDT — watch history, resume positions, My List, and the
//! tombstones that stop a removal on one device losing to an older copy on
//! another.
//!
//! VENDORED from `Groloo-Heart@0.1.0 src/library.rs`. The merge rules, the caps
//! and the tombstone TTL are Heart's, unchanged, because they are also the web
//! client's, the My List store's and the server's — four hand-maintained copies of
//! one algorithm, none of which is tested against the others. Single-sourcing them
//! is the point of this module.
//!
//! ## What did not come across
//!
//! `LibMsg` / `LibEffect` / `update()`. `Persist`, `SchedulePush` and `Repaint`
//! are shell concerns, and the shell already does them better than the effect
//! vocabulary could express: `history.ts` runs a 25-second coalescing push with a
//! keepalive flush on `pagehide`, which `SchedulePush` cannot describe. What
//! survives is the state and the rules over it.
//!
//! ## What changed, and why each change is a bug being closed
//!
//! - **[`Library`] is widened** to the whole `/api/library-state` document
//!   (`+ mylist`, `+ mylistRemoved`). That is the shape both ends already speak,
//!   and it is what lets one merge replace `stores/library.ts`'s copy as well as
//!   `stores/history.ts`'s. All five fields are `#[serde(default)]`: a partial
//!   document contributes nothing rather than wiping, because the two stores push
//!   independently and each sees only its own half.
//! - **`Progress` carries `lang`** (see [`crate::types::Progress`]), and
//!   [`merge_progress`] carries a language forward from the *losing* record when
//!   the winner has none. Without that rule, syncing from an older client — or
//!   from the server, which stores whatever was PUT and synthesises nothing —
//!   silently erases a language a newer client saved.
//! - **[`Library::remove`] sweeps episode progress.** The shell keys progress by
//!   media key (`id` for a film, `id:S#E#` for an episode) while Heart deleted
//!   `progress[id]` only, so removing a series from Continue Watching left every
//!   episode's resume position behind forever — counting against
//!   [`PROGRESS_CAP`] and resurrecting if the title was ever re-added.
//! - **…and that sweep is now merge-durable.** Deleting locally is only half a
//!   delete in a system where the other device still holds the row: [`merge_progress`]
//!   unions both maps, so the next sync handed every episode key straight back.
//!   [`sweep_removed_progress`] re-applies the sweep against the *merged*
//!   tombstone map, under the same at-or-after rule [`merge_history`] already uses
//!   for the history entry itself. Without it, "the thing I deleted came back" is
//!   not a bug report, it is the specification.
//! - **Ties are given a total order.** Heart resolved an equal-`at` history tie
//!   through a `BTreeMap`'s incidental iteration order and the shell through a
//!   `Map`'s insertion order, so the same two inputs produced different output on
//!   the two implementations. Both list merges now sort by `(at DESC, key ASC)`
//!   explicitly, an equal-`at` collision *on one id* is settled by
//!   [`content_order`], and an equal-`at` *progress* tie is settled by
//!   [`progress_tie_break`] rather than by whichever side was passed first — see
//!   their docs for why "local wins the tie" was an argument-order preference and
//!   not a rule. A value that gets serialised and compared may not depend on a
//!   container's implementation detail, nor on which device initiated the sync.
//! - **Truncation moved out of the merges** into [`Library::cap_history`] and
//!   friends, which report how many records they dropped. Same sort, same cap,
//!   same result — but the boundary can now say `truncated.history` instead of a
//!   user's oldest 20 titles disappearing wordlessly.
//! - **The "finished" threshold is 0.94**, matching `history.ts`'s
//!   `PROGRESS_DONE`. Heart's 0.9 had no consumer; the shell's number is the one
//!   users actually experience.

use crate::envelope::Warning;
use crate::salvage;
use crate::types::{de, LibraryItem, MyListItem, Progress};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Most recent history entries kept per account.
pub const HISTORY_CAP: usize = 60;
/// Most resume positions kept per account.
pub const PROGRESS_CAP: usize = 240;
/// Most My List entries kept per account.
pub const MYLIST_CAP: usize = 200;
/// Tombstones older than this are pruned on merge — 30 days, in ms. Written as a
/// literal because the crate denies `arithmetic_side_effects` and a constant that
/// has to explain itself in a comment is clearer than one that has to be computed.
pub const TOMB_TTL_MS: u64 = 2_592_000_000;

/// Below this fraction a "resume" is really "start over" — offering to resume at
/// eight seconds in is noise.
pub const RESUME_MIN_FRACTION: f64 = 0.01;
/// At or above this fraction a title counts as watched: no resume offer, and it
/// can be hidden from Continue Watching. This is `history.ts`'s `PROGRESS_DONE`;
/// Heart's competing 0.9 is retired.
pub const RESUME_DONE_FRACTION: f64 = 0.94;
/// What separates a title id from its episode coordinates in a media key
/// (`tt123:S1E4`). A core concept because [`Library::remove`] cannot sweep an
/// episode's progress without knowing how episode keys are spelled.
///
/// Re-exported, not defined here. The definition moved to [`crate::media`] when
/// that module took ownership of the media-key format — this module is a
/// *consumer* of the format, and a constant that lives next to its first caller is
/// a constant that grows a second copy the moment it gains a second one. The path
/// `library::MEDIA_KEY_SEPARATOR` keeps working because `api::core_constants`
/// publishes it under that name to every shell.
pub use crate::media::MEDIA_KEY_SEPARATOR;

/// Tombstone map: id -> removal timestamp (ms).
pub type Tombstones = BTreeMap<String, u64>;

/// The library / watch state — the whole `/api/library-state` document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    /// Recently-watched items, newest first, capped at [`HISTORY_CAP`].
    #[serde(default, deserialize_with = "de::lenient_vec")]
    pub history: Vec<LibraryItem>,
    /// Resume positions, keyed by media key (see [`LibraryItem::media_key`]).
    #[serde(default, deserialize_with = "de::lenient_map")]
    pub progress: BTreeMap<String, Progress>,
    /// Tombstones for ids removed from history.
    #[serde(default, deserialize_with = "de::lenient_map")]
    pub removed: Tombstones,
    /// My List (watchlist), newest first, capped at [`MYLIST_CAP`].
    #[serde(default, deserialize_with = "de::lenient_vec")]
    pub mylist: Vec<MyListItem>,
    /// Tombstones for ids removed from My List. The key spelling is the one
    /// already on the wire and already in localStorage; renaming it would orphan
    /// every installed user's removals.
    #[serde(
        rename = "mylistRemoved",
        default,
        deserialize_with = "de::lenient_map"
    )]
    pub mylist_removed: Tombstones,
}

impl Library {
    /// Raw resume record for a media key, whatever its state.
    pub fn resume(&self, key: &str) -> Option<Progress> {
        self.progress.get(key).cloned()
    }

    /// The resume record only when offering it is actually useful: a known
    /// duration, past the "you just started" floor, and short of the "you've
    /// finished" ceiling. This is `history.ts::getResume` verbatim.
    pub fn resumable(&self, key: &str) -> Option<Progress> {
        let p = self.progress.get(key)?;
        if p.dur <= 0.0 {
            return None;
        }
        let f = p.fraction();
        if !(RESUME_MIN_FRACTION..=RESUME_DONE_FRACTION).contains(&f) {
            return None;
        }
        Some(p.clone())
    }

    /// Has this id been tombstoned out of history?
    pub fn is_removed(&self, id: &str) -> bool {
        self.removed.contains_key(id)
    }

    /// Is this id in My List?
    pub fn in_list(&self, id: &str) -> bool {
        self.mylist.iter().any(|m| m.id == id)
    }

    /// Items for the "Continue Watching" rail, newest first (history is already
    /// ordered).
    ///
    /// `hide_finished` defaults to *false* at the boundary on purpose: today's
    /// rail renders every history entry regardless of completion, so filtering is
    /// a UI decision someone should make deliberately rather than inherit from a
    /// core that used to do it unasked. Heart's behaviour is `true`.
    pub fn continue_watching(&self, hide_finished: bool) -> Vec<&LibraryItem> {
        self.history
            .iter()
            .filter(|it| {
                if !hide_finished {
                    return true;
                }
                self.progress
                    .get(it.media_key())
                    .map(|p| p.fraction() < RESUME_DONE_FRACTION)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Sort history newest-first and enforce [`HISTORY_CAP`]. Returns how many
    /// entries were dropped, so the boundary can warn instead of losing them
    /// quietly.
    pub fn cap_history(&mut self) -> usize {
        self.history
            .sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id().cmp(b.id())));
        let dropped = self.history.len().saturating_sub(HISTORY_CAP);
        self.history.truncate(HISTORY_CAP);
        dropped
    }

    /// Enforce [`PROGRESS_CAP`], keeping the most recent. Returns the drop count.
    pub fn cap_progress(&mut self) -> usize {
        if self.progress.len() <= PROGRESS_CAP {
            return 0;
        }
        let mut pairs: Vec<(String, Progress)> =
            std::mem::take(&mut self.progress).into_iter().collect();
        pairs.sort_by(|a, b| b.1.at.cmp(&a.1.at).then_with(|| a.0.cmp(&b.0)));
        let dropped = pairs.len().saturating_sub(PROGRESS_CAP);
        pairs.truncate(PROGRESS_CAP);
        self.progress = pairs.into_iter().collect();
        dropped
    }

    /// Sort My List newest-first and enforce [`MYLIST_CAP`]. Returns the drop count.
    pub fn cap_mylist(&mut self) -> usize {
        self.mylist
            .sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
        let dropped = self.mylist.len().saturating_sub(MYLIST_CAP);
        self.mylist.truncate(MYLIST_CAP);
        dropped
    }

    /// A title was watched or opened — upsert it into history, float it to the
    /// front, and clear any tombstone (re-watching un-removes). The item's `at` is
    /// authoritative; the core does not own a clock.
    pub fn record_watch(&mut self, item: LibraryItem) {
        let id = item.id().to_string();
        self.removed.remove(&id);
        self.history.retain(|it| it.id() != id);
        self.history.push(item);
        self.cap_history();
    }

    /// Write one resume position.
    ///
    /// Deliberately **not** on the JSON boundary: it fires every few seconds
    /// during playback, and routing it through the core would serialise the whole
    /// library across linear memory twice to write a single key. The shell keeps
    /// that write. This exists because the rule — including "a tick with no
    /// language keeps the language already stored" — has to live somewhere
    /// testable, and because `merge_libraries` has to agree with it.
    pub fn set_progress(&mut self, key: &str, pos: f64, dur: f64, now: u64, lang: Option<String>) {
        let carried = lang.or_else(|| self.progress.get(key).and_then(|p| p.lang.clone()));
        self.progress.insert(
            key.to_string(),
            Progress {
                pos,
                dur,
                at: now,
                lang: carried,
            },
        );
        self.cap_progress();
    }

    /// Remove a title: tombstone it, drop its history entry, and drop **every**
    /// resume position it owns — the bare key and every `id:S#E#` episode key
    /// under it.
    ///
    /// This is only the half that happens *here*. The keys come back the moment a
    /// device that has not synced yet pushes its copy, so the durable half is
    /// [`sweep_removed_progress`], which re-derives this same set from the
    /// tombstone on every merge. Neither is redundant: this one is what makes the
    /// removal visible before the next sync, that one is what makes it stick.
    pub fn remove(&mut self, id: &str, now: u64) {
        self.removed.insert(id.to_string(), now);
        self.history.retain(|it| it.id() != id);
        let prefix = format!("{id}{MEDIA_KEY_SEPARATOR}");
        self.progress
            .retain(|k, _| k != id && !k.starts_with(&prefix));
    }

    /// Toggle a title in My List. Returns the **new** membership state, which is
    /// what `stores/library.ts::toggle` returns and what its call sites read.
    pub fn mylist_toggle(&mut self, item: MyListItem, now: u64) -> bool {
        let id = item.id.clone();
        if self.in_list(&id) {
            self.mylist.retain(|m| m.id != id);
            self.mylist_removed.insert(id, now); // the removal must win across devices
            false
        } else {
            let mut fresh = item;
            fresh.at = now;
            self.mylist.insert(0, fresh);
            self.mylist_removed.remove(&id); // re-adding clears any prior tombstone
            self.cap_mylist();
            true
        }
    }
}

/// Parse a `/api/library-state` document, reporting every row the leniency cost.
///
/// [`Library`]'s own `Deserialize` is lenient and **silent**, and it stays that
/// way: a `deserialize_with` hook has no channel back to the caller, and the
/// property that matters most about it — `serde_json::from_str::<Library>` can
/// never fail because of one bad row, whoever calls it — is worth keeping for
/// exactly the caller who would otherwise write `unwrap_or_default()` and wipe a
/// library. What that caller cannot have is an account of what it cost them.
///
/// So everything that answers a shell comes through here instead: the same
/// leniency, the same result, plus the [`Warning`] per dropped or coerced row that
/// [`crate::envelope`]'s doctrine requires. Two implementations of one rule is a
/// drift waiting to happen, which is why `parse_library_agrees_with_the_derive`
/// pins them against each other over the hostile inputs rather than trusting that
/// they were written from the same intent.
///
/// The `Err` case is document-level only — the string is not JSON, or it is JSON
/// that is not a library (a bare array, a number, a `history` that is a string).
/// Those are the cases where there is no row to be lenient about and the caller
/// must choose a fallback that is not `{}`.
pub fn parse_library(json: &str) -> Result<(Library, Vec<Warning>), serde_json::Error> {
    let mut obj = match serde_json::from_str::<Value>(json)? {
        Value::Object(o) => o,
        // Not a document at all. Let serde produce the authentic error rather than
        // invent a message for it — the shell shows `error.detail` to nobody but a
        // developer, and serde's is the one that names the type it expected.
        other => return serde_json::from_value::<Library>(other).map(|l| (l, Vec::new())),
    };

    // Fixed order, so two devices reporting the same broken document report it in
    // the same order. Field names are the wire names: `mylistRemoved` is the
    // spelling already in every installed user's localStorage.
    let mut warnings = Vec::new();
    let library = Library {
        history: salvage::items("history", obj.remove("history"), &mut warnings)?,
        progress: salvage::entries("progress", obj.remove("progress"), &mut warnings)?,
        removed: salvage::entries("removed", obj.remove("removed"), &mut warnings)?,
        mylist: salvage::items("mylist", obj.remove("mylist"), &mut warnings)?,
        mylist_removed: salvage::entries(
            "mylistRemoved",
            obj.remove("mylistRemoved"),
            &mut warnings,
        )?,
    };
    Ok((library, warnings))
}

/// What a whole-library merge dropped or clamped, so the boundary can report it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryMergeReport {
    pub truncated_history: usize,
    pub truncated_progress: usize,
    pub truncated_mylist: usize,
    /// Ids whose tombstones aged past [`TOMB_TTL_MS`] and were forgotten.
    pub pruned_tombstones: Vec<String>,
}

/// A merged library plus the account of what the merge cost.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryMerge {
    pub library: Library,
    pub report: LibraryMergeReport,
}

/// Union two tombstone maps, keeping the newest removal per id. No pruning — the
/// TTL is applied separately so the caller can see *which* ids expired.
pub fn union_tombstones(a: &Tombstones, b: &Tombstones) -> Tombstones {
    let mut out: Tombstones = BTreeMap::new();
    for (id, &at) in a.iter().chain(b.iter()) {
        let e = out.entry(id.clone()).or_insert(0);
        if at > *e {
            *e = at;
        }
    }
    out
}

/// Drop tombstones older than [`TOMB_TTL_MS`] relative to `now`, returning the
/// ids that were forgotten.
pub fn prune_tombstones(t: &mut Tombstones, now: u64) -> Vec<String> {
    let expired: Vec<String> = t
        .iter()
        .filter(|(_, &at)| now.saturating_sub(at) > TOMB_TTL_MS)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &expired {
        t.remove(id);
    }
    expired
}

/// Merge two tombstone maps (keep the newest removal per id) and prune entries
/// older than [`TOMB_TTL_MS`] relative to `now`.
pub fn merge_tombstones(a: &Tombstones, b: &Tombstones, now: u64) -> Tombstones {
    let mut out = union_tombstones(a, b);
    prune_tombstones(&mut out, now);
    out
}

/// The order two records for the **same id at the same `at`** are compared in:
/// their canonical JSON, lexicographically, greater wins.
///
/// **Why there has to be one.** `Some(prev) if prev.at >= it.at => {}` keeps
/// whichever record the loop reached first, and the loop reaches `local` first —
/// so device A merging B keeps A's row and device B merging A keeps B's, and the
/// two never agree on that row's *content* however many rounds they trade. The
/// list *ordering* was already given a total order for exactly this reason; the
/// per-id upsert underneath it was not, and this closes the same hole one level
/// down. It is the [`progress_tie_break`] argument applied to the half of the
/// document that has no `pos`.
///
/// **Why bytes and not a semantic rule.** [`Progress`] had a real answer — of two
/// equally-recent positions the further-along one has seen the other's frames, so
/// "never rewind the viewer" is a rule a user could be told. A [`LibraryItem`] has
/// no such field: nothing about a poster, a title or a genre is *more true* than
/// the other side's copy, and inventing a per-field preference (longest title
/// wins? non-null poster wins?) would be an arbitrary rule that has to be
/// re-litigated every time the struct grows a field. Canonical bytes are arbitrary
/// too — but honestly so, and they buy three things a hand-written order does not:
/// they are **total** over every field simultaneously, they **stay** total when a
/// field is added, and they are the *same* bytes the differential harness and the
/// shell compare, so "the two devices agree" and "the two documents are equal" are
/// the same statement.
///
/// The cost is real and worth naming: the winner is not the *better* record, only
/// the *agreed* one, and a user watching two devices could see a title's poster
/// flip once. It flips once and then stops, which is strictly better than a row
/// that disagrees forever.
///
/// **What "canonical" is resting on**, since the word is doing real work here:
/// `serde_json` is built without `preserve_order`, so the one free-form field in
/// these types (`MyListItem::rating`, a passed-through [`Value`]) serialises its
/// object keys in `BTreeMap` order rather than in the order they arrived. Two
/// devices that received the same rating object through different JSON parsers
/// therefore produce the same bytes. Turning that feature on would silently make
/// this order depend on input order again — which is the bug, one level further
/// down.
///
/// A serialisation that fails compares as empty. It cannot fail for these types —
/// a [`Value`] number can never be `NaN` and its maps are always string-keyed —
/// but the fallback is chosen so that even then the comparison stays deterministic
/// and symmetric rather than collapsing into argument order, which is the failure
/// being fixed.
fn content_order<T: Serialize>(a: &T, b: &T) -> Ordering {
    let bytes = |v: &T| serde_json::to_string(v).unwrap_or_default();
    bytes(a).cmp(&bytes(b))
}

/// Does `candidate` take the slot currently held by `held`? Newer `at` wins;
/// an exact tie is settled by [`content_order`], never by which side was
/// iterated first.
///
/// Shared by [`merge_history`] and [`merge_mylist`] so the two cannot drift: they
/// are the same CRDT over two different row types, and the only reason they are
/// two functions is that Rust has no anonymous "has an `at`" bound worth the
/// trait it would cost.
fn takes_the_slot<T: Serialize>(held: Option<(&T, u64)>, candidate: (&T, u64)) -> bool {
    match held {
        None => true,
        Some((prev, prev_at)) => match prev_at.cmp(&candidate.1) {
            Ordering::Greater => false,
            Ordering::Less => true,
            // Equal canonical bytes means the two records ARE equal, so keeping
            // the one already in hand is both free and symmetric.
            Ordering::Equal => content_order(prev, candidate.0) == Ordering::Less,
        },
    }
}

/// Merge two history lists: newest entry wins per id, an equal-`at` collision on
/// one id is settled by [`content_order`], tombstoned ids (where the tombstone is
/// at-or-after the entry) are dropped, newest first.
///
/// Uncapped — see [`Library::cap_history`]. Splitting them is what lets a caller
/// tell "you have 40 titles" from "you had 80 and we kept 60".
pub fn merge_history(a: &[LibraryItem], b: &[LibraryItem], tomb: &Tombstones) -> Vec<LibraryItem> {
    let mut by_id: BTreeMap<String, LibraryItem> = BTreeMap::new();
    for it in a.iter().chain(b.iter()) {
        let id = it.id().to_string();
        let held = by_id.get(&id).map(|prev| (prev, prev.at));
        if takes_the_slot(held, (it, it.at)) {
            by_id.insert(id, it.clone());
        }
    }
    let mut out: Vec<LibraryItem> = by_id
        .into_values()
        .filter(|it| tomb.get(it.id()).map(|&t| t < it.at).unwrap_or(true))
        .collect();
    out.sort_by(|x, y| y.at.cmp(&x.at).then_with(|| x.id().cmp(y.id())));
    out
}

/// Merge two My List lists — the same rule as [`merge_history`], including the
/// same [`content_order`] tie-break, over the other half of the document.
pub fn merge_mylist(a: &[MyListItem], b: &[MyListItem], tomb: &Tombstones) -> Vec<MyListItem> {
    let mut by_id: BTreeMap<String, MyListItem> = BTreeMap::new();
    for it in a.iter().chain(b.iter()) {
        let held = by_id.get(&it.id).map(|prev| (prev, prev.at));
        if takes_the_slot(held, (it, it.at)) {
            by_id.insert(it.id.clone(), it.clone());
        }
    }
    let mut out: Vec<MyListItem> = by_id
        .into_values()
        .filter(|it| tomb.get(&it.id).map(|&t| t < it.at).unwrap_or(true))
        .collect();
    out.sort_by(|x, y| y.at.cmp(&x.at).then_with(|| x.id.cmp(&y.id)));
    out
}

/// The order two resume positions are compared in when they claim the same `at`.
///
/// **Why there has to be one.** "Newer `at` wins, and on a tie the local side
/// keeps its record" is not a merge rule, it is an argument-order preference
/// dressed as one: device A merging B keeps A's position and device B merging A
/// keeps B's, so the two never converge and each keeps re-pushing its own answer
/// forever. Every other merge in this module was given a total order for exactly
/// this reason ("a value that gets serialised and compared may not depend on a
/// container's implementation detail"); progress was the one that was missed, and
/// commutativity is documented here as load-bearing.
///
/// **Why these fields, in this order.** A content hash would also converge, but it
/// would converge on an arbitrary record — half the time the *earlier* position —
/// and nobody could explain to a user why their episode jumped backwards. This
/// order is a rule instead:
///
/// 1. **`pos`, larger wins.** An equal `at` means two devices wrote in the same
///    millisecond, which in practice means one of them was mid-playback and the
///    other was echoing something older. Never rewind the person watching: of two
///    equally-recent claims, the further-along one is the one that has seen the
///    other's frames.
/// 2. **`dur`, larger wins.** A duration only grows as a player learns it; `0`
///    means "not known yet", and a known duration is what makes the record
///    resumable at all ([`Library::resumable`] refuses `dur <= 0`).
/// 3. **`lang`, `Some` beats `None`, then lexicographic.** A language that was
///    recorded beats one that never was — the same instinct as the carry rule
///    below — and comparing the strings makes the order *total* rather than
///    merely usually-decisive.
///
/// `total_cmp` rather than `partial_cmp`: a `NaN` position is nonsense a hostile
/// document can contain, and it must order deterministically instead of collapsing
/// the comparison to "neither wins", which is where the non-commutativity would
/// creep straight back in.
fn progress_tie_break(a: &Progress, b: &Progress) -> Ordering {
    a.pos
        .total_cmp(&b.pos)
        .then_with(|| a.dur.total_cmp(&b.dur))
        .then_with(|| a.lang.cmp(&b.lang))
}

/// Merge two progress maps: newer `at` wins per key, and an exact tie is broken by
/// [`progress_tie_break`] — never by which argument the record arrived in.
///
/// The loser is not discarded entirely — if the winner has no `lang` and the
/// loser does, the language is carried onto the winner. A record that has been
/// through a client too old to know about languages, or through the server (which
/// stores what it was given and invents nothing), otherwise erases a preference
/// the user set on the device they are holding. The carry is commutative too: the
/// winner is the same record either way round, so the language that rides along is
/// the same language either way round.
///
/// Uncapped — see [`Library::cap_progress`].
pub fn merge_progress(
    a: &BTreeMap<String, Progress>,
    b: &BTreeMap<String, Progress>,
) -> BTreeMap<String, Progress> {
    let mut out: BTreeMap<String, Progress> = BTreeMap::new();
    for (key, p) in a.iter().chain(b.iter()) {
        match out.get_mut(key) {
            Some(prev) => {
                let holder_wins = match prev.at.cmp(&p.at) {
                    Ordering::Greater => true,
                    Ordering::Less => false,
                    // `!= Less` rather than `== Greater`: when the order calls
                    // them equal the two records ARE equal (a `Progress` is
                    // exactly these four fields), so keeping the one already in
                    // hand is both free and symmetric.
                    Ordering::Equal => progress_tie_break(prev, p) != Ordering::Less,
                };
                if holder_wins {
                    if prev.lang.is_none() {
                        prev.lang = p.lang.clone();
                    }
                } else {
                    let carried = if p.lang.is_some() {
                        p.lang.clone()
                    } else {
                        prev.lang.clone()
                    };
                    *prev = p.clone();
                    prev.lang = carried;
                }
            }
            None => {
                out.insert(key.clone(), p.clone());
            }
        }
    }
    out
}

/// The titles a media key could belong to, cheapest-first: the whole key (a
/// film's position is filed under its bare id), then every
/// [`MEDIA_KEY_SEPARATOR`]-terminated prefix of it (`tt1:S1E4` belongs to `tt1`).
///
/// Enumerating the key's prefixes rather than testing every tombstone against
/// every key is not a micro-optimisation. The obvious spelling — for each
/// tombstoned `id`, `k == id || k.starts_with(&format!("{id}:"))` — is
/// `removed.len() * progress.len()` comparisons and one `String` allocation per
/// *pair*, on a function that runs on every boot and every pull, over a map capped
/// at 240 keys and a tombstone map capped at nothing at all. A media key has a
/// handful of separators at most, so this is a bounded number of `BTreeMap`
/// lookups per key however many titles the user has ever deleted — and it decides
/// exactly the same set.
///
/// The empty prefix is skipped. [`crate::api::library_remove`] refuses to
/// tombstone an empty id precisely because `""` would then own every key beginning
/// with a separator; a merge that honoured a `""` tombstone smuggled in by a
/// hostile document would reopen the hole the boundary closes.
fn owning_ids(key: &str) -> impl Iterator<Item = &str> {
    std::iter::once(key)
        .chain(
            key.match_indices(MEDIA_KEY_SEPARATOR)
                .filter_map(|(i, _)| key.get(..i)),
        )
        .filter(|id| !id.is_empty())
}

/// Drop every resume position whose owning title was removed at-or-after the
/// position was recorded.
///
/// **Why this cannot live in [`Library::remove`] alone.** `remove` deletes the
/// keys *on this device*. [`merge_progress`] then unions two maps unconditionally,
/// so the second device — which still holds `tt1:S1E1` because it has not synced
/// yet — hands every episode key straight back on the next pull, where it counts
/// against [`PROGRESS_CAP`] again and repopulates a rail the user cleared. A
/// deletion that any replica can undo is not a deletion; the tombstone has to be
/// the thing that decides, on every merge, for as long as it lives.
///
/// **Why at-or-after, and not unconditionally.** This is [`merge_history`]'s
/// existing keep-rule (`tomb < it.at` keeps the entry) applied to the other map,
/// and it has to be, or the two halves of one title disagree: a re-watch newer
/// than the removal already resurrects the *history entry*, so it must also keep
/// the *position*, or the title comes back to Continue Watching having forgotten
/// where the user was. Equal timestamps go to the removal — a delete and a tick in
/// the same millisecond is a delete of that tick.
///
/// **Why no wire change.** Nothing new is stored and nothing new is sent: the
/// tombstone that already travels in `removed` is simply read by one more merge
/// step. An older client that has never heard of this rule still converges, because
/// the rule is a function of state both ends already exchange.
///
/// Commutative and idempotent by construction — it is a filter over the *merged*
/// tombstone map, so it cannot see argument order, and a second application has
/// nothing left to remove.
pub fn sweep_removed_progress(progress: &mut BTreeMap<String, Progress>, tomb: &Tombstones) {
    progress.retain(|key, p| {
        !owning_ids(key).any(|id| tomb.get(id).map(|&t| t >= p.at).unwrap_or(false))
    });
}

/// Merge a whole local library with a whole remote one.
///
/// This is the body of the boundary's `merge_library`, and it is the *only* place
/// the four hand-written copies of this algorithm collapse into.
///
/// **Neither side is authoritative**, and that is the property the whole
/// device-sync model rests on: `merge_libraries(a, b)` and `merge_libraries(b, a)`
/// produce the same bytes, so two devices that pull from each other converge in
/// one round instead of trading answers forever. Ties are settled by the data
/// (`(at DESC, id ASC)` for list order, [`content_order`] for a same-id list
/// collision, [`progress_tie_break`] for resume positions), never by argument
/// order.
///
/// The tombstone map is applied to **both** halves of the document, not just the
/// lists: [`sweep_removed_progress`] is what stops a removal being undone by the
/// first replica that syncs an older copy of the episode keys back.
pub fn merge_libraries(local: &Library, remote: &Library, now: u64) -> LibraryMerge {
    let mut removed = union_tombstones(&local.removed, &remote.removed);
    let mut pruned = prune_tombstones(&mut removed, now);

    let mut mylist_removed = union_tombstones(&local.mylist_removed, &remote.mylist_removed);
    pruned.extend(prune_tombstones(&mut mylist_removed, now));

    // Swept before the cap runs, so `truncated_progress` counts what the cap
    // actually ate rather than including rows the tombstones were about to take
    // anyway — a warning that over-reports is a warning the shell learns to
    // ignore. `removed` only: a My List tombstone says nothing about where the
    // user is in a title, and `mylist_removed` sweeping resume positions would
    // make un-listing a film silently forget it.
    let mut progress = merge_progress(&local.progress, &remote.progress);
    sweep_removed_progress(&mut progress, &removed);

    let mut library = Library {
        history: merge_history(&local.history, &remote.history, &removed),
        progress,
        mylist: merge_mylist(&local.mylist, &remote.mylist, &mylist_removed),
        removed,
        mylist_removed,
    };

    let report = LibraryMergeReport {
        truncated_history: library.cap_history(),
        truncated_progress: library.cap_progress(),
        truncated_mylist: library.cap_mylist(),
        pruned_tombstones: pruned,
    };

    LibraryMerge { library, report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MetaItem;

    fn item(id: &str, at: u64) -> LibraryItem {
        LibraryItem {
            meta: MetaItem {
                id: id.to_string(),
                title: id.to_string(),
                ..Default::default()
            },
            at,
            ..Default::default()
        }
    }

    fn prog(pos: f64, dur: f64, at: u64, lang: Option<&str>) -> Progress {
        Progress {
            pos,
            dur,
            at,
            lang: lang.map(str::to_string),
        }
    }

    fn listed(id: &str, at: u64) -> MyListItem {
        MyListItem {
            id: id.to_string(),
            at,
            ..Default::default()
        }
    }

    // ---- ported from Heart, with `update(&mut m, LibMsg::…)` inlined ----

    #[test]
    fn record_watch_upserts_and_orders() {
        let mut m = Library::default();
        m.record_watch(item("a", 100));
        m.record_watch(item("b", 200));
        m.record_watch(item("a", 300)); // re-watch a -> moves to front, no dup
        let ids: Vec<&str> = m.history.iter().map(|i| i.id()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(m.history[0].at, 300);
    }

    #[test]
    fn set_progress_and_continue_watching() {
        let mut m = Library::default();
        m.record_watch(item("a", 100));
        m.record_watch(item("b", 90));
        m.set_progress("a", 30.0, 100.0, 110, None);
        m.set_progress("b", 99.0, 100.0, 95, None); // finished
        let cw: Vec<&str> = m.continue_watching(true).iter().map(|i| i.id()).collect();
        assert_eq!(cw, vec!["a"]); // b is finished -> excluded
        assert_eq!(m.resume("a").unwrap().pos, 30.0);
    }

    #[test]
    fn remove_tombstones_and_drops() {
        let mut m = Library::default();
        m.record_watch(item("a", 100));
        m.set_progress("a", 5.0, 100.0, 100, None);
        m.remove("a", 200);
        assert!(m.is_removed("a"));
        assert!(m.history.is_empty());
        assert!(m.resume("a").is_none());
    }

    #[test]
    fn pull_merges_by_recency_and_respects_tombstones() {
        let mut local = Library::default();
        local.record_watch(item("a", 100));
        // remote has a newer "a" and a new "b", but we tombstoned "b" later
        local.remove("b", 500);

        let remote = Library {
            history: vec![item("a", 300), item("b", 200)],
            progress: [("a".to_string(), prog(50.0, 100.0, 300, None))].into(),
            ..Default::default()
        };

        let merged = merge_libraries(&local, &remote, 600).library;
        // "a" adopted at newer at; "b" stays tombstoned (removed at 500 > entry at 200)
        assert_eq!(merged.history.len(), 1);
        assert_eq!(merged.history[0].id(), "a");
        assert_eq!(merged.history[0].at, 300);
        assert_eq!(merged.resume("a").unwrap().pos, 50.0);
    }

    #[test]
    fn tombstone_ttl_prunes_old() {
        let mut old = Tombstones::new();
        old.insert("x".into(), 1000);
        let merged = merge_tombstones(&old, &Tombstones::new(), 1000 + TOMB_TTL_MS + 1);
        assert!(merged.is_empty()); // expired
        let fresh = merge_tombstones(&old, &Tombstones::new(), 1000 + 5);
        assert_eq!(fresh.get("x"), Some(&1000));
    }

    #[test]
    fn history_cap_enforced() {
        let mut m = Library::default();
        for i in 0..(HISTORY_CAP as u64 + 10) {
            m.record_watch(item(&format!("id{i}"), i));
        }
        assert_eq!(m.history.len(), HISTORY_CAP);
        assert_eq!(m.history[0].at, HISTORY_CAP as u64 + 9); // newest kept
    }

    // ---- new: the corrections this module makes ----

    /// `Progress.lang` must survive a full merge in both directions. This is the
    /// field whose absence `history.ts::keepLangs()` exists to work around.
    #[test]
    fn progress_lang_survives_a_merge() {
        let local = Library {
            progress: [("k".to_string(), prog(10.0, 100.0, 200, Some("eng")))].into(),
            ..Default::default()
        };
        let remote = Library {
            progress: [("k".to_string(), prog(50.0, 100.0, 100, None))].into(),
            ..Default::default()
        };

        let out = merge_libraries(&local, &remote, 1000).library;
        let p = out.resume("k").unwrap();
        assert_eq!(p.pos, 10.0); // local is newer, local wins
        assert_eq!(p.lang.as_deref(), Some("eng"));
    }

    /// The rule that makes `keepLangs` deletable rather than merely relocated: a
    /// newer record with no language inherits the older record's, instead of
    /// erasing it.
    #[test]
    fn newer_record_without_lang_inherits_the_older_one() {
        let local = Library {
            progress: [("k".to_string(), prog(10.0, 100.0, 100, Some("jpn")))].into(),
            ..Default::default()
        };
        let remote = Library {
            progress: [("k".to_string(), prog(80.0, 100.0, 900, None))].into(),
            ..Default::default()
        };

        let out = merge_libraries(&local, &remote, 1000).library;
        let p = out.resume("k").unwrap();
        assert_eq!(p.pos, 80.0); // remote is newer, remote's position wins
        assert_eq!(p.lang.as_deref(), Some("jpn")); // …but the language carries
    }

    /// A tick that supplies no language keeps the one already stored, matching
    /// `history.ts`'s `lang ?? map[key]?.lang`.
    #[test]
    fn set_progress_keeps_a_language_it_was_not_given() {
        let mut m = Library::default();
        m.set_progress("k", 10.0, 100.0, 1, Some("eng".into()));
        m.set_progress("k", 20.0, 100.0, 2, None);
        assert_eq!(m.resume("k").unwrap().lang.as_deref(), Some("eng"));
        m.set_progress("k", 30.0, 100.0, 3, Some("fra".into()));
        assert_eq!(m.resume("k").unwrap().lang.as_deref(), Some("fra"));
    }

    /// D1: an exact `at` tie resolves by the DATA, identically from either side.
    ///
    /// The old rule ("local keeps the tie") is what made this non-commutative:
    /// each device kept its own record and neither ever adopted the other's.
    #[test]
    fn progress_tie_resolves_to_the_further_position_from_either_side() {
        let local = Library {
            progress: [("k".to_string(), prog(1.0, 100.0, 500, None))].into(),
            ..Default::default()
        };
        let remote = Library {
            progress: [("k".to_string(), prog(2.0, 100.0, 500, None))].into(),
            ..Default::default()
        };
        let ab = merge_libraries(&local, &remote, 1000).library;
        let ba = merge_libraries(&remote, &local, 1000).library;
        assert_eq!(ab.resume("k").unwrap().pos, 2.0, "never rewind the viewer");
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap(),
            "a tie must not depend on which device started the sync"
        );
    }

    /// The rest of the tie order, each level reached only when the one above it is
    /// equal — and each asserted in both argument orders, because a tiebreaker that
    /// is only commutative for its first field is not a tiebreaker.
    #[test]
    fn the_progress_tie_order_is_total_and_symmetric() {
        // (the record that must lose, the record that must win)
        let cases = [
            // dur decides when pos is equal: a known duration beats "not known yet".
            (prog(5.0, 0.0, 500, None), prog(5.0, 100.0, 500, None)),
            // lang decides when pos and dur are equal: recorded beats absent.
            (
                prog(5.0, 100.0, 500, None),
                prog(5.0, 100.0, 500, Some("eng")),
            ),
        ];
        for (x, y) in cases {
            let one = Library {
                progress: [("k".to_string(), x.clone())].into(),
                ..Default::default()
            };
            let two = Library {
                progress: [("k".to_string(), y.clone())].into(),
                ..Default::default()
            };
            let ab = merge_libraries(&one, &two, 1000).library;
            let ba = merge_libraries(&two, &one, 1000).library;
            assert_eq!(ab.resume("k"), Some(y.clone()), "the declared order lost");
            assert_eq!(
                serde_json::to_string(&ab).unwrap(),
                serde_json::to_string(&ba).unwrap()
            );
        }
    }

    /// A `NaN` position is nonsense a hostile document can carry, and the one input
    /// on which `partial_cmp` would quietly reintroduce argument-order dependence.
    #[test]
    fn a_nan_position_still_merges_commutatively() {
        let a = Library {
            progress: [("k".to_string(), prog(f64::NAN, 100.0, 500, None))].into(),
            ..Default::default()
        };
        let b = Library {
            progress: [("k".to_string(), prog(50.0, 100.0, 500, None))].into(),
            ..Default::default()
        };
        let ab = merge_libraries(&a, &b, 1000).library;
        let ba = merge_libraries(&b, &a, 1000).library;
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap()
        );
    }

    /// D2: equal-`at` history entries come out in id order, from either argument
    /// order. Commutativity is the property the total order buys — assert it.
    #[test]
    fn history_ties_are_ordered_and_commutative() {
        let a = Library {
            history: vec![item("zulu", 100), item("alpha", 100)],
            ..Default::default()
        };
        let b = Library {
            history: vec![item("mike", 100)],
            ..Default::default()
        };

        let ab = merge_libraries(&a, &b, 1000).library;
        let ba = merge_libraries(&b, &a, 1000).library;
        let ids: Vec<&str> = ab.history.iter().map(|i| i.id()).collect();
        assert_eq!(ids, vec!["alpha", "mike", "zulu"]);
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap()
        );
    }

    /// D3: removing a series takes its episodes' resume positions with it. Under
    /// Heart these survived forever, counted against the cap, and came back if the
    /// title was ever re-watched.
    #[test]
    fn remove_sweeps_episode_progress_keys() {
        let mut m = Library::default();
        m.set_progress("tt1", 5.0, 100.0, 1, None);
        m.set_progress("tt1:S1E1", 10.0, 100.0, 2, None);
        m.set_progress("tt1:S2E9", 20.0, 100.0, 3, None);
        m.set_progress("tt10", 30.0, 100.0, 4, None); // a *different* title
        m.set_progress("tt10:S1E1", 40.0, 100.0, 5, None);

        m.remove("tt1", 99);

        let keys: Vec<&str> = m.progress.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["tt10", "tt10:S1E1"]);
    }

    /// D5: the resume window is [0.01, 0.94]. Heart's 0.9 is gone.
    #[test]
    fn resumable_window() {
        let mut m = Library::default();
        m.progress
            .insert("just_started".into(), prog(0.5, 100.0, 1, None));
        m.progress.insert("mid".into(), prog(50.0, 100.0, 1, None));
        m.progress
            .insert("near_end".into(), prog(95.0, 100.0, 1, None));
        m.progress
            .insert("old_cutoff".into(), prog(92.0, 100.0, 1, None));
        m.progress.insert("no_dur".into(), prog(10.0, 0.0, 1, None));

        assert!(m.resumable("just_started").is_none());
        assert!(m.resumable("mid").is_some());
        assert!(m.resumable("near_end").is_none());
        // 0.92 sits between Heart's 0.9 and the shell's 0.94 — the case that pins
        // which threshold actually won.
        assert!(m.resumable("old_cutoff").is_some());
        assert!(m.resumable("no_dur").is_none());
        assert!(m.resumable("absent").is_none());
    }

    #[test]
    fn mylist_toggle_round_trip() {
        let mut m = Library::default();
        assert!(m.mylist_toggle(listed("tt1", 0), 100));
        assert!(m.in_list("tt1"));
        assert_eq!(m.mylist[0].at, 100); // `at` is the toggle's clock, not the caller's
        assert!(!m.mylist_removed.contains_key("tt1"));

        assert!(!m.mylist_toggle(listed("tt1", 0), 200));
        assert!(!m.in_list("tt1"));
        assert_eq!(m.mylist_removed.get("tt1"), Some(&200)); // tombstoned

        assert!(m.mylist_toggle(listed("tt1", 0), 300));
        assert!(!m.mylist_removed.contains_key("tt1")); // re-adding clears it
    }

    #[test]
    fn mylist_merges_and_respects_its_own_tombstones() {
        let local = Library {
            mylist: vec![listed("a", 100)],
            mylist_removed: [("b".to_string(), 500)].into(),
            ..Default::default()
        };
        let remote = Library {
            mylist: vec![listed("a", 300), listed("b", 200)],
            ..Default::default()
        };

        let out = merge_libraries(&local, &remote, 600).library;
        let ids: Vec<&str> = out.mylist.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
        assert_eq!(out.mylist[0].at, 300);
    }

    /// A partial document — the shape each store actually PUTs — must contribute
    /// its half and leave the other half alone, never wipe it.
    #[test]
    fn a_partial_document_contributes_rather_than_wipes() {
        let local = Library {
            history: vec![item("a", 100)],
            mylist: vec![listed("m", 100)],
            ..Default::default()
        };

        // What stores/library.ts pushes: My List only, no history key at all.
        let remote: Library = serde_json::from_str(r#"{"mylist":[{"id":"n","at":200}]}"#).unwrap();

        let out = merge_libraries(&local, &remote, 1000).library;
        assert_eq!(out.history.len(), 1, "history must survive a My List push");
        let ids: Vec<&str> = out.mylist.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["n", "m"]);
    }

    /// The single most damaging failure this module exists to close: one row the
    /// core cannot read must cost that row, not the library.
    #[test]
    fn one_unreadable_row_does_not_wipe_the_document() {
        let lib: Library = serde_json::from_str(
            r#"{"history":[{"id":"good","at":2},{"no_id_here":true},{"id":"also_good","at":1}],
                "progress":{"good":{"pos":1,"dur":2,"at":3},"broken":null},
                "removed":{"gone":123}}"#,
        )
        .unwrap();
        let ids: Vec<&str> = lib.history.iter().map(|i| i.id()).collect();
        assert_eq!(ids, vec!["good", "also_good"]);
        assert_eq!(lib.progress.keys().collect::<Vec<_>>(), vec!["good"]);
        assert_eq!(lib.removed.get("gone"), Some(&123));
    }

    /// …and now it costs that row OUT LOUD. Same document as above, same result,
    /// with the account the shell needs to stop persisting the loss as truth.
    #[test]
    fn parse_library_names_every_row_it_drops() {
        let (lib, warnings) = parse_library(
            r#"{"history":[{"id":"good","at":2},{"no_id_here":true},{"id":603,"at":1}],
                "progress":{"good":{"pos":1,"dur":2,"at":3},"broken":null},
                "removed":{"gone":123}}"#,
        )
        .unwrap();
        let ids: Vec<&str> = lib.history.iter().map(|i| i.id()).collect();
        assert_eq!(ids, vec!["good", "603"]);

        let seen: Vec<(&str, &str)> = warnings
            .iter()
            .map(|w| (w.code.as_str(), w.subject.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("dropped.bad_item", "history[1]"),
                ("coerced.field", "history[2]"),
                ("dropped.bad_item", "progress[broken]"),
            ],
            "every row that did not survive verbatim must be named"
        );
    }

    /// The drift guard. Two implementations of one leniency rule is a bug waiting
    /// for someone to fix only one of them, so the reporting parse and the silent
    /// derive are compared over the documents that actually exercise the
    /// difference — not asserted to be equal by inspection.
    #[test]
    fn parse_library_agrees_with_the_derive() {
        let hostile = [
            r#"{}"#,
            r#"{"history":[{"id":"a","at":1}]}"#,
            r#"{"history":[{"id":"a","at":1},{"garbage":true},{"id":603,"at":2}]}"#,
            r#"{"progress":{"a":{"pos":1,"dur":2,"at":3},"b":null,"c":"nonsense"}}"#,
            r#"{"history":null,"progress":null,"removed":null,"mylist":null,"mylistRemoved":null}"#,
            r#"{"removed":{"x":50,"y":"no"},"mylistRemoved":{"z":1}}"#,
            r#"{"mylist":[{"id":"m","at":1},{"id":null}],"unknownKey":{"nested":true}}"#,
            r#"{"history":[{"id":"a","at":null,"poster":null,"season":null}]}"#,
            // Not a mistake and not an endorsement: serde's derived impl accepts a
            // struct in SEQUENCE form, so `[]` is a library with every field
            // defaulted. Pinned because the reporting parse hands non-objects to
            // that same impl, and the two must agree on the odd cases too.
            "[]",
        ];
        for doc in hostile {
            let (reported, _) = parse_library(doc).unwrap_or_else(|e| panic!("{doc}: {e}"));
            let derived: Library =
                serde_json::from_str(doc).unwrap_or_else(|e| panic!("{doc}: {e}"));
            assert_eq!(reported, derived, "{doc}");
        }

        // …and the document-level failures have to be failures on both paths too,
        // or the boundary's "never wipe, choose the other side" contract silently
        // becomes "return an empty library".
        for doc in [
            "not json",
            "42",
            "null",
            r#""a string""#,
            r#"{"history":42}"#,
            r#"{"progress":[]}"#,
        ] {
            assert!(parse_library(doc).is_err(), "{doc}");
            assert!(serde_json::from_str::<Library>(doc).is_err(), "{doc}");
        }
    }

    /// A numeric id — the realistic trigger for the above, straight out of a
    /// third-party add-on's catalog.
    #[test]
    fn numeric_ids_and_years_survive() {
        let lib: Library = serde_json::from_str(
            r#"{"history":[{"id":603,"title":"The Matrix","year":1999,"rating":8.7,"at":5}]}"#,
        )
        .unwrap();
        assert_eq!(lib.history.len(), 1);
        assert_eq!(lib.history[0].id(), "603");
        assert_eq!(lib.history[0].meta.year.as_deref(), Some("1999"));
        assert_eq!(lib.history[0].meta.rating.as_deref(), Some("8.7"));
    }

    #[test]
    fn caps_report_what_they_dropped() {
        let mut local = Library::default();
        for i in 0..(HISTORY_CAP as u64 + 5) {
            local.history.push(item(&format!("h{i:03}"), i));
        }
        for i in 0..(PROGRESS_CAP as u64 + 3) {
            local
                .progress
                .insert(format!("p{i:03}"), prog(1.0, 100.0, i, None));
        }
        for i in 0..(MYLIST_CAP as u64 + 7) {
            local.mylist.push(listed(&format!("m{i:03}"), i));
        }
        let merged = merge_libraries(&local, &Library::default(), u64::MAX / 2);
        assert_eq!(merged.report.truncated_history, 5);
        assert_eq!(merged.report.truncated_progress, 3);
        assert_eq!(merged.report.truncated_mylist, 7);
        assert_eq!(merged.library.history.len(), HISTORY_CAP);
        assert_eq!(merged.library.progress.len(), PROGRESS_CAP);
        assert_eq!(merged.library.mylist.len(), MYLIST_CAP);
    }

    #[test]
    fn expired_tombstones_are_reported_not_just_dropped() {
        let mut local = Library::default();
        local.removed.insert("stale".into(), 1);
        local.mylist_removed.insert("stale_list".into(), 1);
        local.removed.insert("fresh".into(), 1_000_000_000);

        let merged = merge_libraries(&local, &Library::default(), 1_000_000_000 + TOMB_TTL_MS);
        let mut pruned = merged.report.pruned_tombstones.clone();
        pruned.sort();
        assert_eq!(pruned, vec!["stale", "stale_list"]);
        assert!(merged.library.removed.contains_key("fresh"));
    }

    // ---- D8: the removal survives the next sync ----

    /// The user's bug report, in four lines: delete a series on the phone, open
    /// the TV — which has not synced and still holds every episode key — and the
    /// resume positions are back. `remove` deleted them locally; `merge_progress`
    /// unions, so the union handed them straight back.
    #[test]
    fn a_removed_titles_episode_progress_does_not_come_back_on_the_next_sync() {
        let mut local = Library::default();
        local.set_progress("tt1", 5.0, 100.0, 100, None);
        local.set_progress("tt1:S1E1", 10.0, 100.0, 100, None);
        local.record_watch(item("tt1", 100));
        local.remove("tt1", 200);

        // The device that has not seen the removal yet.
        let remote = Library {
            history: vec![item("tt1", 100)],
            progress: [
                ("tt1".to_string(), prog(5.0, 100.0, 100, None)),
                ("tt1:S1E1".to_string(), prog(10.0, 100.0, 100, None)),
                ("tt1:S2E9".to_string(), prog(20.0, 100.0, 150, None)),
            ]
            .into(),
            ..Default::default()
        };

        let ab = merge_libraries(&local, &remote, 1000).library;
        assert!(
            ab.progress.is_empty(),
            "{:?} survived the removal",
            ab.progress
        );
        assert!(ab.history.is_empty());

        let ba = merge_libraries(&remote, &local, 1000).library;
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap(),
            "which device initiated the sync must not decide what got deleted"
        );
    }

    /// …and it must not come back on the sync after that either. The tombstone,
    /// not the local delete, is what does the work — so re-merging the already
    /// merged document against the stale device is still a no-op.
    #[test]
    fn a_swept_position_stays_gone_on_every_later_round() {
        let local = Library {
            removed: [("tt1".to_string(), 200)].into(),
            ..Default::default()
        };
        let stale = Library {
            progress: [("tt1:S1E1".to_string(), prog(10.0, 100.0, 100, None))].into(),
            ..Default::default()
        };

        let once = merge_libraries(&local, &stale, 1000).library;
        assert!(once.progress.is_empty());
        let twice = merge_libraries(&once, &stale, 1000).library;
        assert!(twice.progress.is_empty(), "the union let it back in");
    }

    /// The rule is at-or-after, not unconditional, and this is why: a re-watch
    /// newer than the removal already resurrects the history entry, so it has to
    /// keep the position too — or the title returns to Continue Watching having
    /// forgotten where the user was. Mirrors `merge_history`'s `tomb < it.at`.
    #[test]
    fn a_re_watch_newer_than_the_removal_keeps_its_position() {
        let local = Library {
            removed: [("tt1".to_string(), 200)].into(),
            ..Default::default()
        };
        let remote = Library {
            history: vec![item("tt1", 300)],
            progress: [
                // after the removal — survives, exactly as the history entry does
                ("tt1:S1E1".to_string(), prog(10.0, 100.0, 300, None)),
                // before it — swept
                ("tt1:S2E9".to_string(), prog(20.0, 100.0, 100, None)),
                // in the same millisecond — a delete beats a tick it ties with
                ("tt1".to_string(), prog(1.0, 100.0, 200, None)),
            ]
            .into(),
            ..Default::default()
        };

        let out = merge_libraries(&local, &remote, 1000).library;
        let keys: Vec<&str> = out.progress.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["tt1:S1E1"]);
        assert_eq!(
            out.history.len(),
            1,
            "the entry came back, so must the position"
        );
    }

    /// The sweep takes what the tombstone owns and nothing adjacent to it. `tt1`
    /// must not eat `tt10`, and a tombstone on the *episode* key is not a
    /// tombstone on the series.
    #[test]
    fn the_sweep_only_takes_keys_its_tombstone_owns() {
        let removed: Tombstones = [("tt1".to_string(), 500)].into();
        let mut progress: BTreeMap<String, Progress> = [
            ("tt1".to_string(), prog(1.0, 100.0, 100, None)),
            ("tt1:S1E1".to_string(), prog(2.0, 100.0, 100, None)),
            ("tt1:S1E1:extra".to_string(), prog(3.0, 100.0, 100, None)),
            ("tt10".to_string(), prog(4.0, 100.0, 100, None)),
            ("tt10:S1E1".to_string(), prog(5.0, 100.0, 100, None)),
            ("tt1x".to_string(), prog(6.0, 100.0, 100, None)),
        ]
        .into();

        sweep_removed_progress(&mut progress, &removed);
        let keys: Vec<&str> = progress.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["tt10", "tt10:S1E1", "tt1x"]);
    }

    /// A deeper key is owned by every separator-terminated prefix of itself, so a
    /// tombstone on an id that itself contains a separator still sweeps.
    #[test]
    fn a_tombstone_on_a_compound_id_sweeps_the_keys_under_it() {
        let removed: Tombstones = [("tt1:2".to_string(), 500)].into();
        let mut progress: BTreeMap<String, Progress> = [
            ("tt1:2".to_string(), prog(1.0, 100.0, 100, None)),
            ("tt1:2:S1E1".to_string(), prog(2.0, 100.0, 100, None)),
            ("tt1:3".to_string(), prog(3.0, 100.0, 100, None)),
        ]
        .into();

        sweep_removed_progress(&mut progress, &removed);
        let keys: Vec<&str> = progress.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["tt1:3"]);
    }

    /// An empty id owns nothing. `api::library_remove` refuses to create this
    /// tombstone for exactly this reason; a hostile document must not be able to
    /// smuggle one in and wipe every episode key on the device.
    #[test]
    fn an_empty_id_tombstone_sweeps_nothing() {
        let removed: Tombstones = [(String::new(), u64::MAX)].into();
        let before: BTreeMap<String, Progress> = [
            ("tt1".to_string(), prog(1.0, 100.0, 100, None)),
            ("tt1:S1E1".to_string(), prog(2.0, 100.0, 100, None)),
            (":S1E1".to_string(), prog(3.0, 100.0, 100, None)),
        ]
        .into();
        let mut progress = before.clone();

        sweep_removed_progress(&mut progress, &removed);
        assert_eq!(progress, before);
    }

    /// An expired tombstone sweeps nothing either — the prune runs first, so the
    /// two halves of the document forget a removal on the same beat.
    #[test]
    fn an_expired_tombstone_lets_a_position_return() {
        let local = Library {
            removed: [("tt1".to_string(), 1000)].into(),
            ..Default::default()
        };
        let remote = Library {
            progress: [("tt1:S1E1".to_string(), prog(10.0, 100.0, 500, None))].into(),
            ..Default::default()
        };
        let out = merge_libraries(&local, &remote, 1000 + TOMB_TTL_MS + 1).library;
        assert!(out.removed.is_empty());
        assert_eq!(out.progress.len(), 1, "nothing left to enforce the removal");
    }

    // ---- D7: a same-id, equal-`at` collision converges on content ----

    /// Two devices, one id, the same millisecond, different content. Under
    /// `prev.at >= it.at` each side kept its own row forever; now both sides pick
    /// the same one.
    #[test]
    fn a_same_id_equal_at_history_collision_converges() {
        let mut mine = item("tt1", 100);
        mine.meta.title = "Alpha".into();
        let mut theirs = item("tt1", 100);
        theirs.meta.title = "Beta".into();

        let a = Library {
            history: vec![mine],
            ..Default::default()
        };
        let b = Library {
            history: vec![theirs],
            ..Default::default()
        };

        let ab = merge_libraries(&a, &b, 1000).library;
        let ba = merge_libraries(&b, &a, 1000).library;
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap(),
            "the row's CONTENT must not depend on which device merged"
        );
        // Greater canonical bytes wins, and `"title":"Beta"` > `"title":"Alpha"`.
        // Asserted by value, not just by symmetry: a tie-break that always
        // returned `Equal` would satisfy the assertion above and still be the bug.
        assert_eq!(ab.history[0].meta.title, "Beta");
    }

    /// The same hole in the other half of the document.
    #[test]
    fn a_same_id_equal_at_mylist_collision_converges() {
        let mut mine = listed("tt1", 100);
        mine.poster = Some("alpha.jpg".into());
        let mut theirs = listed("tt1", 100);
        theirs.poster = Some("beta.jpg".into());

        let a = Library {
            mylist: vec![mine],
            ..Default::default()
        };
        let b = Library {
            mylist: vec![theirs],
            ..Default::default()
        };

        let ab = merge_libraries(&a, &b, 1000).library;
        let ba = merge_libraries(&b, &a, 1000).library;
        assert_eq!(
            serde_json::to_string(&ab).unwrap(),
            serde_json::to_string(&ba).unwrap()
        );
        assert_eq!(ab.mylist[0].poster.as_deref(), Some("beta.jpg"));
    }

    /// The tie-break must fire only on an exact `at` tie — a newer record still
    /// wins outright even when its bytes sort lower. Otherwise "greater content"
    /// would quietly become "greater content, recency ignored".
    #[test]
    fn content_order_never_overrules_recency() {
        let mut older = item("tt1", 100);
        older.meta.title = "Zulu".into(); // sorts high…
        let mut newer = item("tt1", 200);
        newer.meta.title = "Alpha".into(); // …but this one is newer

        let a = Library {
            history: vec![older],
            ..Default::default()
        };
        let b = Library {
            history: vec![newer],
            ..Default::default()
        };
        for (x, y) in [(&a, &b), (&b, &a)] {
            let out = merge_libraries(x, y, 1000).library;
            assert_eq!(out.history[0].meta.title, "Alpha");
            assert_eq!(out.history[0].at, 200);
        }
    }

    /// Three colliding copies, not two: "keep the greater" has to be a real max
    /// over a total order, or the answer depends on the order the fold saw them.
    #[test]
    fn a_three_way_content_collision_has_one_answer() {
        let titled = |t: &str| {
            let mut it = item("tt1", 100);
            it.meta.title = t.to_string();
            it
        };
        let permutations = [
            ["Alpha", "Beta", "Gamma"],
            ["Gamma", "Alpha", "Beta"],
            ["Beta", "Gamma", "Alpha"],
        ];
        for p in permutations {
            let a = Library {
                history: vec![titled(p[0]), titled(p[1])],
                ..Default::default()
            };
            let b = Library {
                history: vec![titled(p[2])],
                ..Default::default()
            };
            let out = merge_libraries(&a, &b, 1000).library;
            assert_eq!(out.history.len(), 1);
            assert_eq!(out.history[0].meta.title, "Gamma", "{p:?}");
        }
    }

    // ---- the property both fixes exist to protect ----

    /// A deterministic xorshift, so a failure is reproducible from the seed alone.
    /// A dev-dependency would buy a shrinker and cost the crate's
    /// two-dependency budget, which is a rule the workspace manifest calls a
    /// review failure to break.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            &xs[(self.next() as usize) % xs.len()]
        }
        fn chance(&mut self, n: u64) -> bool {
            self.next().is_multiple_of(n)
        }
    }

    /// Commutativity and idempotence over generated libraries, with the generator
    /// **deliberately biased toward the two cases these fixes exist for**: the
    /// alphabets are tiny, so same-id/equal-`at`/different-content collisions and
    /// tombstones that outrank a live progress key are the common case rather than
    /// a one-in-a-million draw. A generator that avoided them would pass against
    /// the bugs.
    ///
    /// The coverage counters are the point of the test as much as the assertions
    /// are: if a later change makes those inputs unreachable, this fails loudly
    /// instead of quietly testing nothing.
    #[test]
    fn merging_is_commutative_and_idempotent_over_generated_libraries() {
        const IDS: [&str; 3] = ["tt1", "tt2", "tt1:2"];
        const KEYS: [&str; 6] = ["tt1", "tt1:S1E1", "tt2", "tt2:S1E1", "tt1:2", "tt1:2:S1E1"];
        const ATS: [u64; 3] = [100, 200, 300];
        const TITLES: [&str; 3] = ["Alpha", "Beta", "Gamma"];
        const LANGS: [Option<&str>; 3] = [None, Some("eng"), Some("jpn")];
        const NOW: u64 = 1_000_000;

        let mut rng = Rng(0x5EED_1A2B_3C4D_5E6F);
        let gen = |rng: &mut Rng| {
            let mut lib = Library::default();
            for _ in 0..4 {
                if rng.chance(2) {
                    let mut it = item(rng.pick(&IDS), *rng.pick(&ATS));
                    it.meta.title = rng.pick(&TITLES).to_string();
                    lib.history.push(it);
                }
                if rng.chance(2) {
                    let mut m = listed(rng.pick(&IDS), *rng.pick(&ATS));
                    m.title = Some(rng.pick(&TITLES).to_string());
                    lib.mylist.push(m);
                }
                if rng.chance(2) {
                    lib.progress.insert(
                        rng.pick(&KEYS).to_string(),
                        prog(
                            (rng.next() % 100) as f64,
                            100.0,
                            *rng.pick(&ATS),
                            *rng.pick(&LANGS),
                        ),
                    );
                }
                if rng.chance(3) {
                    lib.removed
                        .insert(rng.pick(&IDS).to_string(), *rng.pick(&ATS));
                }
                if rng.chance(4) {
                    lib.mylist_removed
                        .insert(rng.pick(&IDS).to_string(), *rng.pick(&ATS));
                }
            }
            lib
        };

        let canon = |l: &Library| serde_json::to_string(l).unwrap();
        let (mut saw_content_collision, mut saw_sweep) = (0u32, 0u32);

        for case in 0..3000 {
            let a = gen(&mut rng);
            let b = gen(&mut rng);

            // Did this case actually exercise fix 2? A same id at the same `at`
            // with different bytes, one on each side.
            for x in a.history.iter() {
                for y in b.history.iter() {
                    if x.id() == y.id()
                        && x.at == y.at
                        && serde_json::to_string(x).unwrap() != serde_json::to_string(y).unwrap()
                    {
                        saw_content_collision += 1;
                    }
                }
            }
            // …and fix 1? A key the union would keep that the tombstones take.
            let unioned = merge_progress(&a.progress, &b.progress);
            let mut swept = unioned.clone();
            sweep_removed_progress(&mut swept, &union_tombstones(&a.removed, &b.removed));
            if swept.len() < unioned.len() {
                saw_sweep += 1;
            }

            let ab = merge_libraries(&a, &b, NOW).library;
            let ba = merge_libraries(&b, &a, NOW).library;
            assert_eq!(
                canon(&ab),
                canon(&ba),
                "case {case}: not commutative\nA={}\nB={}",
                canon(&a),
                canon(&b)
            );

            // Idempotence in the shape a device actually re-runs it: the merged
            // document pulled against the same remote must not move again.
            let again = merge_libraries(&ab, &b, NOW).library;
            assert_eq!(canon(&ab), canon(&again), "case {case}: not idempotent");

            // And the deletion has to be durable across the round trip, which is
            // the property idempotence alone does not state.
            for (key, p) in ab.progress.iter() {
                for id in owning_ids(key) {
                    if let Some(&t) = ab.removed.get(id) {
                        assert!(t < p.at, "case {case}: {key} outlived its tombstone");
                    }
                }
            }
        }

        assert!(
            saw_content_collision > 0,
            "the generator stopped producing same-id equal-at content collisions — \
             this test would now pass against the bug it exists for"
        );
        assert!(
            saw_sweep > 0,
            "the generator stopped producing tombstoned progress keys — \
             this test would now pass against the bug it exists for"
        );
    }

    /// Merging is idempotent and commutative — the two properties the whole
    /// device-sync model rests on. If either fails, two devices that pull from
    /// each other never converge.
    #[test]
    fn merge_is_idempotent_and_commutative() {
        let a = Library {
            history: vec![item("x", 300), item("y", 100)],
            progress: [("x".to_string(), prog(10.0, 100.0, 300, Some("eng")))].into(),
            mylist: vec![listed("w", 50)],
            removed: [("z".to_string(), 400)].into(),
            ..Default::default()
        };
        let b = Library {
            history: vec![item("y", 250), item("z", 100)],
            progress: [("y".to_string(), prog(20.0, 100.0, 250, None))].into(),
            mylist: vec![listed("v", 80)],
            ..Default::default()
        };

        let ab = merge_libraries(&a, &b, 1000).library;
        let ba = merge_libraries(&b, &a, 1000).library;
        let canon = |l: &Library| serde_json::to_string(l).unwrap();
        assert_eq!(canon(&ab), canon(&ba), "merge must be commutative");

        let again = merge_libraries(&ab, &b, 1000).library;
        assert_eq!(canon(&ab), canon(&again), "merge must be idempotent");
    }
}
