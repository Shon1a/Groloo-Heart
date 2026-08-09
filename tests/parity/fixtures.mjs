/* The parity corpus: the TypeScript that ships, against the Rust that is meant to
 * replace it, one fixture at a time.
 *
 * ## What this corpus is FOR
 *
 * THE TWIN RULE: a TypeScript implementation may only be deleted once a
 * differential corpus proves the Rust replacement produces byte-identical output on
 * that corpus. No parity proof, no deletion. That makes this file the gate on
 * deleting `addonClient.ts:32-163`, `stores/addons.ts:203`, and the two-rule
 * manifest check at `stores/addons.ts:233` — and it makes a wrong `expect: 'match'`
 * here a silent production regression later, because the next agent acts on the
 * verdict this file produces.
 *
 * So every fixture states up front whether it expects the two to AGREE or to differ
 * in a specific, named way, and both directions fail. Nothing is compared loosely
 * and no comparison is relaxed to make the gate green: where the core deliberately
 * changed a behaviour, the fixture says so, says WHY, and still asserts the exact
 * new bytes.
 *
 * ## Thirteen families, and the twin each one names
 *
 *   normalize_manifest_url   × 2   server.js:1992 AND stores/addons.ts:203 — the
 *                                  same fixtures through both, because the whole
 *                                  point of D6 is that the two copies disagree and
 *                                  the core picked one of them. On two axes it
 *                                  picked NEITHER (D6-host, D-slash), which is only
 *                                  visible because both copies are driven.
 *   addon_base_url                 addonClient.ts:45
 *   manifest_has_resource          addonClient.ts:49
 *   stream_parse                   addonClient.ts:93-126, reached through
 *                                  collectAddonStreams with a recording fetch
 *   addon_resource_path            addonClient.ts:118 / :161 — asserted against the
 *                                  URL the twin ACTUALLY REQUESTED
 *   catalog_metas                  addonClient.ts:130-163, through fetchAddonCatalog
 *   addon_catalogs                 addonClient.ts:147-156
 *   order_langs                    addonClient.ts:32-38
 *   validate_manifest        × 2   server.js:1947 AND stores/addons.ts:233
 *   rank_streams                   DetailModal.tsx:648-649
 *   media key round trip           DetailModal.tsx:426 / :378 / :543 / :623
 *
 * The private functions (`mapAddonStream`, `detectQuality`, `extractSize`,
 * `parseStreamLangs`, `mapCatalogMeta`) are not exported and are therefore reached
 * the only honest way: through the exported I/O functions that call them, with the
 * network replaced and nothing else. That is also the more faithful comparison —
 * the shell does not call `mapAddonStream` either.
 *
 * ## Divergence ids
 *
 *   D6-*   the manifest-URL unification. The contract's own D6, split by axis.
 *   D-*    a difference this increment intended, argued at the call site in
 *          `api.rs` or in the module doc it names.
 *   U-*    a difference NOBODY PLANNED, surfaced by this corpus. Same convention as
 *          `tests/differential/`: it gets its own section in the report and its own
 *          line in the summary, because a divergence nobody chose is a behaviour
 *          change nobody reviewed.
 *
 *          THERE ARE CURRENTLY NONE, and how they left is the point. This corpus
 *          found three — U-slash, U-coerce, U-notobject — and every one of them was
 *          closed by CHANGING THE RUST, not by re-labelling the fixture: the trailing
 *          slash is trimmed before the `.json` test (so the client row is a `match`
 *          and the SERVER row is now the declared D-slash), the label triple coerces
 *          numbers, and `validate_manifest` pre-checks `Value::is_object` and emits
 *          the server's own sentence. A row may only move to `match` because the code
 *          moved. Nothing here was widened, and no fixture was retired.
 *   N-*    behaviour that is new rather than changed (the envelope, warnings), true
 *          of a whole family rather than of one fixture. */

import { assertSourceLine, assertNotACoreCallSite } from './twins.mjs';
import { unwrap } from './compare.mjs';

const S = JSON.stringify;

/* --------------------------------------------------------------------------
 * 1-2. normalize_manifest_url — the three-way ledger, asserted
 * ------------------------------------------------------------------------ */

/* `crate::addon`'s module doc contains a table claiming what each of the three
 * copies answers for ten inputs. This is that table, executable — extended to 25, because
 * a ledger row is a claim and the inputs it does not mention are where a unification
 * quietly loses a case. Each fixture
 * carries TWO expectations because the core matches one copy and not the other, and
 * a corpus that checked only the client would prove the core matches the copy it
 * deliberately does not match.
 *
 * The verdict for the SERVER family is the one the task names: the server's
 * behaviour is what the core adopted, so a divergence from the server is a real
 * finding and a divergence from the client is (mostly) the fix. */

const WHY_QUERY = 'The client\'s `/manifest\\.json$/` runs against the WHOLE string including the query, so it never matches a query-bearing URL and appends a second segment — `…?y=2/manifest.json`. The server tests `parsed.pathname` and inserts the segment BEFORE the query. The core took the server\'s rule. By add-on protocol convention the URLs that carry a query are exactly the CREDENTIALED ones, so the client copy fails precisely on the add-ons a user paid for; this is the strongest single argument in the increment and the reason `addon.rs` was worth exposing at all.';

const WHY_HOST = 'Neither TypeScript copy won outright, so the core is a third answer and this divergence from the server is deliberate. The server rejects EVERY schemeless string, which loses `v3-cinemeta.strem.io/manifest.json` — a paste people actually make. The client prepends `https://` to anything at all, which turns a typo into a URL that fails later as an opaque network error. The core accepts a schemeless string only when its authority is host-shaped (`addon::looks_like_host`: host characters only, plus a dot or a port), which accepts every input the client accepted FOR A REASON and rejects the ones it accepted by accident. The consequence for the server is that it will keep rejecting a paste the browser now accepts — the two are not unified on this axis and the ledger says so.';

const WHY_TYPO = 'The client prepends `https://` unconditionally, so `not-an-id` becomes `https://not-an-id/manifest.json`, `my addon` becomes a URL with a space in it, and `""` becomes `https:///manifest.json`. Each of those fails later, as a network error, with nothing anywhere to say it was never a URL. The core returns `null` with `ok:false` and `error.detail = "not a usable URL or hostname"` — the same answer the server gives, and a reportable one. A rejection the UI can surface beats a guess it cannot.';

const URL_LEDGER = [
  {
    id: 'an-already-canonical-url',
    url: 'https://a.co/x/manifest.json',
    server: 'match', client: 'match',
  },
  {
    id: 'a-configured-url-with-a-query',
    url: 'https://a.co/x/manifest.json?y=2',
    server: 'match',
    client: { d: 'D6-query', why: WHY_QUERY },
  },
  {
    id: 'a-base-url-with-a-query',
    url: 'https://a.co/addon?x=1',
    server: 'match',
    client: { d: 'D6-query', why: WHY_QUERY },
  },
  {
    id: 'a-config-key-packed-into-the-path',
    url: 'https://a.co/provider=KEY|other=K2,sort=quality/manifest.json',
    server: 'match', client: 'match',
  },
  {
    /* The one that costs a paying user their add-on: `|`-packed config AND a
     * query. The client appends a second segment after the query; the core keeps
     * the config bytes untouched and inserts the segment before the `?`. */
    id: 'a-config-key-and-a-query-together',
    url: 'https://a.co/provider=KEY|other=K2/addon?u=1',
    server: 'match',
    client: { d: 'D6-query', why: WHY_QUERY },
  },
  {
    id: 'a-json-path-that-is-not-manifest-json',
    url: 'https://a.co/x/foo.json',
    server: 'match',
    client: { d: 'D6-json', why: 'The client tests `/manifest\\.json$/`; the server and the core test the PATH for any `.json` suffix. An add-on served from `foo.json` is already a manifest URL, and the client turns it into `foo.json/manifest.json`, which 404s.' },
  },
  {
    id: 'a-native-client-deep-link',
    url: 'stremio://a.co/x',
    server: 'match', client: 'match',
  },
  {
    id: 'a-nonsense-scheme-is-still-rewritten',
    url: 'ftp://a.co/x',
    server: 'match', client: 'match',
  },
  {
    id: 'a-schemeless-host',
    url: 'a.co/x',
    server: { d: 'D6-host', why: WHY_HOST },
    client: 'match',
  },
  {
    id: 'the-paste-people-actually-make',
    url: 'v3-cinemeta.strem.io/manifest.json',
    server: { d: 'D6-host', why: WHY_HOST },
    client: 'match',
  },
  {
    id: 'a-typo-is-not-a-url',
    url: 'not-an-id',
    server: 'match',
    client: { d: 'D6-typo', why: WHY_TYPO },
    expectOk: false, expectErrorCode: 'invalid.url', expectErrorDetail: 'not a usable URL or hostname',
  },
  {
    id: 'a-sentence-is-not-a-url',
    url: 'my addon',
    server: 'match',
    client: { d: 'D6-typo', why: WHY_TYPO },
    expectOk: false, expectErrorCode: 'invalid.url',
  },
  {
    id: 'the-empty-string',
    url: '',
    server: 'match',
    client: { d: 'D6-typo', why: WHY_TYPO },
    expectOk: false, expectErrorCode: 'invalid.url',
  },
  {
    id: 'whitespace-only',
    url: '   ',
    server: 'match',
    client: { d: 'D6-typo', why: WHY_TYPO },
    expectOk: false, expectErrorCode: 'invalid.url',
  },
  {
    id: 'a-bare-scheme-with-no-authority',
    url: 'https://',
    server: 'match',
    client: { d: 'D6-typo', why: WHY_TYPO },
    expectOk: false, expectErrorCode: 'invalid.url',
  },
  { id: 'trailing-slashes-are-collapsed', url: 'https://a.co/x///', server: 'match', client: 'match' },
  { id: 'a-bare-origin', url: 'https://a.co', server: 'match', client: 'match' },
  { id: 'an-uppercase-scheme-is-not-rewritten', url: 'HTTPS://A.co/X', server: 'match', client: 'match' },
  { id: 'the-json-test-is-case-insensitive', url: 'https://a.co/x/MANIFEST.JSON', server: 'match', client: 'match' },
  {
    /* `strip_foreign_scheme`'s charset guard: the `://` here belongs to a query
     * parameter, not to a scheme. The core must not treat `a.co/x?u=https` as a
     * scheme name and rewrite the whole thing. */
    id: 'a-url-inside-the-query-string',
    url: 'https://a.co/x?u=https://y',
    server: 'match',
    client: { d: 'D6-query', why: WHY_QUERY },
  },
  {
    /* All THREE copies agree here, and all three are wrong in the same way: the
     * fragment is treated as part of the path, so the segment lands after it
     * (`…/x#frag/manifest.json`) and the fetch asks a server for a path it does
     * not have. Kept as a `match` because that is what the corpus measures — it
     * is not this increment's job to fix a bug the unification did not introduce
     * — but recorded here so the next person to read a `#`-related bug report
     * finds the fixture that already covers it. */
    id: 'a-fragment-is-not-a-query', url: 'https://a.co/x#frag', server: 'match', client: 'match',
  },
  { id: 'a-lan-host-with-a-port', url: 'http://192.168.1.5:11470/addon', server: 'match', client: 'match' },
  {
    /* WAS `U-slash`, THE ONE UNPLANNED DIVERGENCE THAT WAS A LIVE REGRESSION.
     *
     * It is now declared on the SERVER side and a `match` on the client's, and the
     * two halves of that swap are the whole record of the fix: `addon::normalize_manifest_url`
     * step 4 splits the query off, `trim_end_matches('/')` the PATH, and only then
     * tests it for `.json` — so the client's TRIM ORDER now runs ahead of the
     * server's PATHNAME TEST, and this input is a fixpoint for the first time in any
     * of the three copies.
     *
     * The client row moved to `match` because the RUST CHANGED, which is the only
     * reason a row is ever allowed to move: `addons.ts:203` answered
     * `https://a.co/manifest.json` before the fix and answers it now, and it is the
     * core that came to meet it. The neighbouring `trailing-slashes-are-collapsed`
     * row (`https://a.co/x///`) still matches BOTH copies, so the trim did not buy
     * this row by breaking that one. */
    id: 'a-canonical-url-with-a-trailing-slash',
    url: 'https://a.co/manifest.json/',
    server: {
      d: 'D-slash',
      why: 'THE CORE IS A THIRD ANSWER, and here it is the only correct one. `server.js:1992` tests `parsed.pathname` BEFORE trimming, sees `/manifest.json/`, fails its `\\.json($|\\?)` test and answers `https://a.co/manifest.json/manifest.json` — a URL that 404s. `addons.ts:203` trims first and answers the fetchable `https://a.co/manifest.json`. Neither copy has both halves: the server has the right TEST (pathname, so a query cannot defeat it — that is D6-query) and the client has the right ORDER (trim, then test). The core takes one from each, which is why it matches the client on this row and diverges from the server, and it is the only spelling under which `…/manifest.json/` is a FIXPOINT rather than a URL that grows a segment on every pass. The consequence for the server is the same one D6-host already carries: the two are not unified on this axis, and a manifest URL a user pastes with a trailing slash will keep 404ing server-side until `server.js:1992` trims before it tests. That is a server change, not a core one, and this row is where it is recorded.',
    },
    client: 'match',
  },
  { id: 'multiple-query-parameters', url: 'https://a.co/x/manifest.json?y=2&z=3', server: 'match', client: { d: 'D6-query', why: WHY_QUERY } },
  {
    /* Idempotency, which the client's twin fails by construction: its output grows
     * another `/manifest.json` on every pass. Fed the CORE's own answer here, so
     * the fixture states the property rather than a literal. */
    id: 'the-cores-own-answer-fed-back-in',
    url: 'https://a.co/addon/manifest.json?x=1',
    server: 'match',
    client: { d: 'D6-query', why: WHY_QUERY },
  },
];

const normalizeVsServer = {
  name: 'normalize_manifest_url  (vs server.js:1992)',
  twin: { at: 'Groloo-server/server/server.js:1992  normalizeManifestUrl', verdict: 'keep' },
  ts: 'server.js:1992 normalizeManifestUrl(raw) -> string|null   [lifted from the server\'s own source text]',
  rust: 'normalize_manifest_url(raw) -> envelope{ data: string|null }',
  fixtures: URL_LEDGER.map((f) => ({ ...f, expect: f.server })),
  run(t, core, f) {
    const tsOut = t.server.normalizeManifestUrl(f.url) ?? null;
    const env = unwrap(core.normalize_manifest_url(f.url), 'normalize_manifest_url');
    return {
      tsOut, coreOut: env.data, env,
      notes: [`client (addons.ts:203) answered ${S(t.store.normalizeManifestUrl(f.url))}`],
    };
  },
};

const normalizeVsClient = {
  name: 'normalize_manifest_url  (vs stores/addons.ts:203)',
  twin: { at: 'Groloo-Web/src/stores/addons.ts:203  normalizeManifestUrl', verdict: 'delete' },
  ts: 'stores/addons.ts:203 normalizeManifestUrl(raw) -> string   [imported and run]',
  rust: 'normalize_manifest_url(raw) -> envelope{ data: string|null }',
  fixtures: URL_LEDGER.map((f) => ({
    ...f,
    expect: f.client,
    // The envelope assertions belong to one family, not both — they are a
    // statement about the core, and asserting them twice would double-count.
    expectOk: undefined, expectErrorCode: undefined, expectErrorDetail: undefined,
  })),
  run(t, core, f) {
    const tsOut = t.store.normalizeManifestUrl(f.url);
    const env = unwrap(core.normalize_manifest_url(f.url), 'normalize_manifest_url');
    return { tsOut, coreOut: env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 3. addon_base_url
 * ------------------------------------------------------------------------ */

/* `addonClient.ts:45` is `String(manifestUrl||'').replace(/[^/]*$/,'')` and
 * `addon.rs:15` is `rfind('/')` — the same three lines in two languages, which plan
 * 06 §2.5 cites as the proof that "write it once" erodes without a gate. There is no
 * reason for any of these to differ and the fixtures say so; the value of the family
 * is that it will notice the day one of them does. */
const baseUrl = {
  name: 'addon_base_url  (vs addonClient.ts:45)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:45-47  addonBaseUrl', verdict: 'delete' },
  ts: 'addonBaseUrl(manifestUrl) -> string',
  rust: 'addon_base_url(manifestUrl) -> envelope{ data: string }',
  fixtures: [
    { id: 'a-manifest-url', expect: 'match', url: 'https://a.co/x/manifest.json' },
    { id: 'a-directory', expect: 'match', url: 'https://a.co/' },
    { id: 'a-bare-origin-has-no-directory', expect: 'match', url: 'https://a.co' },
    { id: 'a-configured-url-with-packed-keys', expect: 'match', url: 'https://a.co/provider=KEY|other=K2/manifest.json' },
    { id: 'a-url-with-a-query', expect: 'match', url: 'https://a.co/x/manifest.json?q=1' },
    { id: 'no-slash-at-all', expect: 'match', url: 'noslash' },
    { id: 'the-empty-string', expect: 'match', url: '' },
    { id: 'a-deep-path', expect: 'match', url: 'https://a.co/a/b/c/manifest.json' },
  ],
  run(t, core, f) {
    const env = unwrap(core.addon_base_url(f.url), 'addon_base_url');
    return { tsOut: t.client.addonBaseUrl(f.url), coreOut: env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 4. manifest_has_resource
 * ------------------------------------------------------------------------ */

const M = (extra) => ({ id: 'org.x', name: 'X', ...extra });

const hasResource = {
  name: 'manifest_has_resource  (vs addonClient.ts:49)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:49-53  addonHasResource', verdict: 'delete' },
  ts: 'addonHasResource({manifest}, resource, type?) -> boolean',
  rust: 'manifest_has_resource(manifestJson, resource, typ) -> envelope{ data: boolean }',
  /* Every fixture is run for all four (resource, type) pairs below, so one entry
   * is four comparisons — the type filter is where the two implementations have
   * the most room to disagree and a single probe per manifest would miss it. */
  probes: [['stream', 'movie'], ['stream', ''], ['catalog', ''], ['stream', 'series']],
  fixtures: [
    { id: 'the-short-resource-form', expect: 'match', manifest: M({ resources: ['stream'], types: ['movie'] }) },
    { id: 'the-full-resource-form', expect: 'match', manifest: M({ resources: [{ name: 'stream', types: ['series'] }], types: ['series'] }) },
    { id: 'a-catalog-only-addon', expect: 'match', manifest: M({ resources: ['catalog', 'meta'], types: ['movie', 'series'] }) },
    { id: 'no-resources-key-at-all', expect: 'match', manifest: M({}) },
    { id: 'explicitly-null-resources-and-types', expect: 'match', manifest: M({ resources: null, types: null }) },
    { id: 'a-resource-object-with-no-name', expect: 'match', manifest: M({ resources: [{ noName: 1 }, 'stream'], types: ['movie'] }) },
    { id: 'an-empty-types-list', expect: 'match', manifest: M({ resources: ['stream'], types: [] }) },
    { id: 'a-stream-addon-as-it-really-arrives', expect: 'match', manifest: M({ name: 'Streamer', resources: ['stream'], types: ['movie', 'series', 'anime'], idPrefixes: ['tt', 'kitsu'] }) },
  ],
  run(t, core, f) {
    // Compared as one record per manifest rather than four fixtures, so the report
    // names the manifest and the diff names the probe that disagreed.
    const tsOut = {};
    const coreOut = {};
    let env = null;
    for (const [resource, typ] of hasResource.probes) {
      const key = `${resource}/${typ || '*'}`;
      tsOut[key] = t.client.addonHasResource({ manifest: f.manifest }, resource, typ || undefined);
      env = unwrap(core.manifest_has_resource(S(f.manifest), resource, typ), 'manifest_has_resource');
      coreOut[key] = env.data;
    }
    return { tsOut, coreOut, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 5. stream_parse
 * ------------------------------------------------------------------------ */

/* Reached through `collectAddonStreams`, not around it. `mapAddonStream`,
 * `detectQuality`, `extractSize` and `parseStreamLangs` are module-private, and the
 * shell does not call them either — it calls this. The fetch is replaced and
 * nothing else is: the filter, the fan-out, the per-add-on catch and the
 * `.map(...).filter(Boolean)` are all the twin's own.
 *
 * The add-on record is the same one on both sides, so `source` is the same string
 * and any difference in it is a real one. */
const STREAM_ADDON = {
  id: 'org.streamer',
  url: 'https://a.co/provider=KEY|other=K2/manifest.json',
  manifest: { id: 'org.streamer', name: 'Streamer', resources: ['stream', 'catalog'], types: ['movie', 'series'], catalogs: [{ type: 'movie', id: 'top', name: 'Top' }] },
};

const streamFixture = (id, streams, extra = {}) => ({ id, expect: 'match', body: { streams }, ...extra });

const streamParse = {
  name: 'stream_parse  (vs addonClient.ts:93-126)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:65-111 + :122  detectQuality, extractSize, FLAG_LANG, parseStreamLangs, mapAddonStream', verdict: 'delete' },
  ts: 'collectAddonStreams(videoId, type) with a recording fetch -> AddonStream[]',
  rust: 'stream_parse(responseJson, addonName) -> envelope{ data: AddonStream[] }',
  fixtures: [
    /* ---- the shapes real add-ons send ---- */
    streamFixture('a-release-name-with-quality-and-size', [{ name: 'Streamer', title: 'Movie.2019.1080p.WEB.x264 2.3 GB', url: 'https://a.co/v.mp4' }]),
    streamFixture('name-title-and-description-all-set', [{ name: 'N', title: 'T', description: 'D', url: 'https://a.co/v.mp4' }]),
    streamFixture('no-label-at-all-falls-back-to-Source', [{ url: 'https://a.co/v.mp4' }]),
    streamFixture('several-sources-from-one-addon', [
      { name: 'Streamer\n4K', title: '🇬🇧 Movie 2160p HDR 12.4 GB', url: 'https://a.co/a.mkv' },
      { name: 'Streamer\n1080p', title: '🇷🇺 Movie 1080p 2.3 GB', url: 'https://a.co/b.mp4' },
      { name: 'Streamer\n720p', title: 'Movie 720p 900 MB', url: 'https://a.co/c.mp4' },
    ]),

    /* ---- language markers, including the malformed ones ---- */
    streamFixture('a-single-flag-emoji', [{ title: '🇬🇧 English 1080p', url: 'https://a.co/v.mp4' }]),
    streamFixture('THREE-consecutive-flags-yield-two-langs', [{ title: '🇬🇧🇺🇸🇷🇺 Multi 720p', url: 'https://a.co/v.mp4' }]),
    streamFixture('an-unmapped-country-degrades-to-its-own-letters', [{ title: '🇸🇪 Swedish 480p', url: 'https://a.co/v.mp4' }]),
    streamFixture('a-lone-regional-indicator-is-not-a-flag', [{ title: '🇬 half a flag 1080p', url: 'https://a.co/v.mp4' }]),
    streamFixture('duplicate-flags-are-deduped-first-seen-wins', [{ title: '🇷🇺🇺🇦🇷🇺 dual 1080p', url: 'https://a.co/v.mp4' }]),
    streamFixture('an-explicit-behaviorHints-lang-beats-the-flags', [{ name: '🇩🇪 German', url: 'https://a.co/v.mp4', behaviorHints: { lang: 'ka' } }]),
    streamFixture('an-EMPTY-behaviorHints-lang-falls-through-to-the-flags', [{ name: '🇩🇪 German', url: 'https://a.co/v.mp4', behaviorHints: { lang: '' } }]),
    streamFixture('no-language-signal-anywhere-guesses-en', [{ name: 'Source', url: 'https://a.co/v.mp4' }]),

    /* ---- the quality detector, quirks included ---- */
    streamFixture('a-bitrate-that-contains-1080', [{ title: 'Bitrate 10800 kbps', url: 'https://a.co/v.mp4' }]),
    streamFixture('an-id-that-contains-480', [{ title: 'id 114800', url: 'https://a.co/v.mp4' }]),
    streamFixture('4k-is-word-bounded-but-only-in-ascii', [
      { title: 'x4k pack', url: 'https://a.co/a.mp4' },
      { title: 'The 4K one', url: 'https://a.co/b.mp4' },
      { title: 'ე4k', url: 'https://a.co/c.mp4' },
    ]),
    streamFixture('uhd-counts-as-2160', [{ title: 'UHD Remux', url: 'https://a.co/v.mp4' }]),
    streamFixture('first-match-wins-not-highest', [{ title: '2160p downscale of 1080p', url: 'https://a.co/v.mp4' }]),
    streamFixture('360p-has-no-badge', [{ title: 'Potato 360p', url: 'https://a.co/v.mp4' }]),

    /* ---- the size extractor ---- */
    streamFixture('size-with-no-space', [{ title: 'Movie 700mb', url: 'https://a.co/v.mp4' }]),
    streamFixture('size-with-a-decimal', [{ title: 'Movie 1.5 GB', url: 'https://a.co/v.mp4' }]),
    streamFixture('the-leftmost-size-wins', [{ title: '2 GB / 3.5 GB', url: 'https://a.co/v.mp4' }]),
    streamFixture('no-size-at-all', [{ title: 'Movie 1080p', url: 'https://a.co/v.mp4' }]),
    streamFixture('a-size-with-a-non-breaking-space', [{ title: 'Movie 4.7 GB', url: 'https://a.co/v.mp4' }]),

    /* ---- HLS sniffing ---- */
    streamFixture('an-explicit-hls-streamType', [{ name: 'Live', url: 'https://a.co/live', behaviorHints: { streamType: 'hls' } }]),
    streamFixture('an-m3u8-with-a-query', [{ name: 'Live', url: 'https://a.co/x.m3u8?tok=1' }]),
    streamFixture('an-hls-path-segment', [{ name: 'Live', url: 'https://a.co/hls/x.mp4' }]),
    streamFixture('a-txt-playlist', [{ name: 'Live', url: 'https://a.co/x.txt' }]),

    /* ---- subtitles: three different absences ---- */
    streamFixture('subtitles-present-one-usable', [{ name: 'X', url: 'https://a.co/v.mp4', subtitles: [{ url: 'https://a.co/s.srt', lang: 'en' }, { lang: 'ru' }] }]),
    streamFixture('an-empty-subtitles-array-stays-an-empty-array', [{ name: 'X', url: 'https://a.co/v.mp4', subtitles: [] }]),
    streamFixture('an-array-whose-every-entry-is-unusable', [{ name: 'X', url: 'https://a.co/v.mp4', subtitles: [{ lang: 'ru' }] }]),

    /* ---- streams with nothing playable ---- */
    streamFixture('a-torrent-only-source-is-dropped-silently', [{ name: 'torrent only', infoHash: 'abc' }, { name: 'ok', url: 'https://a.co/v.mp4' }]),
    streamFixture('an-empty-url-string', [{ name: 'x', url: '' }]),
    streamFixture('null-streams', null),
    streamFixture('no-streams-key', undefined, { body: { ok: true } }),

    /* ---- the wrongly-typed fields, and the line between coercing and refusing ---- */

    /* WAS `U-coerce`. `Stream.name`/`title`/`description` now read through
     * `de::opt_string_or_number` — the wire type `LibraryItem.id` has used for exactly
     * this reason since the previous increment — so a numeric label is stringified
     * instead of failing the field, failing the `Stream` and being dropped by
     * `StreamsResponse`.
     *
     * The row is a `match` because the RUST CHANGED. Nothing here was relaxed: the
     * comparison is still exact bytes over BOTH streams, and the `expectWarning` that
     * used to ride along was REMOVED rather than kept and excused, because nothing is
     * dropped any more and a warning assertion that cannot fire is a fixture lying
     * about what it covers.
     *
     * Read it against `a-stream-whose-url-is-a-number` directly below, which is still
     * D-typed-url and still drops: the rule the port settled on is COERCE WHAT IS
     * DISPLAYED, REFUSE WHAT IS DEREFERENCED. A label the add-on wrote as `1080`
     * renders as "1080" and the source plays; a `url` written as `12345` would resolve
     * against the page origin and manufacture the broken-looking source that D-typed-url
     * exists to keep out of the list. Both rows have to stay for the pair to mean
     * anything — one of them alone reads as a preference rather than a rule. */
    streamFixture('a-stream-whose-name-is-a-number', [{ name: 5, url: 'https://a.co/v.mp4' }, { name: 'good', url: 'https://a.co/w.mp4' }]),

    /* THE ONE-FIELD-OVER ROWS, and the reason they are here rather than assumed.
     *
     * U-coerce was found on `name`, but `name` was never the only field with that
     * shape: every CARRIED field — one the core reads off the wire and hands back
     * without displaying it — had the same "a wrong type costs the RECORD" wiring,
     * and the twin reads all of them through `?.`, which cannot fail. `de::optional`
     * and `de::default_on_error` now make each of them cost the FIELD.
     *
     * None of these fields appears in `AddonStream`, so the twin's answer is the same
     * either way and every row below is an exact-bytes `match`. That is precisely what
     * makes them worth asserting: before the fix each one answered `[]` where the twin
     * answered a playable source, and NOTHING IN THIS CORPUS WOULD HAVE SAID SO — the
     * family's twin verdict would have read DELETE on a rule that silently deleted
     * sources. A fixture that can only ever agree is still the difference between a
     * proof and an assumption when the thing it pins is a regression that already
     * happened once.
     *
     * `url` is the stated exception and keeps its own row (D-typed-url, below): coerce
     * what is DISPLAYED, refuse what is DEREFERENCED. */
    streamFixture('a-videoSize-hint-written-as-a-string', [{ name: 'x', url: 'https://a.co/v.mp4', behaviorHints: { videoSize: '9.2 GB' } }]),
    streamFixture('a-notWebReady-hint-written-as-a-word', [{ name: 'x', url: 'https://a.co/v.mp4', behaviorHints: { notWebReady: 'yes' } }]),
    streamFixture('a-countryWhitelist-that-is-one-bare-country', [{ name: 'x', url: 'https://a.co/v.mp4', behaviorHints: { countryWhitelist: 'us' } }]),
    streamFixture('behaviorHints-that-is-not-an-object-at-all', [{ name: 'x', url: 'https://a.co/v.mp4', behaviorHints: 'hls' }]),
    streamFixture('a-fileIdx-written-as-a-string', [{ name: 'x', url: 'https://a.co/v.mp4', infoHash: 'abc', fileIdx: '2' }]),
    streamFixture('a-numeric-ytId-and-a-numeric-infoHash', [{ name: 'x', url: 'https://a.co/v.mp4', ytId: 5, infoHash: 7 }]),
    /* The hints the twin DOES read, wrongly typed. `bh.streamType === 'hls'` is false
     * for a number and `bh.lang ? [bh.lang] : …` is false for `0`, so the twin falls
     * through to the flag/`['en']` path — and the core has to fall through the same
     * way rather than refusing the hints object or the stream. */
    streamFixture('a-streamType-and-a-lang-of-the-wrong-type', [{ name: '🇩🇪 German', url: 'https://a.co/v.mp4', behaviorHints: { streamType: 7, lang: 0 } }]),
    {
      id: 'a-stream-whose-url-is-a-number',
      expect: {
        d: 'D-typed-url',
        why: 'The twin hands `url: 12345` — a NUMBER — straight through into the record the player receives, because `s.url || \'\'` is truthy for a non-zero number and nothing downstream re-checks the type. `<video src={12345}>` resolves against the page origin and fails as a 404 the user reads as "this source is broken". The core refuses the record. This one is the port working as intended: an unplayable source that never appears is better than one that appears and fails.',
      },
      body: { streams: [{ name: 'x', url: 12345 }] },
      expectWarning: 'dropped.bad_item',
    },
    {
      /* The response body is not JSON at all. Both sides answer `[]`; what the
       * core adds is the ability to SAY so. `fetchAddonJSON` throws inside
       * `collectAddonStreams`'s per-add-on `catch { return [] }`, which is one of
       * the four blind catches this increment exists to make reportable. */
      id: 'a-response-that-is-not-json',
      expect: 'match',
      expectOk: false, expectErrorCode: 'parse.input',
      raw: '<<<not json>>>',
    },
  ],
  async run(t, core, f) {
    const raw = f.raw ?? S(f.body);
    t.setInstalled([STREAM_ADDON]);
    t.plan(f.raw !== undefined ? { text: f.raw } : { body: f.body });
    const tsOut = await t.client.collectAddonStreams('tt0903747:1:4', 'series');
    const env = unwrap(core.stream_parse(raw, STREAM_ADDON.manifest.name), 'stream_parse');
    return {
      tsOut, coreOut: env.data, env,
      notes: [`the twin requested ${S(t.requested()[0])}`, `warnings [${env.codes.join(', ')}]`],
    };
  },
};

/* --------------------------------------------------------------------------
 * 6. addon_resource_path
 * ------------------------------------------------------------------------ */

/* The only place this rule exists in TypeScript is inside two `fetch` calls, so
 * the assertion is against the URL the twin ACTUALLY REQUESTED — captured by the
 * recording fetch, not reconstructed. That makes this the strongest family in the
 * corpus: it compares the core's answer to a real network request the shipping code
 * built, character for character, percent-encoding included.
 *
 * `encodeURIComponent`'s unreserved set is the thing under test. Add-ons accept both
 * the raw and the encoded form, so this is a byte-identity requirement rather than a
 * correctness one — which is exactly what the twin rule needs. */
const resourcePath = {
  name: 'addon_resource_path  (vs addonClient.ts:118 and :161)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:118 and :161  the two hand-built path expressions', verdict: 'delete' },
  ts: 'the URL collectAddonStreams / fetchAddonCatalog actually requested',
  rust: 'addonBaseUrl + addon_resource_path(resource, mediaType, id) -> envelope{ data: string }',
  fixtures: [
    { id: 'a-film', expect: 'match', resource: 'stream', type: 'movie', id: 'tt0111161' },
    { id: 'an-episode-in-wire-form', expect: 'match', resource: 'stream', type: 'series', id: 'tt0903747:1:4' },
    { id: 'a-tmdb-keyed-episode', expect: 'match', resource: 'stream', type: 'series', id: '1399:2:9' },
    { id: 'an-id-with-a-space', expect: 'match', resource: 'stream', type: 'movie', id: 'tt1 x' },
    { id: 'an-id-with-a-pipe-and-a-colon', expect: 'match', resource: 'stream', type: 'series', id: 'kitsu:1|2' },
    { id: 'an-id-with-the-unreserved-set', expect: 'match', resource: 'stream', type: 'movie', id: "a-_.!~*'()z" },
    { id: 'a-non-ascii-id', expect: 'match', resource: 'stream', type: 'movie', id: 'ფილმი' },
    { id: 'a-catalog-id', expect: 'match', resource: 'catalog', type: 'movie', id: 'top' },
    { id: 'a-catalog-id-with-a-slash', expect: 'match', resource: 'catalog', type: 'series', id: 'anime/top' },
  ],
  async run(t, core, f) {
    t.setInstalled([STREAM_ADDON]);
    t.plan({ body: f.resource === 'stream' ? { streams: [] } : { metas: [] } });
    const base = t.client.addonBaseUrl(STREAM_ADDON.url);
    if (f.resource === 'stream') {
      await t.client.collectAddonStreams(f.id, f.type);
    } else {
      await t.client.fetchAddonCatalog({ addonId: STREAM_ADDON.id, addonName: 'Streamer', type: f.type, id: f.id, name: 'x', base });
    }
    const asked = t.requested()[0];
    const env = unwrap(core.addon_resource_path(f.resource, f.type, f.id), 'addon_resource_path');
    return { tsOut: asked, coreOut: base + env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 7. catalog_metas
 * ------------------------------------------------------------------------ */

const CAT = { addonId: 'org.streamer', addonName: 'Streamer', type: 'movie', id: 'top', name: 'Top', base: 'https://a.co/provider=KEY|other=K2/' };
const metaFixture = (id, metas, extra = {}) => ({ id, expect: 'match', body: { metas }, ...extra });

const catalogMetas = {
  name: 'catalog_metas  (vs addonClient.ts:130-163)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:128-141 + :162  CatalogMeta, mapCatalogMeta, the poster filter', verdict: 'delete' },
  ts: 'fetchAddonCatalog(catalog) with a recording fetch -> MediaItem[]',
  rust: 'catalog_metas(responseJson) -> envelope{ data: MediaItem[] }',
  fixtures: [
    metaFixture('a-full-card', [{ id: 'tt1', name: 'A', poster: 'p.jpg', releaseInfo: '2008-2013', imdbRating: '8.75', genres: ['Drama', 'Crime'], type: 'movie' }]),
    metaFixture('a-card-with-only-the-required-fields', [{ id: 'tt1', name: 'A', poster: 'p.jpg' }]),
    metaFixture('cards-with-no-poster-are-dropped', [{ id: 'tt1', name: 'A' }, { id: 'tt2', name: 'B', poster: 'p.jpg' }]),
    metaFixture('an-explicitly-series-card', [{ id: 'tt1', name: 'A', poster: 'p.jpg', type: 'series' }]),

    /* ---- wrongly-typed fields ---- */
    metaFixture('a-numeric-id', [{ id: 603, name: 'The Matrix', poster: 'p.jpg', year: 1999 }]),
    metaFixture('a-numeric-name', [{ id: 'tt1', name: 2001, poster: 'p.jpg' }]),
    metaFixture('an-empty-name-falls-back-to-Untitled', [{ id: 'tt1', name: '', poster: 'p.jpg' }]),
    metaFixture('a-numeric-poster', [{ id: 'tt1', name: 'A', poster: 5 }]),
    metaFixture('a-numeric-year', [{ id: 'tt1', name: 'A', poster: 'p.jpg', year: 1999 }]),
    metaFixture('a-numeric-rating', [{ id: 'tt1', name: 'A', poster: 'p.jpg', imdbRating: 8.25 }]),
    metaFixture('a-rating-that-is-not-a-number', [{ id: 'tt1', name: 'A', poster: 'p.jpg', imdbRating: 'N/A' }]),
    metaFixture('a-rating-of-zero-as-a-string', [{ id: 'tt1', name: 'A', poster: 'p.jpg', imdbRating: '0' }]),
    metaFixture('an-EMPTY-genres-array-is-truthy-in-javascript', [{ id: 'tt1', name: 'A', poster: 'p.jpg', genres: [], genre: 'Fallback' }]),
    metaFixture('the-singular-genre-spelling', [{ id: 'tt1', name: 'A', poster: 'p.jpg', genre: 'Comedy' }]),
    metaFixture('genres-that-are-not-strings', [{ id: 'tt1', name: 'A', poster: 'p.jpg', genres: [5, 6] }]),
    metaFixture('releaseInfo-beats-year', [{ id: 'tt1', name: 'A', poster: 'p.jpg', releaseInfo: '2015', year: 1999 }]),
    metaFixture('a-releaseInfo-range-is-sliced-not-parsed', [{ id: 'tt1', name: 'A', poster: 'p.jpg', releaseInfo: '2008-2013' }]),

    /* ---- absences ---- */
    metaFixture('null-metas', null),
    metaFixture('no-metas-key', undefined, { body: {} }),
    metaFixture('an-empty-catalog', []),

    /* ---- the split this increment exists to close ---- */
    {
      id: 'an-addon-that-labels-a-show-tv',
      expect: {
        d: 'D-kind',
        why: 'THE `movie|tv` vs `movie|series` SPLIT, and the reason this increment names it. The twin is `m.type === \'series\' ? \'series\' : \'movie\'` — a strict equality against ONE token — so an add-on that labels a show `"tv"` gets a card typed `movie`. That is not cosmetic: the card renders with a film\'s chrome, and its streams are then requested from `stream/movie/…`, which returns nothing, so the title appears in a row and has no sources. The core routes every type token through `MediaKind::from_wire`, where `tv`/`series`/`show` are all series and everything else is a movie. The same three-armed tolerance is what lets a `localStorage` record written by a build that no longer exists keep working.',
      },
      body: { metas: [{ id: 'tt1', name: 'A', poster: 'p.jpg', type: 'tv' }] },
    },
    {
      id: 'an-addon-that-labels-a-show-show',
      expect: { d: 'D-kind', why: 'The same rule on the third token. `show` is what several anime add-ons send.' },
      body: { metas: [{ id: 'tt1', name: 'A', poster: 'p.jpg', type: 'show' }] },
    },
    {
      id: 'one-unreadable-meta-among-good-ones',
      expect: {
        d: 'D-lenient',
        why: 'A `null` in the `metas` array makes `mapCatalogMeta` throw on `m.genres`, the throw escapes to `fetchAddonCatalog`\'s `catch { return [] }`, and the user loses THE WHOLE ROW to one bad record. `catalog.rs` reads the array with `de::lenient_vec`: the bad entry costs itself and the row renders. This is the same rule the library merge already got in the previous increment, one layer down.',
      },
      body: { metas: [null, { id: 'tt2', name: 'B', poster: 'p.jpg' }] },
    },
    {
      id: 'a-response-that-is-not-json',
      expect: 'match',
      expectOk: false, expectErrorCode: 'parse.input',
      raw: '}{ not json',
    },
  ],
  async run(t, core, f) {
    const raw = f.raw ?? S(f.body);
    t.plan(f.raw !== undefined ? { text: f.raw } : { body: f.body });
    const tsOut = await t.client.fetchAddonCatalog(CAT);
    const env = unwrap(core.catalog_metas(raw), 'catalog_metas');
    return { tsOut, coreOut: env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 8. addon_catalogs
 * ------------------------------------------------------------------------ */

const CATALOG_RECORDS = {
  full: [{ id: 'org.a', url: 'https://a.co/cfg=1|b/manifest.json', manifest: { id: 'org.a', name: 'Alpha', resources: ['catalog'], types: ['movie'], catalogs: [{ type: 'movie', id: 'top', name: 'Top' }, { type: 'series', id: 'new', name: 'New' }] } }],
  mixed: [
    { id: 'org.a', url: 'https://a.co/manifest.json', manifest: { id: 'org.a', name: 'Alpha', resources: ['catalog', 'stream'], types: ['movie'], catalogs: [{ type: 'movie', id: 'top', name: 'Top' }] } },
    { id: 'org.b', url: 'https://b.co/manifest.json', manifest: { id: 'org.b', name: 'Beta', resources: ['stream'], types: ['movie'] } },
  ],
  unnamedCatalog: [{ id: 'org.a', url: 'https://a.co/manifest.json', manifest: { id: 'org.a', name: 'Alpha', resources: ['catalog'], types: ['movie'], catalogs: [{ type: 'movie', id: 'top' }] } }],
  unnamedAddon: [{ id: 'org.a', url: 'https://a.co/manifest.json', manifest: { id: 'org.a', resources: ['catalog'], types: ['movie'], catalogs: [{ type: 'movie', id: 'top', name: 'Top' }] } }],
  none: [],
};

const WHY_DISPLAY_FALLBACK = 'The twin invents a display string when the add-on supplies none — `c.name || a.manifest?.name || \'Add-on\'` — and the core does not: an absent catalog name is an ABSENT FIELD in the response, and an absent add-on name is `""`. That is plan 06 §3\'s rule (a rendered string is never an FFI crossing, because "Add-on" is a translatable word and the core has no locale), and it is the same reason `langName` stays in the shell. THE CONSEQUENCE IS A CALL-SITE OBLIGATION, not a free win: whoever deletes `listAddonCatalogs` must apply `name ?? addonName ?? t(\'addon.fallbackName\')` at the row, or catalogs from an add-on that declares no name will render with an empty label. Deleting the twin without that line is a visible regression.';

const addonCatalogs = {
  name: 'addon_catalogs  (vs addonClient.ts:147-156)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:147-156  listAddonCatalogs', verdict: 'delete', obligation: 'the call site must apply `name ?? addonName` — see D-display' },
  ts: 'listAddonCatalogs() over the real zustand store -> AddonCatalog[]',
  rust: 'addon_catalogs(recordsJson) -> envelope{ data: CatalogEntry[] }',
  fixtures: [
    { id: 'two-catalogs-from-one-addon', expect: 'match', records: CATALOG_RECORDS.full },
    { id: 'an-addon-with-no-catalog-resource-is-skipped', expect: 'match', records: CATALOG_RECORDS.mixed },
    { id: 'nothing-installed', expect: 'match', records: CATALOG_RECORDS.none },
    { id: 'a-catalog-with-no-name', expect: { d: 'D-display', why: WHY_DISPLAY_FALLBACK }, records: CATALOG_RECORDS.unnamedCatalog },
    { id: 'an-addon-with-no-name', expect: { d: 'D-display', why: WHY_DISPLAY_FALLBACK }, records: CATALOG_RECORDS.unnamedAddon },
  ],
  run(t, core, f) {
    t.setInstalled(f.records);
    const env = unwrap(core.addon_catalogs(S(f.records)), 'addon_catalogs');
    return { tsOut: t.client.listAddonCatalogs(), coreOut: env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 9. order_langs
 * ------------------------------------------------------------------------ */

const WHY_COLLATE = '`api.rs` declares this risk and this is where it is measured. The twin tie-breaks with `String.prototype.localeCompare`, which is ICU collation: case-insensitive-ish, accent-aware, locale-dependent. The core uses byte order. They agree on every code the app can produce today — `orderLangs` is fed `AddonStream.langs`, which `stream.rs` builds from `behaviorHints.lang` or from a flag emoji, and both produce `[a-z]{2}` — and disagree the moment anything else appears. Byte order is the right choice for a core that has no locale and must answer identically on LG, Android and the browser; `localeCompare` on a webOS 4 device would not even be the same ordering as on Chrome. The exposure is bounded by what can reach this function, which is why it is a declared risk and not a blocker.';

const orderLangs = {
  name: 'order_langs  (vs addonClient.ts:32-38)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:30-38  LANG_ORDER, orderLangs', verdict: 'delete' },
  ts: 'orderLangs(langs) -> string[]',
  rust: 'order_langs(langsJson) -> envelope{ data: string[] }',
  fixtures: [
    { id: 'one-language', expect: 'match', langs: ['en'] },
    { id: 'the-four-preferred-codes-shuffled', expect: 'match', langs: ['uk', 'ru', 'ka', 'en'] },
    { id: 'preferred-before-alphabetical', expect: 'match', langs: ['zz', 'aa', 'en'] },
    { id: 'duplicates-are-removed', expect: 'match', langs: ['en', 'en', 'ru', 'ru'] },
    { id: 'nothing-at-all', expect: 'match', langs: [] },
    { id: 'every-code-a-flag-emoji-can-produce', expect: 'match', langs: ['nl', 'pl', 'tr', 'zh', 'ko', 'ja', 'pt', 'es', 'it', 'de', 'fr', 'ka', 'uk', 'ru', 'en', 'se'] },
    { id: 'the-empty-string-as-a-code', expect: 'match', langs: ['', 'en'] },
    { id: 'a-three-letter-code', expect: 'match', langs: ['eng', 'en'] },
    {
      id: 'a-mixed-case-pair',
      expect: { d: 'D-collate', why: WHY_COLLATE },
      langs: ['B', 'a'],
    },
    {
      id: 'an-accented-code',
      expect: { d: 'D-collate', why: WHY_COLLATE },
      langs: ['ä', 'z'],
    },
    { id: 'the-same-code-in-two-cases', expect: 'match', langs: ['EN', 'en'] },
  ],
  run(t, core, f) {
    const env = unwrap(core.order_langs(S(f.langs)), 'order_langs');
    return { tsOut: t.client.orderLangs(f.langs), coreOut: env.data, env, notes: [] };
  },
};

/* --------------------------------------------------------------------------
 * 10-11. validate_manifest
 * ------------------------------------------------------------------------ */

const VALIDATE_CASES = [
  { id: 'a-usable-manifest', manifest: { id: 'org.x', name: 'N', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'no-resources-key', manifest: { id: 'org.x', name: 'N' }, serverExpect: 'match' },
  { id: 'an-empty-resources-list', manifest: { id: 'org.x', name: 'N', resources: [], types: ['movie'] }, serverExpect: 'match' },
  { id: 'an-empty-types-list', manifest: { id: 'org.x', name: 'N', resources: ['stream'], types: [] }, serverExpect: 'match' },
  { id: 'an-empty-id', manifest: { id: '', name: 'N', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'an-id-that-starts-with-a-dash', manifest: { id: '-bad', name: 'N', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'no-id-at-all', manifest: { name: 'N', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'a-whitespace-only-name', manifest: { id: 'org.x', name: '   ', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'a-name-over-200-characters', manifest: { id: 'org.x', name: 'x'.repeat(201), resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  { id: 'no-name-at-all', manifest: { id: 'org.x', resources: ['stream'], types: ['movie'] }, serverExpect: 'match' },
  {
    /* WAS `U-notobject`. `api::validate_manifest` now parses to `serde_json::Value`
     * first and answers `server.js:1947`'s own first sentence — "Manifest is not a
     * JSON object" — before it ever tries `AddonManifest`, so all FIVE of the
     * server's messages are byte-identical and `api.rs`'s "the detail carries the
     * server's message verbatim" is a claim rather than an aspiration.
     *
     * `expectOk`/`expectErrorCode` are new here and are the half the projected
     * comparison cannot see. `run()` projects the core to `env.ok ? null :
     * env.error.detail`, so a REGRESSION to the old behaviour would surface as a
     * wrong sentence — but a regression to some OTHER code carrying the right
     * sentence would not, and the whole point of the fix is that this input now
     * reaches the manifest rule (`invalid.manifest`) instead of dying in the parser
     * (`parse.input`). Asserting the code is what pins that.
     *
     * NOT JSON AT ALL is a different row and deliberately still answers
     * `parse.input` with serde's message: there is no document there to have an
     * opinion about. ONE WORDING DIFFERENCE SURVIVES AND IS UNASSERTED — a JSON
     * ARRAY is `typeof 'object'` in JS, so the server admits it to its `id` rule and
     * says `Manifest "id" is missing or malformed` where this says `Manifest is not
     * a JSON object`. Same verdict, same code, different sentence. It is documented
     * in `api::validate_manifest` and it is NOT covered here: adding an array
     * fixture means declaring that difference, and this corpus does not get to
     * record a divergence as a silent match. */
    id: 'a-manifest-that-is-not-an-object',
    manifest: null,
    serverExpect: 'match',
    expectOk: false, expectErrorCode: 'invalid.manifest', expectErrorDetail: 'Manifest is not a JSON object',
  },
];

const validateVsServer = {
  name: 'validate_manifest  (vs server.js:1947)',
  twin: { at: 'Groloo-server/server/server.js:1947  validateManifest', verdict: 'keep' },
  ts: 'server.js:1947 validateManifest(m) -> string|null   [lifted from the server\'s own source text]',
  rust: 'validate_manifest(manifestJson) -> envelope{ ok, error.detail }',
  fixtures: VALIDATE_CASES.map((f) => ({ ...f, expect: f.serverExpect })),
  run(t, core, f) {
    const env = unwrap(core.validate_manifest(S(f.manifest)), 'validate_manifest');
    // Both sides projected to "the reason it is unusable, or null". The server
    // returns the message directly; the core carries it in `error.detail`.
    return {
      tsOut: t.server.validateManifest(f.manifest) ?? null,
      coreOut: env.ok ? null : env.error.detail,
      env,
      notes: [`core error.code=${env.ok ? '(none)' : env.error.code}`],
    };
  },
};

const WHY_VALIDATE = '`addons.ts:233` implements TWO of the four rules (`id` and `name` present) under ONE message, so a manifest with a malformed id and a manifest with no `types` produce the same useless sentence — and a manifest with no `resources` or no `types` is INSTALLED, becomes a row in the add-ons screen, is asked for streams, and answers nothing. The core runs all four rules with the server\'s four distinct messages, so the browser and the server now agree about what an add-on is; before this, an add-on the server refused could still be installed from the browser. This divergence is the fix, and it is the one that changes what a user can install.';

/* The CLIENT's copy is not a function — it is `if (!manifest || !manifest.id ||
 * !manifest.name) throw new Error('Not a valid add-on manifest')` inside
 * `install()`, between the fetch and the localStorage write. So it is driven the
 * only way it can be: through `install()`, with the manifest served by the
 * recording fetch. The store is real, `localStorage` is the harness's, and the
 * signed-out auth substitute keeps the write local. */
const validateVsClient = {
  name: 'validate_manifest  (vs stores/addons.ts:233, driven through install())',
  twin: { at: 'Groloo-Web/src/stores/addons.ts:233  the inline `!manifest.id || !manifest.name` check', verdict: 'delete' },
  ts: 'useAddons.install(url) with a recording fetch -> accepted | rejected',
  rust: 'validate_manifest(manifestJson) -> envelope{ ok }',
  fixtures: VALIDATE_CASES.filter((f) => f.manifest !== null).map((f) => {
    const twoRulesPass = !!(f.manifest.id && f.manifest.name);
    const fourRulesPass = !!(f.manifest.id && /^[a-z0-9][a-z0-9._-]{0,200}$/i.test(f.manifest.id)
      && typeof f.manifest.name === 'string' && f.manifest.name.trim() && f.manifest.name.length <= 200
      && Array.isArray(f.manifest.resources) && f.manifest.resources.length
      && Array.isArray(f.manifest.types) && f.manifest.types.length);
    return {
      ...f,
      expect: twoRulesPass === fourRulesPass ? 'match' : { d: 'D-validate', why: WHY_VALIDATE },
    };
  }),
  async run(t, core, f) {
    t.setInstalled([]);
    t.plan({ body: f.manifest });
    let accepted = true;
    try { await t.store.useAddons.getState().install('https://a.co/x/manifest.json'); } catch { accepted = false; }
    const env = unwrap(core.validate_manifest(S(f.manifest)), 'validate_manifest');
    return {
      tsOut: { accepted }, coreOut: { accepted: env.ok }, env,
      notes: [`core says ${env.ok ? 'usable' : S(env.error.detail)}`],
    };
  },
};


/* --------------------------------------------------------------------------
 * 12. rank_streams
 * ------------------------------------------------------------------------ */

/* `DetailModal.tsx:648-649` is an expression inside a React component, not a
 * function and not exported. It is transcribed below and PINNED to its source line
 * by `assertSourceLine`, which re-reads the real file on every run: if that line
 * changes, this family fails rather than silently comparing against a fossil.
 *
 * The transcription calls the REAL `qualityRank` — the only part of the expression
 * that carries a rule — so the only thing written out here is the filter-and-sort
 * shape itself. */
const SHELL_SORT_SOURCE = '.sort((a, b) => qualityRank(b.quality) - qualityRank(a.quality))';
const SHELL_FILTER_SOURCE = '.filter((s) => !lang || s.langs.includes(lang))';

function shellShown(qualityRank, list, lang) {
  return list
    .filter((s) => !lang || s.langs.includes(lang))
    .sort((a, b) => qualityRank(b.quality) - qualityRank(a.quality));
}

const stream = (label, quality, size, langs = ['en'], url = 'https://a.co/v.mp4') => ({ source: 'X', label, quality, size, kind: 'url', url, langs });

const WHY_RANK_SIZE = 'The twin sorts on ONE key — the five-value `qualityRank` of a badge string — and `Array.prototype.sort` is stable, so two 1080p sources keep add-on fan-out order and the user gets whichever add-on answered first. The core\'s `SortKey` continues past resolution into HDR, audio codec, video codec and finally file size, because within one resolution bucket a byte count is the only bitrate proxy available (across buckets it is noise, which falls out of the field order rather than needing a rule). Here that flips a 1 GB re-encode below an 8 GB remux of the same height. This divergence is the entire point of `rank_streams`: a comparison that agreed with the twin would mean the ranker reads nothing the badge did not already say.';

const WHY_RANK_FILTER = 'STRUCTURAL, and deliberate. The twin FILTERS — a stream whose `langs` do not include the wanted one is removed from the array and the UI never sees it. The core never drops anything: every input stream comes back, with `blocked`/`blockedBy` set, and `preferLangs` is an ORDERING key rather than a filter, because a source in the wrong language is still a source and the alternative is an empty list with nothing to explain it. This is the same rule that makes `VideoPlayer.tsx:190`\'s "Source unavailable" wrong on a 4K HEVC stream a TV panel decodes fine: the shell must be able to tell "this device can play none of these" from "the add-ons returned nothing", and only a response that carries the refused streams can do that. The consumer changes shape here — `rank_streams` returns indices and verdicts, not a filtered array — so this is the one twin whose deletion is a call-site rewrite rather than a substitution.';

const rankStreams = {
  name: 'rank_streams  (vs DetailModal.tsx:648-649)',
  twin: { at: 'Groloo-Web/src/lib/addonClient.ts:40-41 qualityRank + DetailModal.tsx:648 and :649', verdict: 'rewrite', obligation: 'rank_streams answers {ranked, summary} with indices and blocked reasons, not a filtered array — the two call sites are a UI change, not a substitution' },
  ts: 'list.filter(langs).sort(qualityRank desc)   [transcribed from JSX, pinned to its source line]',
  rust: 'rank_streams(streamsJson, "{}") -> envelope{ data: { ranked, summary } }, projected back to streams',
  fixtures: [
    { id: 'quality-is-the-only-signal', expect: 'match', list: [stream('720p A', '720p', null), stream('2160p B', '4K', null), stream('1080p C', '1080p', null)], lang: '' },
    { id: 'an-unrecognised-badge-sorts-last', expect: 'match', list: [stream('mystery', '', null), stream('480p x', '480p', null)], lang: '' },
    { id: 'a-360p-source-has-no-badge-either', expect: 'match', list: [stream('360p x', '', null), stream('mystery', '', null)], lang: '' },
    { id: 'everything-equal-keeps-fan-out-order', expect: 'match', list: [stream('a 1080p', '1080p', null), stream('b 1080p', '1080p', null)], lang: '' },
    { id: 'one-source-only', expect: 'match', list: [stream('solo 1080p', '1080p', '2 GB')], lang: '' },
    {
      /* `rank::RankCandidate`'s doc promises that both the MAPPED record and the RAW
       * wire shape deserialise into it, and `api::rank_streams` reads them out of a
       * plain `Vec` — not `de::lenient_vec`. So before the leniency audit one stray
       * field in one candidate did not cost that candidate, it emptied the ENTIRE
       * ranking and answered `ok:false`, on the single call whose whole purpose is to
       * stop the source list looking empty. `expectOk: true` is the assertion that
       * matters here; the byte comparison alone would not distinguish "ranked" from
       * "the harness projected an error envelope's empty data". */
      id: 'one-candidate-with-an-unreadable-field-does-not-empty-the-ranking',
      expect: 'match',
      expectOk: true,
      list: [{ ...stream('a 720p', '720p', null), behaviorHints: 'hls' }, stream('b 4K', '4K', null)],
      lang: '',
    },
    {
      id: 'two-sources-of-the-same-height-different-size',
      expect: { d: 'D-rank-size', why: WHY_RANK_SIZE },
      list: [stream('1080p re-encode 1 GB', '1080p', '1 GB'), stream('1080p remux 8 GB', '1080p', '8 GB')],
      lang: '',
    },
    {
      id: 'a-language-the-user-did-not-pick',
      expect: { d: 'D-rank-filter', why: WHY_RANK_FILTER },
      list: [stream('en 1080p', '1080p', null, ['en']), stream('ru 4K', '4K', null, ['ru'])],
      lang: 'en',
    },
  ],
  run(t, core, f) {
    /* :594-595, moved from :526-527 (the add-on source-kind and add-on-meta work), from :483-484 (which the transcription had NOT been re-pinned to
     * — it still read :476-477, seven lines behind the file, and the run was failing on the
     * pin rather than on the comparison), from :361-362 and from :281-282 before that.
     * The latest move is the add-on fan-out becoming incremental: `collectAddonStreams`
     * publishes per add-on now, and the effect and language-default comments above the
     * component grew with it. The EXPRESSION is untouched — the twin still filters by
     * language and sorts by `qualityRank` alone, which is what this family compares.
     *
     * THE :351 TRAP IS GONE, and that is worth recording rather than just deleting.
     * This comment used to warn that assertSourceLine's hint pointed at a DIFFERENT
     * copy of the sort inside `playEpisode`, which filtered on `want` rather than
     * `lang`, and that re-pinning from the error message would silently move the
     * corpus onto the wrong expression. That second copy no longer exists: the
     * auto-play path now calls `bestFor`, which prefers a source whose only audio
     * language is the wanted one before it looks at quality. So the needle is unique
     * in the file again — but keep pinning the sort and the filter as a PAIR and keep
     * checking they are adjacent, because that is what makes the pin identify an
     * expression rather than a line that happens to contain some text.
     *
     * `bestFor` is deliberately NOT transcribed here. It is a play-time choice, not a
     * ranking: `shownStreams` is still exactly the filter-then-quality-sort the core's
     * `rank_streams` is the twin of, and that is what this family compares. */
    assertSourceLine('src/components/DetailModal/DetailModal.tsx', 649, SHELL_SORT_SOURCE,
      'the parity corpus transcribes this sort because it is an expression inside JSX and cannot be imported.');
    assertSourceLine('src/components/DetailModal/DetailModal.tsx', 648, SHELL_FILTER_SOURCE,
      'the parity corpus transcribes this filter because it is an expression inside JSX and cannot be imported.');
    const tsOut = shellShown(t.client.qualityRank, f.list.slice(), f.lang);
    const env = unwrap(core.rank_streams(S(f.list), '{}'), 'rank_streams');
    const coreOut = env.data.ranked.map((r) => f.list[r.index]);
    return { tsOut, coreOut, env, notes: [`summary ${S(env.data.summary)}`] };
  },
};

/* --------------------------------------------------------------------------
 * 13. the media-key round trip
 * ------------------------------------------------------------------------ */

/* `MediaRef` is NOT reachable from the wasm boundary — there is no `media_ref`
 * export and no `media_key` export. So what can be asserted is the round trip: the
 * shell builds a key and a wire video id with the two expressions in
 * `DetailModal.tsx`, and the core has to read the key back out unchanged
 * (`continue_watching` re-emits it, `resume_position` finds the record under it) and
 * build the same resource path from the video id.
 *
 * That is the whole of what a shell can observe, and it is also the whole of what
 * has to hold: if the two spell the key differently, removals stop cleaning up and
 * resume positions silently detach, and nothing anywhere fails loudly. Both
 * expressions are pinned to their source lines. */
const SHELL_KEY_SOURCE = '`${target.id}:S${pickedEp.season}E${pickedEp.ep}`';
const shellMediaKey = (id, ep) => (ep ? `${id}:S${ep.season}E${ep.ep}` : String(id));
const shellVideoId = (imdb, ep) => (ep ? `${imdb}:${ep.season}:${ep.ep}` : String(imdb));

const mediaKey = {
  name: 'media key round trip  (vs DetailModal.tsx:426 / :378 / :543)',
  twin: { at: 'Groloo-Web/src/components/DetailModal/DetailModal.tsx:426, :378, :543, :623', verdict: 'unreachable', obligation: 'MediaRef is not exported at the boundary, so there is nothing for these expressions to be replaced BY. The round trip is proven; the deletion is not available.' },
  ts: '`${id}:S${season}E${ep}` and `${imdb}:${season}:${ep}`   [transcribed from JSX, pinned]',
  rust: 'continue_watching().key + resume_position(key) + addon_resource_path(videoId)',
  fixtures: [
    { id: 'a-film', expect: 'match', mediaId: 'tt0111161', ep: null, type: 'movie' },
    { id: 'an-imdb-episode', expect: 'match', mediaId: 'tt0903747', ep: { season: 1, ep: 4 }, type: 'series' },
    { id: 'a-tmdb-film', expect: 'match', mediaId: 603, ep: null, type: 'movie' },
    { id: 'a-tmdb-episode', expect: 'match', mediaId: 1399, ep: { season: 2, ep: 9 }, type: 'series' },
    { id: 'a-double-digit-episode', expect: 'match', mediaId: 'tt0903747', ep: { season: 10, ep: 22 }, type: 'series' },
  ],
  run(t, core, f) {
    /* :426, moved from :404 (and :361, :319, :312, :188 before that). Unique in the file.
     *
     * THE THREE COMPANION NUMBERS IN `name`/`twin.at` WERE STALE, and had been since the commit
     * that moved this one to :404 — that pass re-pinned the asserted line and left the prose
     * pointing at :299 / :433 / :458, which by then were two comment lines and a `: null;`.
     * Recovered by reading DetailModal at the revision the numbers were written against
     * (Groloo-Web 91d80fe) rather than guessed at, and they are now :378 / :543 / :623:
     *
     *   :299 → :378   the wire video id. The expression itself changed shape when the builder
     *                 moved into `videoIdFor` and learned to prefer an add-on's own episode id:
     *                 `${imdb}:${pickedEp.season}:${pickedEp.ep}` is now
     *                 `${streamBaseId}:${ep.season}:${ep.ep}`, reached through that helper.
     *   :433 → :543   `buildMediaFor`'s copy of the key builder. Unchanged, only moved.
     *   :458 → :623   where `playEpisode` obtains the video id. It used to interpolate the id
     *                 inline at the `collectAddonStreams` call; it now asks `videoIdFor`, so the
     *                 pin follows the call site rather than an expression that no longer exists.
     *
     * None of the three is asserted — only the key builder above is — so they are documentation,
     * and the reason they went stale is that nothing re-reads them. Worth knowing before trusting
     * any line number in this file that `assertSourceLine` is not holding. */
    assertSourceLine('src/components/DetailModal/DetailModal.tsx', 426, SHELL_KEY_SOURCE,
      'the parity corpus transcribes the media-key builder because it is an expression inside JSX.');
    const key = shellMediaKey(f.mediaId, f.ep);
    const videoId = shellVideoId(f.mediaId, f.ep);
    const lib = {
      history: [{ id: String(f.mediaId), title: 'T', at: 100, key, season: f.ep && f.ep.season, episode: f.ep && f.ep.ep }],
      progress: { [key]: { pos: 30, dur: 100, at: 100 } },
      removed: {},
    };
    const cw = unwrap(core.continue_watching(S(lib), '{}'), 'continue_watching');
    const resume = unwrap(core.resume_position(S(lib), key), 'resume_position');
    const path = unwrap(core.addon_resource_path('stream', f.type, videoId), 'addon_resource_path');
    // The twin's side of the round trip is the two strings it built, plus the
    // resume position it expects to find under the key it wrote.
    return {
      tsOut: { key, pos: 30, path: `stream/${f.type}/${encodeURIComponent(videoId)}.json` },
      coreOut: { key: cw.data[0] && cw.data[0].key, pos: resume.data && resume.data.pos, path: path.data },
      env: cw,
      notes: [`mediaKeySeparator=${S(unwrap(core.core_constants(), 'core_constants').data.mediaKeySeparator)}`],
    };
  },
};

/* --------------------------------------------------------------------------
 * coverage this corpus cannot provide, stated rather than omitted
 * ------------------------------------------------------------------------ */

export const UNCOMPARABLE = [
  {
    fn: 'MediaRef / MediaKind::from_wire / MediaKind::as_api_type',
    side: 'rust-only, UNREACHABLE',
    why: 'None of these is exported at the boundary — `api.rs` has 22 functions and not one of them takes or returns a `MediaRef`. So `queries.ts:35`\'s `type === \'tv\' || type === \'series\' ? \'tv\' : \'movie\'` has NOTHING to be replaced by and cannot be deleted, and the `movie|tv` vs `movie|series` split is closed only where it happens to pass through `catalog_metas`. A shell that wants to parse `tt0903747:S1E4` still has to do it in TypeScript.',
  },
  {
    fn: 'server.js:1002 found.movie_results with no tv_results check',
    side: 'server-only',
    why: 'The `/find/{imdb}` lookup reads `found.movie_results[0]` and never `found.tv_results`, so an IMDb id that names a SERIES resolves to nothing. It is the same `movie|tv` split this increment set out to close, in the third repository, and it is still open — this corpus cannot assert it because there is no core function on that path.',
  },
  {
    fn: 'langName (addonClient.ts:23-28)',
    side: 'ts-only, deliberately',
    why: 'i18n resolution. Plan 06 §3 keeps rendered strings out of the core so a translated string is never an FFI crossing. It stays, and nothing in the core competes with it.',
  },
  {
    fn: 'fetchAddonJSON / collectAddonStreams / fetchAddonCatalog (the I/O halves)',
    side: 'ts-only, deliberately',
    why: 'An AbortController, a 20-second timer and a `Promise.all` fan-out with a per-add-on catch. The core does no I/O by design. The corpus runs THROUGH these functions, so their parsing halves are proven and their transport halves are what remains after the parsing is deleted.',
  },
  {
    fn: 'qualityRank (addonClient.ts:41)',
    side: 'both, still needed',
    why: 'Exported and consumed by two DetailModal expressions. `rank_streams` supersedes it, but only once those two call sites are rewritten to consume `{ranked, summary}` — which is a UI change, not a substitution. Until then the twin has a live caller and cannot go.',
  },
  {
    fn: 'stream_parse warnings / rank_streams caps profile / envelope ok+error',
    side: 'rust-only',
    why: 'No TypeScript counterpart exists to differ from: the twin has no concept of a warning, no device profile, and one undifferentiated `catch { return [] }` where the envelope now carries a code. Reported here rather than compared, exactly as `tests/differential/` reports its own always-on additions.',
  },
];

/* --------------------------------------------------------------------------
 * the canary — proving this gate can go red
 * ------------------------------------------------------------------------ */

/* A parity harness that has never failed is indistinguishable from one that cannot
 * fail, and the second is worse than none: it produces a green tick a reviewer reads
 * as permission to delete the TypeScript. So `run.mjs --prove` pushes these through
 * the IDENTICAL pipeline the real corpus takes and asserts that each is classified
 * the way it claims.
 *
 * Four are real pairs of calls with a deliberately wrong expectation attached. The
 * last three are the two GUARDS this corpus rests on, fired against real bytes: the
 * transcription pin (`assertSourceLine`), and the circular-comparison guard
 * (`assertNotACoreCallSite`) in both directions — because a guard that fires on
 * everything and a guard that fires on nothing are equally worthless, and the whole
 * reason this harness exists is that the second kind was shipped once already. */
const CANARY = {
  name: 'canary (--prove only)',
  ts: 'real calls, deliberately wrong expectations',
  rust: 'each must be caught by the classifier it names',
  fixtures: [
    {
      id: 'a-real-divergence-declared-as-a-match',
      mustClassifyAs: 'unexpected',
      family: catalogMetas, expect: 'match',
      body: { metas: [{ id: 'tt1', name: 'A', poster: 'p.jpg', type: 'tv' }] },
    },
    {
      id: 'an-agreement-declared-as-a-divergence',
      mustClassifyAs: 'stale-expectation',
      family: baseUrl, expect: { d: 'FAKE', why: 'there is no divergence here; the runner must say so' },
      url: 'https://a.co/x/manifest.json',
    },
    {
      id: 'an-ok-flag-that-is-not-what-was-promised',
      mustClassifyAs: 'signal-mismatch',
      family: orderLangs, expect: 'match', expectOk: false, langs: ['en'],
    },
    {
      id: 'an-error-detail-that-is-not-what-was-promised',
      mustClassifyAs: 'signal-mismatch',
      family: normalizeVsServer, expect: 'match', url: 'not-an-id',
      expectOk: false, expectErrorDetail: 'some other sentence',
    },
    {
      id: 'a-warning-that-was-promised-and-not-raised',
      mustClassifyAs: 'warning-mismatch',
      family: streamParse, expect: 'match', expectWarning: 'dropped.bad_item',
      body: { streams: [{ name: 'perfectly fine', url: 'https://a.co/v.mp4' }] },
    },
    {
      /* The pin on a transcribed expression. Two families compare against text
       * copied out of `DetailModal.tsx`, and the ONLY thing keeping those copies
       * honest is `assertSourceLine` failing the run when the original moves. A
       * guard that has never fired is a guard nobody has checked, so it is fired
       * here on purpose, against a line that deliberately does not match. */
      id: 'a-transcription-whose-source-line-has-changed',
      mustClassifyAs: 'threw',
      family: {
        name: 'pin',
        run() {
          assertSourceLine('src/components/DetailModal/DetailModal.tsx', 649,
            '.sort((a, b) => somethingElse(a, b))',
            'this canary asserts that the transcription pin fails loudly when the source moves.');
          return { tsOut: null, coreOut: null, notes: [] };
        },
      },
      expect: 'match',
    },
    {
      /* THE CIRCULAR-COMPARISON GUARD, FIRED ON PURPOSE.
       *
       * `PRE_PORT_REV` is `'HEAD'`, and that is correct for exactly as long as the
       * `addonClient.ts` deletions stay uncommitted. The moment they land,
       * `git show HEAD:` returns the CALL SITE, every twin becomes `callData(...)`,
       * and the corpus goes back to comparing the core against itself — the defect
       * that made this gate unable to fail in the direction it exists to fail in.
       *
       * `assertNotACoreCallSite` catches that on every run. This canary is what makes
       * it a checked guard rather than an unchecked one, and it is fired against the
       * REAL WORKING-TREE `addonClient.ts` — which imports `./heart` today and is
       * byte-for-byte what HEAD becomes on commit. Nothing here is written to trip it. */
      id: 'a-pre-port-revision-that-is-already-a-core-call-site',
      mustClassifyAs: 'threw',
      family: {
        name: 'circular guard (positive)',
        run(t) {
          assertNotACoreCallSite(
            t.sources.working['src/lib/addonClient.ts'],
            'src/lib/addonClient.ts',
            'the working tree (which is what HEAD becomes once the port is committed)',
          );
          return { tsOut: 'the guard stayed silent', coreOut: 'the guard stayed silent', notes: [] };
        },
      },
      expect: 'match',
    },
    {
      /* The other direction, and it is not a formality. A guard that throws on
       * everything is as useless as one that throws on nothing — it would fail every
       * run for a reason nobody could act on, and the first response would be to delete
       * it. So the same function is driven over the QUOTED revision, which is the copy
       * the corpus is entitled to compare against, and must stay silent. */
      id: 'a-pre-port-revision-that-is-still-typescript',
      mustClassifyAs: 'match',
      family: {
        name: 'circular guard (negative)',
        run(t) {
          assertNotACoreCallSite(t.sources.quoted['src/lib/addonClient.ts'], 'src/lib/addonClient.ts', t.rev);
          assertNotACoreCallSite(t.sources.quoted['src/stores/addons.ts'], 'src/stores/addons.ts', t.rev);
          return { tsOut: 'the guard stayed silent', coreOut: 'the guard stayed silent', notes: [] };
        },
      },
      expect: 'match',
    },
  ],
  run(t, core, f) { return f.family.run(t, core, f); },
};

export { CANARY };

export const FAMILIES = [
  normalizeVsServer, normalizeVsClient, baseUrl, hasResource,
  streamParse, resourcePath, catalogMetas, addonCatalogs, orderLangs,
  validateVsServer, validateVsClient, rankStreams, mediaKey,
];
