# Declared divergences

Every way `groloo-core` deliberately answers differently from the artifact
Groloo-Web ships today (`public/assets/heart/0.1.0-5ecaa78/`), with the fixture
that pins it and the reason the new answer is the right one.

This file is the prose half of `fixtures.mjs`. The machine half is the `expect`
field on each fixture, and the two are kept honest by the runner rather than by
discipline: `node tests/differential/run.mjs` fails when a fixture declared
`match` diverges **and** when a fixture declared a divergence agrees. A
divergence that quietly stops happening makes this document wrong, and the gate
says so on the next run.

**A declared divergence is not a relaxed comparison.** The output is still
asserted byte for byte; what a declaration changes is *which* answer counts as
correct, not *how closely* it is checked. Nothing here is compared loosely, no
matcher is widened, and no fixture was deleted to turn the gate green.

---

## What the gate runs against

Both cores are vendored artifacts read out of `Groloo-Web/public/assets/`. The
old one is the wasm serving the app right now. The new one is
`public/assets/groloo-core/<build>/`, and three things must agree before the
first fixture is evaluated:

| check | failure mode it closes | on failure |
|---|---|---|
| every file hashes to what its `manifest.json` claims | truncated copy, half-finished vendor, hand-edited glue | throws |
| `scripts/build-wasm.mjs --no-vendor` reproduces those exact bytes | fixing a rule, gating, then shipping an artifact built before the fix | throws |
| `Groloo-Web/src/lib/heart.ts` pins that `CORE_BUILD` and that `CORE_WASM_SHA256` | the app fetches a folder the gate never looked at | reservation; `--strict` exits 1 |

The harness used to build its own copy into `tests/.artifacts/new/`. Those bytes
were never the bytes anybody deployed — 319,319 against 319,143 — because
`scripts/build-wasm.mjs` passes two `--remap-path-prefix` flags and the harness's
own `cargo build` did not. A gate that validates a different build than the one
deployed is not validating the deployment, which is the failure this entire phase
exists to eliminate; reproducing it inside the gate was the worst place for it.
The first two checks have no flag to skip them.

The third is a reservation rather than a throw because `heart.ts` belongs to
Groloo-Web: re-vendoring legitimately makes its pin stale for as long as it
takes to paste the two lines the build script prints. The run still evaluates
every fixture and still prints the numbers; it simply refuses to call itself
green.

---

## One id collision, since closed

`crates/groloo-core/src/api.rs` used to label the removal-durability rule
**"recorded divergence D6"**. The contract's D6 was already taken — it is the
add-on manifest-URL normalisation, used with that meaning at `api.rs:741` and in
this harness's `UNCOMPARABLE` table — and the same number cannot mean two things
in one record, so this harness called the removal sweep **D8**.

The Rust side has since been renumbered to agree (`api.rs`, `library.rs`), so
**D8 now means the same rule on both sides of the boundary** and D6 is the
manifest-URL rule everywhere. It is recorded here rather than deleted because a
number that changed meaning once is worth being able to look up.

---

## The register

### D1 — an exact `at` tie on a resume position is settled by the data

*Fixture:* `merge_library/a-progress-tie-is-settled-by-the-data`
(was `progress-tie-goes-to-local` while it was still declared a match)
*Source:* `library::progress_tie_break`, `library.rs:521`

| | |
|---|---|
| input | local `{k: {pos 1, dur 100, at 500}}`, remote `{k: {pos 2, dur 100, at 500}}` |
| old | `$.progress.k.pos` → `1` |
| new | `$.progress.k.pos` → `2` |

0.1.0 resolved an equal-`at` collision by keeping whichever record the loop
reached first. Under `heartLibrary.ts:44` — `hydrate(local)` then
`pulled(remote)` — that is always **local**, so two devices that wrote in the
same millisecond each kept their own position forever and neither ever
converged. The new rule is the data: further-along `pos`, then `dur`, then
`lang`. Never rewind the person watching — of two equally-recent claims, the
further-along one is the one that has seen the other's frames.

**Why this can never be re-declared `match`.** `merge_libraries(a, b)` and
`merge_libraries(b, a)` now produce the same bytes, and *any* commutative rule
must disagree with "whichever argument came first" on some tie input. There is no
version of the commutativity fix that keeps this fixture matching. The fixture was
renamed because its old name stated the answer the new core does not give, which
is a trap for the next reader; the inputs, the clock and the strictness are
unchanged.

### D2 — equal-`at` history entries come out in id order

*Source:* `library.rs:931`. No fixture diverges: `merge_library/history-ties-order-by-id`
declares `match` and matches.

Recorded here because D7 is one level below it and the two are easy to confuse.
D2 is the **order** of equal-`at` rows in the output list, and it was already
commutative in 0.1.0. D7 is the **content** of a single row.

### D3 — removing a series takes its episodes' resume positions with it

*Fixtures:* `library_remove/sweeps-episode-progress-keys`,
`continue_watching/episode-keyed-progress`
*Source:* `Library::remove` (`library.rs:251`), `LibraryItem::media_key` (`types.rs:334`)

The shell keys progress by media key (`id:S#E#`), but 0.1.0's `Remove` did
`progress.remove(&id)` and nothing else, so every episode's resume position
survived a series removal forever — counting against `PROGRESS_CAP` and
resurrecting on re-add. `continue_watching` had the mirror-image bug: it looked
progress up by `item.id()`, so an episode never matched and the rail was always
empty, which is why `heartLibrary.ts:10-12` says the rail is still derived in the
store.

D3 is the **local** half of the deletion story; D8 is the half that makes it
stick across devices. They are not redundant — see D8.

### D4 — `Progress.lang` rides along with the position it belongs to

*Fixtures:* the four `merge_library/progress-lang-*`
*Source:* `library::merge_progress` (`library.rs:540`)

`lang` cannot exist on the old side at all, so it is lifted **out** of the
structural comparison by `compare.mjs::stripLang` and asserted **by value**
through each fixture's `expectLang`. A fixture that declares a language and does
not get it fails the run as hard as a structural difference; `pos`/`dur`/`at` are
still compared byte for byte. A field removed from the comparison and asserted
elsewhere is still tested — a field merely removed is not.

The rule: `merge_progress` carries a language forward from the **losing** record
when the winner has none. 0.1.0 had nowhere to put it, so `history.ts::keepLangs()`
re-attached it after every core call — and being called from only two of the four
write paths, it silently discarded the saved audio track on the other two.

### D5 — the resume window is `[0.01, 0.94]`

*Fixture:* `continue_watching/between-the-two-finished-thresholds`
*Source:* `library.rs:972`, `api.rs:699`

`0.92` sits between 0.1.0's `Progress::is_finished` cutoff of `0.9` and the
shell's long-standing `PROGRESS_DONE` of `0.94` (`history.ts:26`). `0.94` wins: it
is the number users actually experience, and the `0.9` had no consumer. The title
leaves the rail on the old core and stays on it in the new one.

### D6 — add-on manifest URLs normalise correctly

*No fixture here, by construction.* `addon.rs` shipped in 0.1.0 with **no
binding of any kind**, so there is no old-core behaviour to differ from; it is
listed in `UNCOMPARABLE`. Its real twin is TypeScript (`stores/addons.ts:203`,
`addonClient.ts:45-53`) and D6 has to be asserted against *that*.

`stores/addons.ts:203` tests `/manifest\.json$/` against the whole string
including the query, so it never matches a query-bearing URL and appends a second
segment. By the Stremio convention documented in that same file's header, a
*configured* add-on packs its credentials into the URL — so the add-ons that
break are exactly the credentialed ones.

**This is the D6 that owns the number.** See the collision note above.

### D7 — a same-id, equal-`at` list collision converges on content

*Fixture:* `merge_library/a-same-id-equal-at-collision-converges`
*Source:* `library::content_order` / `library::takes_the_slot` (`library.rs:421`, `:434`), `api.rs:425`

| | |
|---|---|
| input | local history `[{id tt1, title "Alpha", at 100}]`, remote `[{id tt1, title "Beta", at 100}]` |
| old | `$.history[0].title` → `"Alpha"` |
| new | `$.history[0].title` → `"Beta"` |

D1 one level up. `Some(prev) if prev.at >= it.at => {}` kept whichever row the
loop reached first — local — so two devices holding different content for one id
in the same millisecond each kept their own row forever. `takes_the_slot` is
shared by `merge_history` and `merge_mylist` so the two cannot drift, and it
settles an exact tie on canonical `serde_json` bytes, greater wins.

**Why bytes.** Unlike a `Progress` there is no "further along" field to prefer,
and any per-field rule (longest title? non-null poster?) would be arbitrary *and*
would need re-litigating every time the struct grows a field. Bytes are arbitrary
too, but honestly so: they are total over every field simultaneously, they stay
total when a field is added, and they are the same bytes this harness and the
shell already compare.

**The cost, named.** The winner is the *agreed* record, not the *better* one, so a
poster can flip once. Once, and then stop — which is strictly better than a row
that disagrees forever.

**The hidden dependency.** `serde_json` is built without `preserve_order`, so
`MyListItem::rating` — the one free-form `Value` in these types — serialises its
keys in `BTreeMap` order rather than arrival order. Two devices that received the
same rating object through different parsers therefore produce the same bytes.
**Turning that feature on would reintroduce input-order dependence one level
down**, which is the bug this rule exists to remove.

**Not the same as D2**, which is the ordering of equal-`at` rows and was already
commutative. Like D1, this can never be re-declared `match`: any commutative rule
disagrees with "whichever argument came first" on some tie input.

*My List half, subsumed:* the identical rule applies to `merge_mylist`, but
0.1.0's `Library` has three fields and drops `mylist` entirely, so there is no
old-side value to compare against. It is already covered by `N-mylist` and adds no
information; it is recorded here only so the register is complete.

### D8 — the deletion survives the next sync

*Fixtures:* `merge_library/a-removed-titles-progress-does-not-come-back`,
`merge_library/a-re-watch-after-the-removal-keeps-its-position`
*Source:* `library::sweep_removed_progress` (`library.rs:633`), `api.rs:416`
(which called it D6 until the renumbering above)

**Half one — the removal sticks.**

| | |
|---|---|
| input | local `removed {tt1: 200}`, remote `progress {tt1: …at 100, tt1:S1E1: …at 100}`, `now 1000` |
| old | `$.progress` → `{"tt1":{…},"tt1:S1E1":{…}}` |
| new | `$.progress` → `{}` |

0.1.0 unions the two progress maps unconditionally, so a device that has not yet
seen a removal hands every `id:S#E#` key straight back on the next pull: the user
deletes a series, it returns, and it counts against `PROGRESS_CAP` again. D3's
local sweep cannot fix this — it deletes the keys on *this* device, and the other
replica still has them. **A deletion any replica can undo is not a deletion**, so
the tombstone has to be what decides, on every merge, for as long as it lives.
`$.history` agrees on both cores here: the history half of the tombstone rule
already worked, and only progress was missing it.

**Half two — a re-watch after the removal keeps its position.**

| | |
|---|---|
| input | local `removed {tt1: 200}`, remote `progress {tt1:S1E1: …at 300, tt1:S2E9: …at 100}` |
| old | `$.progress["tt1:S2E9"]` present |
| new | `$.progress["tt1:S2E9"]` absent (and `tt1:S1E1` identical on both sides) |

The sweep is `removed[id] >= p.at`, which mirrors `merge_history`'s existing
keep-rule (`tomb < it.at`) exactly. It has to: a re-watch newer than the removal
already resurrects the *history entry*, so it must also keep the *position*, or
the title returns to Continue Watching having forgotten where the user was. Equal
timestamps go to the removal — a delete and a tick in the same millisecond is a
delete of that tick. Two fixtures rather than one because a fixture pinning only
the first half would also be satisfied by an *unconditional* sweep, which is a
different and worse rule.

**Scope.** Swept against `removed` only, never `mylist_removed`: a My List
tombstone says nothing about where the user is in a title, and un-listing a film
must not silently forget its position. Swept *before* `cap_progress`, so the
`truncated.progress` warning reports what the cap actually ate rather than
including rows the tombstones were about to take anyway.

**An empty-string tombstone owns nothing.** `api::library_remove` refuses to
create one precisely because `""` would then own every key beginning with a
separator (see `N-guard`); honouring one smuggled in by a hostile remote document
would reopen the hole the boundary closes.

**No wire change.** Nothing new is stored and nothing new is sent — the tombstone
that decides was already travelling in `removed`, and an older client still
converges, because the rule is a function of state both ends already exchange.

---

## `N-*` — new behaviour rather than changed behaviour

### N-clock — a clock the core cannot believe is zero, not `u64::MAX`

*Fixture:* `merge_library/clock-is-infinity`
*Source:* `api::clock` (`api.rs:91`)

| | |
|---|---|
| input | `now: Infinity`, local `removed {x: 50, y: 4000}` |
| old | `$.removed` → `{}` — every tombstone pruned |
| new | `$.removed` → `{"x":50,"y":4000}` |

0.1.0 turned `now` into a `u64` with an `as` cast, which **saturates** rather than
wraps. That reads like a defence and is the opposite of one: a tombstone is pruned
when `now - at > TOMB_TTL_MS`, so a clock of `u64::MAX` prunes every tombstone the
device has ever recorded. One uninitialised `Date.now()` wrapper, one division by
zero in a shim, and every title the user had removed on any device comes back on
the next sync — permanently, because the evidence of the removal is what got
deleted. `is_nan()` did not catch it.

The new core clamps any clock that is not finite, positive and under `2^53` to
**zero**, which prunes nothing and expires nothing. The bound is the ABI's, not a
guess about calendars: past `Number.MAX_SAFE_INTEGER` an `f64` cannot hold
consecutive integers, so whatever JS meant is already unrecoverable.

Not free, and the cost is named in the source: a removal recorded under a broken
clock gets `at: 0` and loses to any history entry on the next merge, so that
removal does not stick. Removing the title again fixes it. Wiping the tombstone
map is not recoverable by anything.

There is no version of this fix that stays byte-identical to a core that wipes the
map — agreeing here would mean keeping the bug. The other four clocks
(`zero`, `negative`, `nan`, `fractional`, `far-behind`) still declare `match` and
still match; this is the only member of the family that behaves differently, which
is why its fixture stays next to them rather than being filed with the
divergences.

### N-nowipe — an unreadable document no longer erases the library

*Fixture:* `merge_library/unreadable-local-leaves-the-remote-un-normalised`

Heart's `hydrate()` ends in `unwrap_or_default()` (`wasm.rs:231`): an unreadable
local document became `{}`, and `heartLibrary.ts:46` then wrote that empty object
back as the user's truth. The new core returns the readable side untouched with
`ok:false`. The visible cost: nothing is lost, but nothing is normalised either —
history stays in the order it arrived and an expired tombstone survives until the
next successful merge.

### N-lenient — one unreadable record costs that record

*Fixtures:* `merge_official/null-name-on-a-cdn-record`,
`merge_library/one-unreadable-history-row`, `merge_library/a-null-progress-value`,
`merge_library/a-numeric-id-from-a-third-party-catalog`,
`library_record_watch/an-item-with-a-numeric-id`

0.1.0 failed the whole-document deserialize and took the library with it. `de::lenient_vec`
drops the row and keeps the rest. Includes `id: 603` — `lib/types.ts` has always
typed ids as `string | number`.

### N-guard — an empty id is refused

*Fixture:* `library_remove/an-empty-id-is-refused`

An empty id would tombstone the empty string and, with D3's prefix sweep, delete
every key beginning `:`. 0.1.0 happily tombstoned `""`. The new core refuses with
`ok:false` and changes nothing. This is the guard D8 leans on at the merge.

### N-mylist — `mylist` / `mylistRemoved` are new fields

*Fixture:* `merge_library/mylist-half-of-the-document`

0.1.0's `Library` had three fields, so `stores/library.ts` maintained a fourth
hand-written copy of this same CRDT. `compare.mjs::stripMyList` drops the two keys
**only when empty**; a non-empty value reaches the comparison and fails, because
silently discarding a real extra output would be the harness hiding a difference
to stay green.

### N-nosort — `continue_watching` is a pure read

*Fixture:* `continue_watching/history-supplied-out-of-order`

0.1.0's `hydrate` sorted and capped as a side effect of loading. The new
`continue_watching` preserves the caller's order; sorting belongs to
`merge_library`/`record_watch`, which is where the shell already gets it.

### N-loud / N-envelope / N-core / N-warnings

Always-on, true of every call, so they are reported once by the runner rather than
tagged onto ninety fixtures. Every function except `core_version` answers
`{ok, core, data, warnings, error}`; `data` is always the graceful-degradation
value, so a shell that destructures `data` and ignores `ok` behaves bit-identically
to 0.1.0. `N-loud` is asserted per-fixture via `expectOk`/`expectErrorCode` where
the old core swallowed a failure: the degraded **value** is unchanged, and the
silence is what went away.

---

## Not declared — still open

None. `U1` was the only entry here; it is settled below.

---

## Settled — U1, `reconcile_install_state` and an empty remote map

*Fixture:* `reconcile_install_state/newer-remote-carrying-an-EMPTY-map` — now
declared `match`, because **the core was changed to agree with 0.1.0**.

0.1.0 decided remote-present as `!map.is_empty()` (`runtime.rs:182`), so an empty
remote map read as "there is no remote" and it uploaded. The new core decided on
`remote != null` and therefore **adopted**, moving `at` to the remote clock — so
where 0.1.0 preserved the device's install state, the new core cleared it.

**Decision: 0.1.0 wins. An empty remote map means "there is no remote yet".**

The asymmetry is what settles it. A remote can read back empty for reasons that
have nothing to do with the user's intent — a first sync, a server that has not
written the row yet, a failed migration, an account restored from a blank slate.
If empty means "adopt", any of those silently erases what is on the device, and
the user has no way to get it back. If empty means "no remote", the cost of being
wrong is re-uploading a state the server already had, which costs nothing and is
undone by the next pull. One direction loses data and the other wastes a request.

Implemented at `api.rs` in `reconcile_install_state`, which now passes
`req.remote.as_ref().is_some_and(|r| !r.map.is_empty())` as `remote_present`
rather than `req.remote.is_some()`. `state::reconcile` is untouched — the rule
about what "present" *means* belongs at the boundary where the request is read,
not inside the decision table.

Two unit tests pin it directly, in addition to the fixture: an empty map with a
*newer* remote clock still uploads, and an empty map with `ownerChanged` still
noops rather than adopting nothing.

Recorded here rather than deleted, per the note at the top of this file: a
divergence that quietly stops happening is exactly what this document exists to
make impossible to do silently. It stopped happening because the code changed,
which is the only sanctioned reason.
