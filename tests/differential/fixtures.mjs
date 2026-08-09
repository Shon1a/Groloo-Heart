/* The corpus, and the eight adapters that push it through two cores whose call
 * shapes do not match.
 *
 * ## Why an adapter per family
 *
 * The old core is an Elm runtime; the new one is free functions. There is no
 * function in 0.1.0-5ecaa78 called `merge_library`, and there never was — the
 * shell got that behaviour by holding a `LibraryRuntime`, hydrating the whole
 * document into it, sending one message and snapshotting it back out
 * (`heartLibrary.ts:44`). So the honest unit of comparison is not "the same
 * function on both sides", it is "the same OBSERVABLE BEHAVIOUR, reached the way
 * each core actually offers it". Each family below therefore names the exact old
 * call sequence it is standing in for, and where possible that sequence is the one
 * Groloo-Web runs today rather than a tidier equivalent:
 *
 *   official collection  official.ts:58-66   AddonRuntime + official_payload_fetched
 *   library              heartLibrary.ts:44  LibraryRuntime.hydrate -> op -> snapshot
 *   home rows            heartCatalog.ts:27  CatalogRuntime.set_gating + visible_rows_json
 *
 * ## Why some fixtures are declared divergent
 *
 * A fixture's `expect` is either `'match'` or a divergence id from the phase
 * contract. Declaring one is not a way of excusing a failure — the runner fails
 * just as loudly when a declared divergence stops happening, because that means
 * the contract is now describing something that is not true. A declared divergence
 * still asserts the exact new output, byte for byte; what it changes is which
 * answer counts as correct, not how closely the answer is checked. Nothing in this
 * corpus is compared loosely, and no fixture is deleted to make the gate green.
 *
 * The divergence ids are the contract's own (D1-D6), `D7`/`D8` for two rules the
 * contract did not anticipate but which are now decided and documented, and `N-*`
 * for behaviour that is new rather than changed. Each `why` below is the short
 * form; `DIVERGENCES.md` in this directory is the register, with the inputs, both
 * cores' answers and the argument for the new one.
 *
 * ## One id collision, since closed
 *
 * `api.rs:416` used to label the removal-durability rule "recorded divergence D6",
 * a number already taken by the add-on manifest-URL normalisation — the one
 * asserted against the TypeScript twins rather than here (see `UNCOMPARABLE`), and
 * still what `api.rs:741` means by D6. The same number cannot mean two things in
 * one record, so the removal sweep is **D8** here; the Rust side has since been
 * renumbered to agree, so D8 now names the same rule on both sides.
 *
 * ## What is deliberately NOT here
 *
 * `collection_addons_json` (old-only: folded into merge_official's schema gate),
 * `set_progress` (deliberately absent from the new boundary — see api.rs), the
 * five add-on-protocol functions and `core_version`/`core_constants` (new-only:
 * `addon.rs` had no binding in 0.1.0, so there is nothing on the old side to
 * differ from). Those are reported as coverage gaps, not silently omitted. */

/* --------------------------------------------------------------------------
 * shared inputs
 * ------------------------------------------------------------------------ */

/** The four cards Groloo-Web actually ships (`stores/official.ts:35-44`). */
const INLINE_SHIPPED = [
  { id: 'catalog', section: 'official', name: 'Catalog Rows', ver: 'v1.0.0', tags: ['catalog'], defaultInstalled: true, flags: { official: true, protected: true } },
  { id: 'providers', section: 'official', name: 'Streaming Services', ver: 'v1.0.0', tags: ['providers'], defaultInstalled: false, flags: { official: true, protected: true } },
  { id: 'studios', section: 'official', name: 'Studios', ver: 'v1.0.0', tags: ['studios'], defaultInstalled: true, noConfig: true, preview: true, flags: { official: true, protected: true } },
  { id: 'upcoming', section: 'official', name: 'Upcoming Radar', ver: 'v1.0.0', tags: ['upcoming'], defaultInstalled: true, noConfig: true, preview: true, flags: { official: true, protected: true } },
];

/** Heart's own four, which carry `iconCls`/`glyph` — needed to exercise the icon
 * allow-list on a KNOWN id, where the shipped set has no icon to overwrite. */
const INLINE_ICONED = [
  { id: 'upcoming', section: 'official', name: 'Upcoming', ver: 'v1.3.0', iconCls: 'puzzle', glyph: 'A', tags: ['catalog', 'metadata'], installed: true, noConfig: true, preview: true },
  { id: 'studios', section: 'official', name: 'Studios', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'B', tags: ['catalog', 'metadata'], installed: true, noConfig: true, preview: true },
  { id: 'catalog', section: 'official', name: 'Catalog', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'C', tags: ['catalog', 'metadata'], installed: true },
  { id: 'providers', section: 'official', name: 'Providers', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'D', tags: ['catalog', 'metadata'], installed: false },
];

/** Heart's compiled-in `HOME_ROWS` (`catalog.rs:36-104`), restated as the wire
 * table the new core takes as an argument. Feeding the new core the OLD core's
 * own table is what makes the row comparison about the RULE rather than about two
 * different lists of rows. */
const HEART_ROWS = [
  { cat: 'trending_movie', kind: 'catalog' }, { cat: 'trending_tv', kind: 'catalog' },
  { cat: 'top_movie', kind: 'catalog' }, { cat: 'top_tv', kind: 'catalog' },
  { cat: 'trending_anime', kind: 'catalog' }, { cat: 'top_anime', kind: 'catalog' },
  { cat: 'prov_netflix', kind: 'provider' }, { cat: 'prov_disney', kind: 'provider' },
  { cat: 'prov_prime', kind: 'provider' }, { cat: 'prov_apple', kind: 'provider' },
  { cat: 'prov_max', kind: 'provider' }, { cat: 'prov_paramount', kind: 'provider' },
  { cat: 'prov_crunchyroll', kind: 'provider' }, { cat: 'studios', kind: 'studio' },
];

const S = JSON.stringify;
const doc = (schema, addons, version = '6') => S({ schema, version, section: 'official', addons });
const item = (id, at, extra = {}) => ({ id, title: id, at, ...extra });
const prog = (pos, dur, at, extra = {}) => ({ pos, dur, at, ...extra });

/** 30 days in ms — the tombstone TTL both cores compile in. Written out rather
 * than imported because the point of a differential fixture is to state the
 * number independently of either implementation. */
const TTL = 2592000000;

/* --------------------------------------------------------------------------
 * 1. official_payload_file
 * ------------------------------------------------------------------------ */

const officialPayloadFile = {
  name: 'official_payload_file',
  old: 'official_payload_file(index) -> string ("" on every failure)',
  neu: 'official_payload_file(index) -> envelope{ data: string|null }',
  fixtures: [
    { id: 'happy', expect: 'match', index: S({ schema: 1, version: '6', collections: [{ id: 'official', section: 'official', file: 'addons.json' }] }) },
    { id: 'official-listed-second', expect: 'match', index: S({ schema: 1, collections: [{ id: 'community', section: 'community', file: 'community.json' }, { id: 'official', section: 'official', file: 'addons.json' }] }) },
    { id: 'unsupported-schema', expect: 'match', index: S({ schema: 2, collections: [{ id: 'official', section: 'official', file: 'addons.json' }] }) },
    { id: 'no-official-collection', expect: 'match', index: S({ schema: 1, collections: [{ id: 'c', section: 'community', file: 'c.json' }] }) },
    { id: 'official-with-empty-file', expect: 'match', index: S({ schema: 1, collections: [{ id: 'official', section: 'official', file: '' }] }) },
    { id: 'no-collections-key', expect: 'match', index: S({ schema: 1, version: '6' }) },
    { id: 'unknown-forward-fields', expect: 'match', index: S({ schema: 1, version: '7', updated: '2026-07-01', repo: 'x', cdn: 'y', futureKey: [1, 2], collections: [{ id: 'official', section: 'official', file: 'addons.json', count: 9, protectedIds: ['catalog'] }] }) },
    { id: 'not-json', expect: 'match', index: 'not json at all' },
    { id: 'empty-string', expect: 'match', index: '' },
    { id: 'json-but-wrong-shape', expect: 'match', index: S([1, 2, 3]) },
  ],
  run(old, neu, f) {
    const oldOut = old.official_payload_file(f.index);
    const env = neu.__unwrap(neu.official_payload_file(f.index), 'official_payload_file');
    // The old core folds "wrong schema", "no official collection" and "that was
    // not JSON" into one empty string. `data ?? ''` is the projection that makes
    // them comparable; `ok`/`error.code` is the information the old core threw
    // away, and it is reported alongside rather than compared.
    return {
      oldOut, newOut: env.data ?? '', env,
      notes: [`N-envelope: new core also reports ok=${env.ok}` + (env.error ? ` code=${env.error.code}` : '')],
    };
  },
};

/* --------------------------------------------------------------------------
 * 2. merge_official
 * ------------------------------------------------------------------------ */

const mergeOfficial = {
  name: 'merge_official',
  old: 'new AddonRuntime(inline).official_payload_fetched(doc) -> addons_json()  [official.ts:58-66]',
  neu: 'merge_official(inline, doc) -> envelope{ data: AddonDescriptor[] }',
  fixtures: [
    { id: 'identical-is-noop', expect: 'match', inline: INLINE_ICONED, payload: doc(1, [
      { id: 'upcoming', section: 'official', name: 'Upcoming', version: '1.3.0', ver: 'v1.3.0', iconCls: 'puzzle', glyph: 'A', tags: ['catalog', 'metadata'], defaultInstalled: true, noConfig: true, preview: true, kind: 'discovery' },
      { id: 'studios', section: 'official', name: 'Studios', version: '1.0.0', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'B', tags: ['catalog', 'metadata'], defaultInstalled: true, noConfig: true, preview: true, kind: 'discovery' },
      { id: 'catalog', section: 'official', name: 'Catalog', version: '1.0.0', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'C', tags: ['catalog', 'metadata'], defaultInstalled: true, kind: 'discovery' },
      { id: 'providers', section: 'official', name: 'Providers', version: '1.0.0', ver: 'v1.0.0', iconCls: 'puzzle', glyph: 'D', tags: ['catalog', 'metadata'], defaultInstalled: false, kind: 'discovery' },
    ]) },
    { id: 'appends-new-card-and-derives-ver', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'nebula', section: 'official', name: 'Nebula', version: '2.1.0', tags: ['catalog'], defaultInstalled: true },
    ]) },
    { id: 'refines-display-never-behaviour', expect: 'match', inline: INLINE_ICONED, payload: doc(1, [
      { id: 'catalog', section: 'official', name: 'Trending & Top', ver: 'v2.0.0', installed: false, locked: true, noConfig: true, preview: true },
    ]) },
    { id: 'xss-iconcls-on-known-id', expect: 'match', inline: INLINE_ICONED, payload: doc(1, [
      { id: 'catalog', section: 'official', iconCls: 'x"><img src=y onerror=alert(1)>' },
    ]) },
    { id: 'xss-iconcls-on-new-card', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'evil', section: 'official', name: 'Evil', iconCls: '"><script>alert(1)</script>' },
    ]) },
    { id: 'iconcls-over-40-chars', expect: 'match', inline: INLINE_ICONED, payload: doc(1, [
      { id: 'catalog', section: 'official', iconCls: 'a'.repeat(41) },
    ]) },
    { id: 'iconcls-exactly-40-chars-is-allowed', expect: 'match', inline: INLINE_ICONED, payload: doc(1, [
      { id: 'catalog', section: 'official', iconCls: 'b'.repeat(40) },
    ]) },
    { id: 'rejects-transport-url', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'streamer', section: 'official', name: 'P', transportUrl: 'http://x/manifest.json' },
    ]) },
    { id: 'rejects-stream-resource', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'streamer2', section: 'official', name: 'P2', resources: ['stream'] },
    ]) },
    { id: 'rejects-stream-on-a-KNOWN-id', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'catalog', section: 'official', name: 'Hijacked', transportUrl: 'http://x/manifest.json' },
    ]) },
    { id: 'rejects-community-section', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'c1', section: 'community', name: 'community card' },
    ]) },
    { id: 'rejects-empty-id', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: '', section: 'official', name: 'x' },
    ]) },
    { id: 'mixed-batch-good-and-rejected', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: '', section: 'official', name: 'idless' },
      { id: 'nebula', section: 'official', name: 'Nebula', version: '2.1.0' },
      { id: 'streamer', section: 'official', name: 'P', resources: ['stream', 'meta'] },
      { id: 'c1', section: 'community', name: 'community' },
      { id: 'catalog', section: 'official', name: 'Catalog Rows v2', version: '1.4.0' },
      { id: 'quasar', section: 'official', iconCls: '<img>', name: '' },
    ]) },
    { id: 'tags-are-never-wiped-to-empty', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'catalog', section: 'official', name: 'Catalog Rows', tags: [] },
    ]) },
    { id: 'tags-are-replaced-when-supplied', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'catalog', section: 'official', tags: ['catalog', 'rows', 'new'] },
    ]) },
    { id: 'new-card-with-empty-name-falls-back-to-id', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'anon', section: 'official', name: '' },
    ]) },
    { id: 'section-defaults-to-official-when-absent', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, [
      { id: 'implicit', section: undefined, name: 'Implicit' },
    ]) },
    { id: 'empty-payload-addons', expect: 'match', inline: INLINE_SHIPPED, payload: doc(1, []) },
    { id: 'unsupported-schema-keeps-inline', expect: 'match', inline: INLINE_SHIPPED, payload: doc(2, [
      { id: 'nebula', section: 'official', name: 'Nebula' },
    ]) },
    { id: 'unparseable-payload-keeps-inline', expect: 'match', inline: INLINE_SHIPPED, payload: '}{ not json' },
    { id: 'payload-as-a-bare-array-keeps-inline', expect: 'match', inline: INLINE_SHIPPED, payload: S([{ id: 'nebula', section: 'official' }]) },
    {
      id: 'null-name-on-a-cdn-record',
      expect: { d: 'N-lenient', why: 'The new AddonDescriptor maps an explicit `null` name/tags/types/resources to the default (types.rs `de::null_to_default`). 0.1.0 failed the WHOLE payload deserialize on it, so one null field cost the entire CDN collection and the inline set stood. Here the record merges.' },
      inline: INLINE_SHIPPED,
      payload: doc(1, [{ id: 'nebula', section: 'official', name: null, tags: null, version: '2.1.0' }]),
    },
  ],
  run(old, neu, f) {
    const inline = S(f.inline);
    const rt = new old.AddonRuntime(inline);
    rt.official_payload_fetched(f.payload);
    const oldOut = JSON.parse(rt.addons_json());
    const status = JSON.parse(rt.status());
    rt.free();

    const env = neu.__unwrap(neu.merge_official(inline, f.payload), 'merge_official');
    return {
      oldOut, newOut: env.data, env,
      notes: [`N-envelope: old status=${status}; new ok=${env.ok} warnings=[${env.codes.join(', ')}]`],
    };
  },
};

/* --------------------------------------------------------------------------
 * 3. reconcile install state
 * ------------------------------------------------------------------------ */

/* The old core keeps `install_at` INSIDE the model and exposes no way to set it
 * except `toggle_addon(id, now)` (`Msg::LocalStateLoaded` was never bound to
 * wasm). So a fixture that needs a local timestamp establishes it the only way a
 * browser could: by toggling a card. The resulting descriptor list and map are
 * then read back out and become the new core's `local`, which keeps both sides
 * starting from provably the same state rather than from two hand-written
 * approximations of it. */
const ADDONS_3 = [
  { id: 'catalog', section: 'official', name: 'Catalog Rows', installed: true },
  { id: 'providers', section: 'official', name: 'Streaming Services', installed: false },
  { id: 'studios', section: 'official', name: 'Studios', installed: true },
  { id: 'cinemeta', section: 'official', name: 'Cinemeta', installed: true, locked: true },
];

const reconcileInstallState = {
  name: 'reconcile_install_state',
  old: 'new AddonRuntime(a).toggle_addon(id, at).install_state_pulled(map, at, owner) -> addons_json()/install_map_json()',
  neu: 'reconcile_install_state({addons, local, remote, ownerChanged}) -> envelope{ data: {decision, addons, map, at} }',
  fixtures: [
    { id: 'adopts-a-newer-remote', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 10 }, remote: { map: { catalog: false, providers: true }, at: 20 } },
    { id: 'uploads-when-local-is-newer', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 30 }, remote: { map: { catalog: false }, at: 20 } },
    { id: 'uploads-on-an-exact-timestamp-tie', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 20 }, remote: { map: { catalog: false }, at: 20 } },
    { id: 'owner-changed-adopts-an-older-remote', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 30 }, remote: { map: { catalog: false }, at: 20 }, ownerChanged: true },
    { id: 'owner-changed-with-no-remote-is-a-noop', expect: 'match', addons: ADDONS_3, remote: null, ownerChanged: true },
    { id: 'no-remote-uploads-local', expect: 'match', addons: ADDONS_3, remote: null },
    { id: 'locked-addon-is-never-toggled-or-mapped', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 10 }, remote: { map: { cinemeta: false, catalog: false }, at: 20 } },
    { id: 'remote-map-with-unknown-ids', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 10 }, remote: { map: { ghost: true, phantom: false, catalog: false }, at: 20 } },
    { id: 'remote-adopts-every-id-at-once', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 5 }, remote: { map: { catalog: false, providers: false, studios: false }, at: 900 } },
    // Was U1, the one unplanned divergence in this corpus. Settled in favour of
    // 0.1.0 and the core changed to match, so it is a `match` like the rest — not
    // a declaration, and not a widened comparison. See U1 in DIVERGENCES.md.
    { id: 'newer-remote-carrying-an-EMPTY-map', expect: 'match', addons: ADDONS_3, toggle: { id: 'providers', at: 10 }, remote: { map: {}, at: 20 } },
  ],
  run(old, neu, f) {
    const rt = new old.AddonRuntime(S(f.addons));
    if (f.toggle) rt.toggle_addon(f.toggle.id, f.toggle.at);
    const localAt = f.toggle ? f.toggle.at : 0;
    const localAddons = JSON.parse(rt.addons_json());
    const localMap = JSON.parse(rt.install_map_json());

    const remoteMap = f.remote ? f.remote.map : {};
    const remoteAt = f.remote ? f.remote.at : 0;
    const fx = JSON.parse(rt.install_state_pulled(S(remoteMap), remoteAt, !!f.ownerChanged));
    const oldOut = {
      // The old core does not name its decision; it emits effects. Persist+Repaint
      // is AdoptRemote, a lone Push is UploadLocal, nothing at all is Noop.
      decision: fx.some((e) => e && e.PersistInstallState) ? 'adoptRemote'
        : fx.some((e) => e && e.PushInstallState) ? 'uploadLocal' : 'noop',
      addons: JSON.parse(rt.addons_json()),
      map: JSON.parse(rt.install_map_json()),
      at: fx.reduce((a, e) => (e && e.PersistInstallState ? e.PersistInstallState.at
        : e && e.PushInstallState ? e.PushInstallState.at : a), localAt),
    };
    rt.free();

    const request = S({
      addons: localAddons,
      local: { map: localMap, at: localAt },
      remote: f.remote ? { map: remoteMap, at: remoteAt } : null,
      ownerChanged: !!f.ownerChanged,
    });
    const env = neu.__unwrap(neu.reconcile_install_state(request), 'reconcile_install_state');
    return { oldOut, newOut: env.data, env, notes: [`N-envelope: new ok=${env.ok}`] };
  },
};

/* --------------------------------------------------------------------------
 * 4. merge_library
 * ------------------------------------------------------------------------ */

/* `heartLibrary.ts:44` — hydrate the whole document, send one message, snapshot
 * back out. `pulled` takes the remote document as THREE separate strings, each
 * independently `unwrap_or_default()`ed, so a remote that is not JSON at all is
 * reproduced by handing the same unparseable string to all three slots: every one
 * of them defaults, which is precisely what a browser would observe. */
function splitRemote(remoteJson) {
  let r;
  try { r = JSON.parse(remoteJson); } catch { return [remoteJson, remoteJson, remoteJson]; }
  if (!r || typeof r !== 'object' || Array.isArray(r)) return [remoteJson, remoteJson, remoteJson];
  return [S(r.history ?? []), S(r.progress ?? {}), S(r.removed ?? {})];
}

const bigHistory = (n, from = 0) => Array.from({ length: n }, (_, i) => item(`h${String(from + i).padStart(3, '0')}`, from + i + 1));
const bigProgress = (n) => Object.fromEntries(Array.from({ length: n }, (_, i) => [`p${String(i).padStart(3, '0')}`, prog(1, 100, i + 1)]));

const mergeLibrary = {
  name: 'merge_library',
  old: 'new LibraryRuntime().hydrate(local).pulled(h, p, r, now) -> snapshot_json()  [heartLibrary.ts:44]',
  neu: 'merge_library(local, remote, now) -> envelope{ data: Library }',
  fixtures: [
    { id: 'merges-by-recency', expect: 'match', now: 1000,
      local: S({ history: [item('a', 100)], progress: { a: prog(10, 100, 100) }, removed: {} }),
      remote: S({ history: [item('a', 300), item('b', 200)], progress: { a: prog(50, 100, 300) }, removed: {} }) },

    { id: 'tombstone-beats-an-older-remote-entry', expect: 'match', now: 600,
      local: S({ history: [item('a', 100)], progress: {}, removed: { b: 500 } }),
      remote: S({ history: [item('a', 300), item('b', 200)], progress: {}, removed: {} }) },

    { id: 'a-re-watch-newer-than-its-tombstone-comes-back', expect: 'match', now: 600,
      local: S({ history: [], progress: {}, removed: { b: 200 } }),
      remote: S({ history: [item('b', 500)], progress: {}, removed: {} }) },

    { id: 'history-ties-order-by-id', expect: 'match', now: 1000,
      local: S({ history: [item('zulu', 100), item('alpha', 100)], progress: {}, removed: {} }),
      remote: S({ history: [item('mike', 100)], progress: {}, removed: {} }) },

    { id: 'empty-both-sides', expect: 'match', now: 1000, local: S({}), remote: S({}) },

    { id: 'episode-keyed-progress-round-trips', expect: 'match', now: 1000,
      local: S({ history: [item('tt1', 100, { key: 'tt1:S1E1', season: 1, episode: 1 })], progress: { 'tt1:S1E1': prog(300, 1400, 100) }, removed: {} }),
      remote: S({ history: [item('tt1', 200, { key: 'tt1:S1E2', season: 1, episode: 2 })], progress: { 'tt1:S1E2': prog(60, 1400, 200) }, removed: {} }) },

    { id: 'realistic-sync-40-titles', expect: 'match', now: 1700000000000,
      local: S({ history: bigHistory(40), progress: bigProgress(30), removed: { old1: 1699000000000 } }),
      remote: S({ history: bigHistory(20, 30), progress: bigProgress(10), removed: { old2: 1699500000000 } }) },

    /* ---- tombstone expiry ---- */
    { id: 'tombstone-expires-exactly-one-ms-past-the-ttl', expect: 'match', now: 1000 + TTL + 1,
      local: S({ history: [], progress: {}, removed: { x: 1000 } }), remote: S({}) },
    { id: 'tombstone-survives-exactly-at-the-ttl', expect: 'match', now: 1000 + TTL,
      local: S({ history: [], progress: {}, removed: { x: 1000 } }), remote: S({}) },
    { id: 'expired-tombstone-lets-a-title-return', expect: 'match', now: 1000 + TTL + 1,
      local: S({ history: [], progress: {}, removed: { b: 1000 } }),
      remote: S({ history: [item('b', 500)], progress: {}, removed: {} }) },
    { id: 'newest-removal-per-id-wins-across-devices', expect: 'match', now: 5000,
      local: S({ history: [], progress: {}, removed: { x: 1000, y: 4000 } }),
      remote: S({ history: [], progress: {}, removed: { x: 3000, y: 2000 } }) },

    /* ---- cap overflow ---- */
    { id: 'history-cap-60-from-one-side', expect: 'match', now: 1000000,
      local: S({ history: bigHistory(70), progress: {}, removed: {} }), remote: S({}) },
    { id: 'history-cap-60-across-both-sides', expect: 'match', now: 1000000,
      local: S({ history: bigHistory(45), progress: {}, removed: {} }),
      remote: S({ history: bigHistory(45, 45), progress: {}, removed: {} }) },
    { id: 'progress-cap-240-from-one-side', expect: 'match', now: 1000000,
      local: S({ history: [], progress: bigProgress(250), removed: {} }), remote: S({}) },
    { id: 'progress-cap-240-across-both-sides', expect: 'match', now: 1000000,
      local: S({ history: [], progress: bigProgress(200), removed: {} }),
      remote: S({ history: [], progress: Object.fromEntries(Array.from({ length: 100 }, (_, i) => [`q${i}`, prog(2, 100, 500 + i)])), removed: {} }) },
    { id: 'exactly-at-the-history-cap', expect: 'match', now: 1000000,
      local: S({ history: bigHistory(60), progress: {}, removed: {} }), remote: S({}) },

    /* ---- clock skew ---- */
    { id: 'clock-is-zero', expect: 'match', now: 0,
      local: S({ history: [item('a', 100)], progress: {}, removed: { x: 50 } }), remote: S({}) },
    { id: 'clock-is-negative', expect: 'match', now: -1,
      local: S({ history: [item('a', 100)], progress: {}, removed: { x: 50 } }), remote: S({}) },
    { id: 'clock-is-nan', expect: 'match', now: NaN,
      local: S({ history: [item('a', 100)], progress: {}, removed: { x: 50 } }), remote: S({}) },
    {
      /* The one clock the old core did not survive. Kept next to its four
       * siblings — zero, negative, NaN, fractional, all of which still match —
       * because what makes this one interesting is that it is the ONLY value in
       * the family that behaves differently, and a fixture filed away with the
       * divergences would lose that. */
      id: 'clock-is-infinity',
      expect: { d: 'N-clock', why: '0.1.0 turned `now` into a `u64` with an `as` cast, which SATURATES rather than wraps — so `Infinity` became `u64::MAX`, and since a tombstone is pruned when `now - at > TOMB_TTL_MS`, that clock prunes EVERY tombstone the device has ever recorded. One uninitialised `Date.now()` wrapper or one division by zero in a shim, and every title the user had removed on any device comes back on the next sync, permanently — because the evidence of the removal is what got deleted. `is_nan()` did not catch it. The new core clamps any clock that is not finite, positive and under 2^53 to ZERO (`api::clock`), which prunes nothing and expires nothing: the tombstones survive, which is why `$.removed` differs. There is no version of this fix that stays byte-identical to a core that wipes the map — agreeing here would mean keeping the bug.' },
      now: Infinity,
      local: S({ history: [item('a', 100)], progress: {}, removed: { x: 50, y: 4000 } }), remote: S({}),
    },
    { id: 'clock-far-behind-every-timestamp', expect: 'match', now: 1,
      local: S({ history: [item('a', 1700000000000)], progress: {}, removed: { x: 1700000000000 } }), remote: S({}) },
    { id: 'clock-is-a-fractional-millisecond', expect: 'match', now: 1000.75,
      local: S({ history: [item('a', 100)], progress: {}, removed: { x: 50 } }), remote: S({}) },

    /* ---- declared divergences ---- */
    /* The unreadable-document pair. `data` AGREES on these two, and that is the
     * good news: the graceful-degradation value the new core hands back is the
     * same value 0.1.0 arrived at by accident. What 0.1.0 could not do is SAY so
     * — it returned a success-shaped snapshot either way, which is problem 7 in
     * one line. So the assertion here is `expectOk`, not a structural difference:
     * same answer, now with a signal attached. */
    { id: 'unreadable-local-document', expect: 'match', expectOk: false, expectErrorCode: 'parse.input',
      now: 1000, local: '<<<not json>>>',
      remote: S({ history: [item('a', 100)], progress: {}, removed: { z: 5 } }) },
    { id: 'unreadable-remote-document', expect: 'match', expectOk: false, expectErrorCode: 'parse.input',
      now: 1000, local: S({ history: [item('a', 100)], progress: {}, removed: { z: 5 } }), remote: '<<<not json>>>' },
    { id: 'both-sides-unreadable', expect: 'match', expectOk: false, expectErrorCode: 'parse.input',
      now: 1000, local: '<<<not json>>>', remote: '}{' },
    {
      /* Where the pair above DOES diverge structurally: 0.1.0 reached its answer
       * by merging the readable side against an empty library, so the result came
       * out sorted, capped and TTL-pruned. The new core returns the readable
       * document AS GIVEN, because a failed call has no business silently
       * rewriting the one input it could read. The shell sees an unsorted history
       * and a tombstone that has aged out — both of which the very next successful
       * merge fixes, and neither of which loses anything. */
      id: 'unreadable-local-leaves-the-remote-un-normalised',
      expect: { d: 'N-nowipe', why: 'THE headline fix, and its one visible cost. 0.1.0 `hydrate()` ends in `unwrap_or_default()` (wasm.rs:231): an unreadable local document became `{}`, and `heartLibrary.ts:46` then wrote that empty object back as the user`s truth. The new core returns the readable side untouched with ok:false — so nothing is lost, but nothing is normalised either: history stays in the order it arrived and an expired tombstone survives until the next successful merge.' },
      expectOk: false,
      now: 1 + TTL + 1, local: '<<<not json>>>',
      remote: S({ history: [item('b', 100), item('a', 300)], progress: {}, removed: { stale: 1 } }),
    },
    {
      id: 'one-unreadable-history-row',
      expect: { d: 'N-lenient', why: 'A history entry with no `id` fails 0.1.0`s whole-document deserialize and therefore wipes the library. The new core drops that row and keeps the rest (types.rs `de::lenient_vec`).' },
      now: 1000, local: S({ history: [item('good', 200), { garbage: true }, item('also_good', 100)], progress: {}, removed: {} }), remote: S({}),
    },
    {
      id: 'a-null-progress-value',
      expect: { d: 'N-lenient', why: '`{"someKey": null}` in progress is a payload the server documents a client can PUT. Under 0.1.0 it took the whole library with it; the new core drops the one key.' },
      now: 1000, local: S({ history: [item('good', 200)], progress: { good: prog(1, 2, 3), broken: null }, removed: {} }), remote: S({}),
    },
    {
      id: 'a-numeric-id-from-a-third-party-catalog',
      expect: { d: 'N-lenient', why: '`lib/types.ts` has always typed ids as `string | number`. 0.1.0 rejected the number and lost the document; the new core coerces to "603".' },
      now: 1000, local: S({ history: [{ id: 603, title: 'The Matrix', year: 1999, rating: 8.7, at: 5 }], progress: {}, removed: {} }), remote: S({}),
    },
    /* ---- the merge is now COMMUTATIVE, and these three are what that cost ----
     *
     * `merge_libraries(a, b)` and `merge_libraries(b, a)` produce the same bytes.
     * That is the property the whole device-sync model rests on: without it two
     * devices pulling from each other trade answers forever instead of converging
     * in one round. 0.1.0 did not have it, and could not have — its rule for a tie
     * was "keep whatever the loop reached first", which under the shell's
     * `hydrate(local)` then `pulled(remote)` order means LOCAL, every time.
     *
     * So D1 and D7 are not two bugs that happen to look alike. They are the same
     * observation at two depths — the resume position, and the row's content — and
     * both are unfixable-while-matching for one structural reason: ANY commutative
     * rule must disagree with "whichever argument came first" on some tie input.
     * A fixture of this shape can therefore never be declared `match` again. The
     * assertion that remains is the exact new output, and it is not weakened. */
    {
      /* Named `progress-tie-goes-to-local` while it was declared a match. The
       * name described the answer 0.1.0 gives, and the new core does not give it;
       * a fixture whose id states the losing behaviour is a trap for the next
       * reader. Same inputs, same `now`, nothing relaxed. */
      id: 'a-progress-tie-is-settled-by-the-data',
      expect: { d: 'D1', why: '0.1.0 resolved an exact `at` tie by keeping whichever record it iterated first — always the local one, because `heartLibrary.ts:44` hydrates local and then pulls remote. Two devices that wrote in the same millisecond therefore each kept their own position forever and neither converged. The new core settles it on the data (`library::progress_tie_break`): further-along `pos` first, then `dur`, then `lang`. Never rewind the person watching — of two equally-recent claims the further-along one is the one that has seen the other`s frames. Here local holds `pos 1` and remote `pos 2`, so the answer flips from 1 to 2. This is a divergence by construction, not by choice: a rule that is the same from both sides cannot also be "whichever came first".' },
      now: 1000,
      local: S({ history: [], progress: { k: prog(1, 100, 500) }, removed: {} }),
      remote: S({ history: [], progress: { k: prog(2, 100, 500) }, removed: {} }),
    },
    {
      id: 'a-same-id-equal-at-collision-converges',
      expect: { d: 'D7', why: 'D1 one level up, on the LIST rather than the progress map. `Some(prev) if prev.at >= it.at => {}` kept whichever row the loop reached first — local — so two devices holding different content for one id in the same millisecond each kept their own row forever. `library::takes_the_slot` (shared by `merge_history` and `merge_mylist`, so the two cannot drift) now compares canonical `serde_json` bytes and keeps the greater, so both sides land on the same row. Unlike a resume position there is no "further along" field to prefer, and any per-field rule (longest title? non-null poster?) would be arbitrary AND would need re-litigating every time the struct grows a field; bytes are arbitrary too, but honestly so, and they stay total over every field at once. The cost is named: the winner is the AGREED record, not the better one, so a poster can flip once — which beats a row that disagrees forever. NOT the same as D2, which is the ORDER of equal-`at` rows and was already commutative.' },
      now: 1000,
      local: S({ history: [item('tt1', 100, { title: 'Alpha' })], progress: {}, removed: {} }),
      remote: S({ history: [item('tt1', 100, { title: 'Beta' })], progress: {}, removed: {} }),
    },
    /* ---- D8: the deletion survives the next sync ----
     *
     * `api.rs:416` called this D6 until it was renumbered; the contract's D6 is the
     * add-on manifest-URL rule (`api.rs:741`, and `UNCOMPARABLE` below) — see the
     * header. Two fixtures, because the rule has two halves and a fixture that
     * pinned only the first would be satisfied by an UNCONDITIONAL sweep, which is
     * a different and worse rule. */
    {
      id: 'a-removed-titles-progress-does-not-come-back',
      expect: { d: 'D8', why: '0.1.0 unions the two progress maps unconditionally, so a device that has not yet seen a removal hands every `id:S#E#` key straight back on the next pull: the user deletes a series, it returns, and it counts against PROGRESS_CAP again. `Library::remove`s local sweep (D3) cannot fix this — it deletes the keys on THIS device, and the other replica still has them. A deletion any replica can undo is not a deletion, so the tombstone has to be what decides, on every merge, for as long as it lives: `library::sweep_removed_progress` re-applies the sweep against the MERGED tombstone map. The history half of this rule already worked in 0.1.0, which is why `$.history` agrees here and only `$.progress` differs. No wire change — the tombstone that decides was already travelling in `removed`.' },
      now: 1000,
      local: S({ history: [], progress: {}, removed: { tt1: 200 } }),
      remote: S({ history: [item('tt1', 100)], progress: { tt1: prog(5, 100, 100), 'tt1:S1E1': prog(10, 100, 100) }, removed: {} }),
    },
    {
      id: 'a-re-watch-after-the-removal-keeps-its-position',
      expect: { d: 'D8', why: 'The other half of the same rule, and the reason the sweep is at-or-after rather than unconditional. `removed[id] >= p.at` mirrors `merge_history`s existing keep-rule (`tomb < it.at`) exactly, so the two halves of one title cannot disagree: a re-watch NEWER than the removal already resurrects the history entry, so it must also keep the position — otherwise the title returns to Continue Watching having forgotten where the user was. `tt1:S1E1` (at 300, after the removal at 200) survives on both cores and is byte-identical; `tt1:S2E9` (at 100, before it) is what diverges. Equal timestamps go to the removal: a delete and a tick in the same millisecond is a delete of that tick.' },
      now: 1000,
      local: S({ history: [], progress: {}, removed: { tt1: 200 } }),
      remote: S({ history: [item('tt1', 300)], progress: { 'tt1:S1E1': prog(10, 100, 300), 'tt1:S2E9': prog(20, 100, 100) }, removed: {} }),
    },
    /* D4. `lang` is lifted out of the structural comparison (it cannot exist on
     * the old side, so leaving it in would fail every progress fixture for one
     * known reason) and asserted here instead, by value. `pos`/`dur`/`at` still
     * have to agree exactly — the language rides along, it does not change who
     * won. A missing or wrong `lang` fails the run just as hard as a structural
     * difference would. */
    { id: 'progress-lang-carried-from-the-losing-record', expect: 'match', expectLang: { k: 'jpn' }, now: 1000,
      local: S({ history: [], progress: { k: prog(10, 100, 100, { lang: 'jpn' }) }, removed: {} }),
      remote: S({ history: [], progress: { k: prog(80, 100, 900) }, removed: {} }) },
    { id: 'progress-lang-on-the-winning-record', expect: 'match', expectLang: { k: 'eng' }, now: 1000,
      local: S({ history: [], progress: { k: prog(10, 100, 200, { lang: 'eng' }) }, removed: {} }),
      remote: S({ history: [], progress: { k: prog(50, 100, 100) }, removed: {} }) },
    { id: 'progress-lang-on-both-sides-newer-wins', expect: 'match', expectLang: { k: 'fra' }, now: 1000,
      local: S({ history: [], progress: { k: prog(10, 100, 100, { lang: 'jpn' }) }, removed: {} }),
      remote: S({ history: [], progress: { k: prog(80, 100, 900, { lang: 'fra' }) }, removed: {} }) },
    { id: 'progress-lang-across-many-keys', expect: 'match', expectLang: { a: 'eng', c: 'jpn' }, now: 1000,
      local: S({ history: [], progress: { a: prog(1, 100, 10, { lang: 'eng' }), b: prog(2, 100, 20) }, removed: {} }),
      remote: S({ history: [], progress: { c: prog(3, 100, 30, { lang: 'jpn' }) }, removed: {} }) },
    {
      /* D7 applies to `merge_mylist` too — `takes_the_slot` is shared — but 0.1.0
       * drops `mylist` entirely, so there is no old-side value for a same-id
       * equal-`at` collision to differ FROM. It would add no information beyond
       * this fixture, so it is recorded in DIVERGENCES.md and not duplicated here. */
      id: 'mylist-half-of-the-document',
      expect: { d: 'N-mylist', why: '`mylist`/`mylistRemoved` are new: 0.1.0`s Library had three fields, so `stores/library.ts` maintained a fourth hand-written copy of this same CRDT. The old core drops both keys entirely.' },
      now: 1000,
      local: S({ history: [item('a', 100)], progress: {}, removed: {}, mylist: [{ id: 'm', at: 100 }], mylistRemoved: {} }),
      remote: S({ history: [], progress: {}, removed: {}, mylist: [{ id: 'n', at: 200 }], mylistRemoved: {} }),
    },
  ],
  run(old, neu, f) {
    const rt = new old.LibraryRuntime();
    rt.hydrate(f.local);
    const [h, p, r] = splitRemote(f.remote);
    rt.pulled(h, p, r, f.now);
    const oldOut = JSON.parse(rt.snapshot_json());
    rt.free();

    const env = neu.__unwrap(neu.merge_library(f.local, f.remote, f.now), 'merge_library');
    return { oldOut, newOut: env.data, env, library: true, notes: [`N-envelope: new ok=${env.ok} warnings=[${env.codes.join(', ')}]`] };
  },
};

/* --------------------------------------------------------------------------
 * 5. record_watch
 * ------------------------------------------------------------------------ */

const recordWatch = {
  name: 'library_record_watch',
  old: 'new LibraryRuntime().hydrate(lib).record_watch(item) -> snapshot_json()',
  neu: 'library_record_watch(lib, item) -> envelope{ data: Library }',
  fixtures: [
    { id: 'upserts-and-floats-to-the-front', expect: 'match', lib: S({ history: [item('b', 200), item('a', 100)], progress: {}, removed: {} }), item: S(item('a', 300)) },
    { id: 'appends-a-brand-new-title', expect: 'match', lib: S({ history: [item('a', 100)], progress: {}, removed: {} }), item: S(item('z', 50)) },
    { id: 'clears-a-tombstone', expect: 'match', lib: S({ history: [], progress: {}, removed: { a: 500 } }), item: S(item('a', 900)) },
    { id: 'into-an-empty-library', expect: 'match', lib: S({}), item: S(item('a', 100)) },
    { id: 'overflows-the-history-cap', expect: 'match', lib: S({ history: bigHistory(60), progress: {}, removed: {} }), item: S(item('newest', 99999)) },
    { id: 'a-full-episode-shaped-entry', expect: 'match', lib: S({ history: [], progress: {}, removed: {} }), item: S(item('tt1', 500, { key: 'tt1:S2E3', season: 2, episode: 3, poster: 'p.jpg', year: '2019', type: 'series', rating: '8.4', ep: 'S2E3' })) },
    { id: 'an-unreadable-item-changes-nothing', expect: 'match', lib: S({ history: [item('a', 100)], progress: {}, removed: {} }), item: '{{{bad' },
    {
      id: 'an-item-with-a-numeric-id',
      expect: { d: 'N-lenient', why: '0.1.0 fails the item parse, returns "[]", and `heartLibrary.ts:46` then reads the un-updated snapshot back — a dropped write that looked like a success. The new core coerces the id and performs the write.' },
      lib: S({ history: [], progress: {}, removed: {} }), item: S({ id: 603, title: 'The Matrix', at: 5 }),
    },
  ],
  run(old, neu, f) {
    const rt = new old.LibraryRuntime();
    rt.hydrate(f.lib);
    rt.record_watch(f.item);
    const oldOut = JSON.parse(rt.snapshot_json());
    rt.free();
    const env = neu.__unwrap(neu.library_record_watch(f.lib, f.item), 'library_record_watch');
    return { oldOut, newOut: env.data, env, library: true, notes: [`N-envelope: new ok=${env.ok} warnings=[${env.codes.join(', ')}]`] };
  },
};

/* --------------------------------------------------------------------------
 * 6. library_remove
 * ------------------------------------------------------------------------ */

const libraryRemove = {
  name: 'library_remove',
  old: 'new LibraryRuntime().hydrate(lib).remove(id, now) -> snapshot_json()',
  neu: 'library_remove(lib, id, now) -> envelope{ data: Library }',
  fixtures: [
    { id: 'tombstones-and-drops-the-history-entry', expect: 'match', now: 200, target: 'a', lib: S({ history: [item('a', 100), item('b', 90)], progress: { a: prog(5, 100, 100), b: prog(5, 100, 90) }, removed: {} }) },
    { id: 'removing-an-absent-id-still-tombstones', expect: 'match', now: 200, target: 'ghost', lib: S({ history: [item('a', 100)], progress: {}, removed: {} }) },
    { id: 'removing-twice-updates-the-tombstone', expect: 'match', now: 900, target: 'a', lib: S({ history: [], progress: {}, removed: { a: 100 } }) },
    { id: 'a-similarly-named-title-is-untouched', expect: 'match', now: 200, target: 'tt1', lib: S({ history: [item('tt1', 100), item('tt10', 90)], progress: { tt1: prog(1, 100, 1), tt10: prog(2, 100, 2) }, removed: {} }) },
    {
      id: 'sweeps-episode-progress-keys',
      expect: { d: 'D3', why: 'The shell keys progress by media key (`id:S#E#`) but 0.1.0`s Remove did `progress.remove(&id)` and nothing else, so every episode`s resume position survived a series removal forever — counting against PROGRESS_CAP and resurrecting on re-add. The new core sweeps the `id:` prefix.' },
      now: 99, target: 'tt1',
      lib: S({ history: [item('tt1', 100)], progress: { tt1: prog(5, 100, 1), 'tt1:S1E1': prog(10, 100, 2), 'tt1:S2E9': prog(20, 100, 3), tt10: prog(30, 100, 4), 'tt10:S1E1': prog(40, 100, 5) }, removed: {} }),
    },
    {
      id: 'an-empty-id-is-refused',
      expect: { d: 'N-guard', why: 'An empty id would tombstone the empty string and — with D3`s prefix sweep — delete every key beginning ":". 0.1.0 happily tombstoned `""`. The new core refuses with ok:false and changes nothing.' },
      now: 200, target: '', lib: S({ history: [item('a', 100)], progress: { a: prog(1, 2, 3) }, removed: {} }),
    },
  ],
  run(old, neu, f) {
    const rt = new old.LibraryRuntime();
    rt.hydrate(f.lib);
    rt.remove(f.target, f.now);
    const oldOut = JSON.parse(rt.snapshot_json());
    rt.free();
    const env = neu.__unwrap(neu.library_remove(f.lib, f.target, f.now), 'library_remove');
    return { oldOut, newOut: env.data, env, library: true, notes: [`N-envelope: new ok=${env.ok}`] };
  },
};

/* --------------------------------------------------------------------------
 * 7. continue_watching
 * ------------------------------------------------------------------------ */

/* Driven with `hideFinished: true`, because that is 0.1.0's unconditional
 * behaviour and the comparison has to be like for like. The boundary's DEFAULT is
 * false — see api.rs — and that default is a shell-adoption decision, not a
 * behaviour the old core can be compared against. */
const continueWatching = {
  name: 'continue_watching',
  old: 'new LibraryRuntime().hydrate(lib).continue_watching_json() -> LibraryItem[]',
  neu: 'continue_watching(lib, {"hideFinished":true}) -> envelope{ data: [{item, key, fraction, resume}] }',
  fixtures: [
    { id: 'unfinished-titles-only', expect: 'match', lib: S({ history: [item('a', 300), item('b', 200), item('c', 100)], progress: { a: prog(30, 100, 1), b: prog(99, 100, 1) }, removed: {} }) },
    { id: 'no-progress-record-means-not-in-the-rail', expect: 'match', lib: S({ history: [item('a', 300), item('b', 200)], progress: { a: prog(30, 100, 1) }, removed: {} }) },
    { id: 'unknown-duration-is-excluded', expect: 'match', lib: S({ history: [item('a', 300)], progress: { a: prog(30, 0, 1) }, removed: {} }) },
    { id: 'empty-library', expect: 'match', lib: S({}) },
    { id: 'well-under-the-finished-line', expect: 'match', lib: S({ history: [item('a', 300)], progress: { a: prog(50, 100, 1) }, removed: {} }) },
    { id: 'well-over-the-finished-line', expect: 'match', lib: S({ history: [item('a', 300)], progress: { a: prog(98, 100, 1) }, removed: {} }) },
    {
      id: 'between-the-two-finished-thresholds',
      expect: { d: 'D5', why: '0.92 sits between 0.1.0`s `Progress::is_finished` cutoff of 0.9 and the shell`s long-standing PROGRESS_DONE of 0.94 (`history.ts:26`). 0.94 wins: it is the number users experience, and the 0.9 had no consumer. So this title leaves the rail on the old core and stays on it in the new one.' },
      lib: S({ history: [item('a', 300)], progress: { a: prog(92, 100, 1) }, removed: {} }),
    },
    {
      id: 'episode-keyed-progress',
      expect: { d: 'D3', why: '0.1.0 looked progress up by `item.id()`; the shell stores it under the media key (`id:S#E#`), so an episode never matched and the rail was always empty — which is exactly why `heartLibrary.ts:10-12` says the rail is still derived in the store. The new core looks up `media_key()`.' },
      lib: S({ history: [item('tt1', 300, { key: 'tt1:S1E4', season: 1, episode: 4 })], progress: { 'tt1:S1E4': prog(300, 1400, 1) }, removed: {} }),
    },
    {
      id: 'history-supplied-out-of-order',
      expect: { d: 'N-nosort', why: '0.1.0`s `hydrate` sorted and capped as a side effect of loading, so a caller could hand it unsorted history and get a sorted rail. The new `continue_watching` is a pure read that preserves the caller`s order — sorting belongs to `merge_library`/`record_watch`, which is where the shell already gets it.' },
      lib: S({ history: [item('a', 100), item('b', 300), item('c', 200)], progress: { a: prog(30, 100, 1), b: prog(30, 100, 1), c: prog(30, 100, 1) }, removed: {} }),
    },
  ],
  run(old, neu, f) {
    const rt = new old.LibraryRuntime();
    rt.hydrate(f.lib);
    const oldOut = JSON.parse(rt.continue_watching_json());
    rt.free();
    const env = neu.__unwrap(neu.continue_watching(f.lib, '{"hideFinished":true}'), 'continue_watching');
    // The new entry is a triple — the item, plus the two derived numbers the rail
    // used to compute itself in `ContinueRow.tsx`. Only `item` has a counterpart.
    const newOut = (env.data || []).map((e) => e.item);
    return {
      oldOut, newOut, env,
      notes: [`N-envelope: new also returns key/fraction/resume per entry: ${JSON.stringify((env.data || []).map((e) => ({ key: e.key, fraction: e.fraction, resume: e.resume })))}`],
    };
  },
};

/* --------------------------------------------------------------------------
 * 8. visible_rows
 * ------------------------------------------------------------------------ */

const visibleRows = {
  name: 'visible_rows',
  old: 'new CatalogRuntime().set_gating(c,p,s).hydrate_row_config(cfg) -> visible_rows_json()  [heartCatalog.ts:27-38]',
  neu: 'visible_rows(HEART_ROWS, gating, cfg) -> envelope{ data: string[] }',
  fixtures: [
    { id: 'nothing-installed-shows-nothing', expect: 'match', gating: { catalog: false, providers: false, studios: false }, cfg: {} },
    { id: 'everything-installed-no-config', expect: 'match', gating: { catalog: true, providers: true, studios: true }, cfg: {} },
    { id: 'catalog-only', expect: 'match', gating: { catalog: true, providers: false, studios: false }, cfg: {} },
    { id: 'providers-only', expect: 'match', gating: { catalog: false, providers: true, studios: false }, cfg: {} },
    { id: 'studios-only', expect: 'match', gating: { catalog: false, providers: false, studios: true }, cfg: {} },
    { id: 'explicit-false-hides-one-row', expect: 'match', gating: { catalog: true, providers: true, studios: true }, cfg: { top_movie: false } },
    { id: 'explicit-true-is-the-same-as-absent', expect: 'match', gating: { catalog: true, providers: true, studios: true }, cfg: { top_movie: true } },
    { id: 'studio-row-ignores-its-own-toggle', expect: 'match', gating: { catalog: false, providers: false, studios: true }, cfg: { studios: false } },
    { id: 'config-for-an-unknown-cat-is-inert', expect: 'match', gating: { catalog: true, providers: false, studios: false }, cfg: { not_a_row: false } },
    { id: 'every-catalog-row-turned-off', expect: 'match', gating: { catalog: true, providers: true, studios: true }, cfg: { trending_movie: false, trending_tv: false, top_movie: false, top_tv: false, trending_anime: false, top_anime: false } },
  ],
  run(old, neu, f) {
    const rt = new old.CatalogRuntime();
    rt.set_gating(f.gating.catalog, f.gating.providers, f.gating.studios);
    rt.hydrate_row_config(S(f.cfg));
    const oldOut = JSON.parse(rt.visible_rows_json());
    rt.free();
    const env = neu.__unwrap(neu.visible_rows(S(HEART_ROWS), S(f.gating), S(f.cfg)), 'visible_rows');
    return { oldOut, newOut: env.data, env, notes: ['N-table: the row table is an argument to the new core; it is fed 0.1.0`s own compiled-in HOME_ROWS so the comparison is about the RULE, not the list'] };
  },
};

/* --------------------------------------------------------------------------
 * coverage the harness cannot provide, stated rather than omitted
 * ------------------------------------------------------------------------ */

export const UNCOMPARABLE = [
  { fn: 'collection_addons_json', side: 'old-only', why: 'Folded into merge_official`s schema gate. Nothing in the new core answers this question separately, and nothing in Groloo-Web called it.' },
  { fn: 'set_progress', side: 'old-only', why: 'Deliberately absent from the new boundary (api.rs) — it fired every ~5s and copied the whole library through linear memory twice to write one key. The shell keeps that write; the rule stays testable as `Library::set_progress`.' },
  { fn: 'AddonRuntime.load_official / official_manifest_fetched / status', side: 'old-only', why: 'Effect-array plumbing for I/O the shell was performing itself. `official_payload_file` replaces the useful part of it and IS compared, above.' },
  { fn: 'AddonRuntime.toggle_addon', side: 'old-only', why: 'A one-line flip the shell already owns. It is USED by this harness to establish `install_at`, since 0.1.0 bound no other way to set it.' },
  { fn: 'CatalogRuntime.row_fetched / hero_fetched / load_home / snapshot_json / toggle_row', side: 'old-only', why: 'Row DATA plumbing. Rows are fetched and cached by the shell; only the gating RULE survived, and that is compared above.' },
  { fn: 'normalize_manifest_url / addon_base_url / validate_manifest / manifest_has_resource / addon_catalogs', side: 'new-only', why: 'Problem 5: `addon.rs` shipped in 0.1.0 with NO binding of any kind, so there is no old-core behaviour to differ from. Their real twins are in TypeScript (`addonClient.ts:45-53`, `stores/addons.ts:203`) and D6 must be asserted against THOSE, not here. ASSERTED NOW: `node tests/parity/run.mjs` imports and runs those files against this core, and its TWIN VERDICT is what authorises deleting any of them.' },
  { fn: 'stream_parse / catalog_metas / addon_resource_path / order_langs / rank_streams', side: 'new-only', why: 'The five add-on-protocol functions added in the second increment. Same reason as the row above — no 0.1.0 counterpart — and same answer: `node tests/parity/run.mjs` compares them against `addonClient.ts` and the two `DetailModal.tsx` expressions.' },
  { fn: 'mylist_toggle', side: 'new-only', why: '0.1.0`s Library had no My List. Its twin is `stores/library.ts:103-118`.' },
  { fn: 'resume_position', side: 'new-only', why: 'No 0.1.0 binding exposed `resume()`. Its twin is `history.ts::getResume`.' },
  { fn: 'core_version / core_constants', side: 'new-only', why: 'The whole point is that 0.1.0 could not answer either question — which is why a mispinned vendored folder was invisible.' },
];

/* --------------------------------------------------------------------------
 * the canary — proving the gate can go red
 * ------------------------------------------------------------------------ */

/* A differential harness that has never failed is indistinguishable from one
 * that cannot fail, and the second is worse than no harness at all: it produces a
 * green tick that a reviewer will read as evidence. So `run.mjs --prove` pushes
 * these four fixtures through the identical pipeline and asserts that the runner
 * classifies each one the way it claims it would.
 *
 * Every one of them is a REAL pair of core calls with a DELIBERATELY WRONG
 * expectation attached — not a mock, not a stub. If any of the four comes back
 * classified as anything other than `mustClassifyAs`, the runner's judgement is
 * broken and no result it prints means anything. */
const CANARY = {
  name: 'canary (--prove only)',
  old: 'real calls, deliberately wrong expectations',
  neu: 'each must be caught by the classifier it names',
  fixtures: [
    { id: 'a-genuine-difference-declared-as-a-match', mustClassifyAs: 'unexpected',
      ...{ expect: 'match', now: 99, target: 'tt1', lib: S({ history: [item('tt1', 100)], progress: { tt1: prog(5, 100, 1), 'tt1:S1E1': prog(10, 100, 2) }, removed: {} }) },
      kind: 'remove' },
    { id: 'an-agreement-declared-as-a-divergence', mustClassifyAs: 'stale-expectation',
      ...{ expect: { d: 'FAKE', why: 'there is no divergence here; the runner must say so' }, now: 200, target: 'a', lib: S({ history: [item('a', 100)], progress: {}, removed: {} }) },
      kind: 'remove' },
    { id: 'a-lang-that-is-not-what-was-promised', mustClassifyAs: 'lang-mismatch',
      expect: 'match', expectLang: { k: 'deu' }, now: 1000,
      local: S({ history: [], progress: { k: prog(10, 100, 100, { lang: 'jpn' }) }, removed: {} }),
      remote: S({ history: [], progress: { k: prog(80, 100, 900) }, removed: {} }), kind: 'merge' },
    { id: 'an-ok-flag-that-is-not-what-was-promised', mustClassifyAs: 'signal-mismatch',
      expect: 'match', expectOk: false, now: 1000,
      local: S({ history: [item('a', 100)], progress: {}, removed: {} }), remote: S({}), kind: 'merge' },
  ],
  run(old, neu, f) {
    return f.kind === 'remove'
      ? libraryRemove.run(old, neu, f)
      : mergeLibrary.run(old, neu, f);
  },
};

export { CANARY };

export const FAMILIES = [
  officialPayloadFile, mergeOfficial, reconcileInstallState,
  mergeLibrary, recordWatch, libraryRemove, continueWatching, visibleRows,
];
