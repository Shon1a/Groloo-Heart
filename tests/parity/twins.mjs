/* Loading the THREE implementations this corpus compares — the TypeScript the app
 * ships, the JavaScript the server ships, and the Rust core — into one node process,
 * so a fixture can be pushed through each of them in the same tick and the answers
 * compared as bytes rather than as prose.
 *
 * ## This is a different harness from `tests/differential/`, on purpose
 *
 * `tests/differential/` compares the OLD vendored core against the NEW vendored
 * core. Both sides are Rust, so it is structurally incapable of noticing the thing
 * that matters here: `addon.rs` shipped in 0.1.0 with no binding of any kind, which
 * means the five add-on-protocol functions have no old-core counterpart and pass
 * that gate trivially. Its own `UNCOMPARABLE` list says so in as many words —
 * "their real twins are in TypeScript … and D6 must be asserted against THOSE".
 *
 * This file is that assertion. The twin rule is absolute: a TypeScript
 * implementation may only be deleted once a corpus proves the Rust replacement
 * produces byte-identical output on that corpus. So the twins are not described
 * here, not transcribed here, not re-implemented here — they are IMPORTED AND RUN,
 * from the files that ship, and where that is impossible the reason is stated and
 * the substitute is checked against the original source text at load time.
 *
 * ## How each twin is reached, and what that cost
 *
 * **`Groloo-Web/src/lib/addonClient.ts`** and **`src/stores/addons.ts`** are
 * imported directly. Node 23+ strips TypeScript types natively, so the bytes node
 * executes are the bytes in the repository. Two things had to be arranged:
 *
 *   1. A resolve hook (`hooks.mjs`), because Vite lets a specifier omit its
 *      extension and node's ESM resolver does not. It appends `.ts` and nothing
 *      else — it does not rewrite, transform or inline any module.
 *   2. Two SUBSTITUTED modules, `src/lib/api.ts` and `src/stores/auth.ts`. The
 *      first reads `import.meta.env`, which exists only under Vite; the second is
 *      the session store. Neither is on any code path this corpus exercises —
 *      `normalizeManifestUrl` is a pure string function and the add-on client never
 *      touches either — but `stores/addons.ts` imports both at module scope, so
 *      without the substitutes the file cannot be loaded at all. They are the
 *      minimum, and they are named here rather than buried so that a reader can
 *      check the claim.
 *
 * `localStorage` and `fetch` are supplied by the harness for the same reason:
 * `stores/addons.ts` reads storage while it is being evaluated, and
 * `collectAddonStreams` is an I/O function whose PARSING is what is under test. The
 * fetch stub is a recorder — it captures the URL the twin asked for (which is how
 * `addon_resource_path` gets compared to something real) and replays the fixture
 * body.
 *
 * **`Groloo-server/server/server.js`** cannot be imported: the module body calls
 * `app.listen`, opens a database and registers timers. Its two twins are not
 * exported either. So their SOURCE TEXT is lifted out of the file by brace-matching
 * from `function <name>(` and evaluated. That is materially different from copying
 * them into this file: if the server edits its copy, this harness runs the edit. If
 * the text cannot be found the loader throws — there is deliberately no transcribed
 * fallback, because a transcribed copy tests the transcription.
 *
 * **`DetailModal.tsx`** holds two expressions that are not functions and not
 * exported — the stream sort at `:476`/`:477` and the media-key builders at
 * `:319`/`:299`/`:433`. They are inside a React component in a `.tsx` file that
 * imports React, and there is no honest way to reach them from node. They are
 * transcribed into `fixtures.mjs`, and [`assertSourceLine`] below re-reads the real
 * file and fails the run if the line it was transcribed from has changed. A
 * transcription nobody re-checks is a fossil; one that is pinned to its source is a
 * quotation.
 *
 * ## THE ASSUMPTION THIS FILE GOT WRONG, AND HOW IT IS REPAIRED
 *
 * Everything above assumes the twin is still on disk. That stopped being true the
 * moment the increment this gate exists to gate started landing: seven of the thirteen
 * twins were DELETED out of `addonClient.ts` and replaced by `callData(...)` call
 * sites, and the manifest check inside `stores/addons.ts:install` was replaced by
 * `validate_manifest`. A harness that keeps importing the working tree after that is
 * no longer comparing two implementations. It is comparing the core against ITSELF,
 * through `heart.ts` — and under node, where there is no browser to fetch
 * `/assets/groloo-core/…/groloo_core.js` from, `loadCore()` fails, `callData`
 * answers `null`, and every one of those call sites returns its degraded value: `[]`,
 * `''`, `false`. That is what the 25 THREW and the 59 UNEXPECTED were. Not one of them
 * was a statement about the Rust.
 *
 * A parity gate whose TypeScript side IS the Rust cannot fail in the direction it
 * exists to fail in, so this is a mechanism defect and not a result. The repair:
 *
 *   1. **The twin is resolved, not assumed.** [`PROVENANCE`] probes every binding the
 *      corpus drives, against both the working tree and the last committed revision,
 *      and classifies it IN TREE / REPLACED / DELETED. Mechanically, by calling it —
 *      no static analysis and no table anyone has to remember to update.
 *   2. **A twin that is no longer on disk is read out of git** (see `hooks.mjs`'s
 *      virtual loader) and run from those bytes. The deletion under review is exactly
 *      what has to be exercised: asserting parity against the deletion's REPLACEMENT
 *      only proves the replacement equals itself.
 *   3. **The report says which.** `run.mjs` prints the provenance table above the
 *      fixtures, because "byte-identical to the twin" means something different when
 *      the twin is a git blob than when it is the file the browser loads, and whoever
 *      is deciding whether a deletion was safe has to be told which one they read.
 *
 * The alternative — teaching the harness to load the wasm so `heart.ts` works under
 * node — was considered and is wrong on purpose. It would make every REPLACED twin
 * agree with the core trivially and unfalsifiably, which is the exact failure this
 * file exists to prevent. */

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { register } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..', '..');
import { webRepo, serverRepo } from '../../scripts/sibling.mjs';
export const WEB = webRepo(REPO);
export const SERVER = serverRepo(REPO);

const ADDON_CLIENT_TS = join(WEB, 'src', 'lib', 'addonClient.ts');
const ADDONS_STORE_TS = join(WEB, 'src', 'stores', 'addons.ts');
const DETAIL_MODAL_TSX = join(WEB, 'src', 'components', 'DetailModal', 'DetailModal.tsx');
const SERVER_JS = join(SERVER, 'server', 'server.js');

/* ------------------------------------------------------------------------- */
/* the browser globals the twins evaluate against                             */
/* ------------------------------------------------------------------------- */

/** `stores/addons.ts` reads storage during module evaluation (`loadForIdentity()`
 * at the top of `create()`), so this has to exist BEFORE the import, not after. It
 * starts empty, which is the state of a device nobody has installed an add-on on —
 * every record the corpus uses is pushed in explicitly by a fixture. */
function installLocalStorage() {
  const cells = new Map();
  globalThis.localStorage = {
    getItem: (k) => (cells.has(k) ? cells.get(k) : null),
    setItem: (k, v) => { cells.set(k, String(v)); },
    removeItem: (k) => { cells.delete(k); },
    clear: () => { cells.clear(); },
    key: (i) => [...cells.keys()][i] ?? null,
    get length() { return cells.size; },
  };
}

/** The fetch every add-on call in the twin goes through.
 *
 * It is a recorder as much as a stub. `collectAddonStreams` and
 * `fetchAddonCatalog` are I/O functions wrapped around the parsing this corpus is
 * actually about, and the URL they build on the way is `addon_resource_path`'s twin
 * — the only place it exists in TypeScript. Capturing the request is therefore not
 * instrumentation, it is one of the comparisons.
 *
 * `plan` is consumed one entry per call so a fan-out over two add-ons can hand each
 * of them a different body. An unplanned call throws rather than returning
 * something plausible: a corpus that silently answered a request nobody predicted
 * would be testing the stub. */
function installFetch() {
  const state = { plan: [], seen: [] };
  globalThis.fetch = async (input) => {
    const url = String(input);
    state.seen.push(url);
    const next = state.plan.shift();
    if (next === undefined) throw new Error(`parity: the twin fetched ${url}, which no fixture planned for`);
    if (typeof next === 'function') return next(url);
    return {
      ok: next.ok !== false,
      status: next.status ?? 200,
      json: async () => {
        // `Response.json()` rejects on a body that is not JSON, and the twin's
        // `catch { return [] }` depends on that. A fixture that supplies `text`
        // reproduces it exactly rather than approximating it with a null.
        if (typeof next.text === 'string') return JSON.parse(next.text);
        return next.body;
      },
      /* The ported half reads `r.text()` and hands the RAW BYTES to the core rather
       * than round-tripping them through `r.json()` first. Both spellings have to
       * work here or the provenance probe below would classify a twin by which
       * method the harness forgot to supply. */
      text: async () => (typeof next.text === 'string' ? next.text : JSON.stringify(next.body)),
    };
  };
  return state;
}

/* ------------------------------------------------------------------------- */
/* the twin that is no longer on disk                                         */
/* ------------------------------------------------------------------------- */

/** The revision the pre-port twins are quoted from.
 *
 * A FIXED SHA, and it has to be. This was `'HEAD'` while the port was uncommitted,
 * on the reasoning that HEAD was then by construction the last all-TypeScript
 * revision. **That expiry date has passed.** Groloo-Web commit `3167326`
 * ("rename Stredio -> Groloo, and land the uncommitted Phase 0 store-blocker
 * work") is the commit that made `src/lib/addonClient.ts` import `./heart`, so
 * from that commit onward `git show HEAD:` returns the CALL SITE and the
 * "TypeScript twin" this corpus quotes would be the core itself.
 *
 * [`assertNotACoreCallSite`] caught exactly that and refused to run — a guard
 * doing precisely its job rather than a bug. The value below is `3167326^`,
 * resolved and written out in full: the last revision in which the twin was
 * still TypeScript (a 164-line file importing only `../stores/addons` and
 * `./types`).
 *
 * Written as a sha and not `3167326^`, `HEAD~n`, a tag or a branch, because every
 * one of those can resolve to different bytes later and the whole point of this
 * constant is that it cannot. It is resolved and printed at load time, so a run's
 * verdict names the bytes it compared against rather than a moving label.
 *
 * If the twins are ever re-ported, move this pin in the SAME commit that does it,
 * for the reason the `gates` job in `.github/workflows/ci.yml` gives at length:
 * between the port and the pin, this job is red for a reason that looks like a
 * bug in the Rust and is not. */
const PRE_PORT_REV = '0dd5447c9ba2';

/** The import that turns a "TypeScript twin" into the core's own call site.
 *
 * `addonClient.ts` reaches the shell as `./heart` and `stores/addons.ts` as
 * `../lib/heart`; both spellings, and no others, are what a port introduces. */
const IMPORTS_THE_SHELL = /from '\.{1,2}\/(lib\/)?heart'/;

/** THE CIRCULAR-COMPARISON GUARD.
 *
 * Refuse to treat `text` as a TypeScript twin when it is really a core call site.
 *
 * Today the deletions are uncommitted, so HEAD still holds TypeScript. The moment they
 * are committed, `git show HEAD:` starts returning the CALL SITE, both copies of every
 * twin become `callData(...)`, the probes below see two identical degraded answers and
 * classify them IN TREE — and the corpus goes back to comparing the core with itself.
 * It would show as a wall of red rather than a false green (the fixtures would see `[]`
 * where the core answers data), but a reader would be looking for the bug in the Rust
 * and the numbers would be meaningless either way.
 *
 * Exported because a guard that has never fired is a guard nobody has checked: the
 * canary in `fixtures.mjs` drives it from BOTH sides — the working-tree file, which is
 * a call site today and is exactly what HEAD becomes on commit, and the quoted file,
 * which must NOT trip it. A guard that always throws is as useless as one that never
 * does, so both directions are asserted.
 *
 * @param {string} text     the file's contents at `rev`
 * @param {string} relPath  its path within Groloo-Web, for the message
 * @param {string} rev      what it was read from, for the message
 */
export function assertNotACoreCallSite(text, relPath, rev) {
  if (!IMPORTS_THE_SHELL.test(text)) return;
  const line = text.split(/\r?\n/).findIndex((l) => IMPORTS_THE_SHELL.test(l)) + 1;
  throw new Error(
    `parity: ${relPath} at ${rev} imports the core shell (\`./heart\`) on line ${line}, so the ` +
    `"TypeScript twin" this corpus would quote from ${rev} IS THE CORE.\n\n` +
    'A comparison of the core against its own call site cannot fail, and it must never be ' +
    'reported as parity — that is the precise defect this harness was rebuilt to remove, and ' +
    'the reason the twin rule demands a proof rather than a green tick.\n\n' +
    'REMEDY — pin PRE_PORT_REV in tests/parity/twins.mjs to the last ALL-TYPESCRIPT revision.\n' +
    `It is currently '${PRE_PORT_REV}', which is only correct while the port is UNCOMMITTED.\n` +
    'Find the commit that introduced the import, and take its parent:\n\n' +
    `    cd ${WEB}\n` +
    `    git log --oneline -S"from './heart'" -- ${relPath} | tail -1\n\n` +
    'then, in tests/parity/twins.mjs:\n\n' +
    "    const PRE_PORT_REV = '<that sha>^';\n\n" +
    'Do NOT point it at a tag or a branch name that can move, and do not delete the check: a ' +
    'run whose TypeScript side is the Rust produces numbers that mean nothing, in either colour.',
  );
}

/** `git show`, with the failure spelled out. Reading history is the only way to reach
 * a deleted twin, and a corpus that silently fell back to the working tree here would
 * report the core matching itself. */
function gitShow(repo, rev, relPath) {
  let text;
  try {
    text = execFileSync('git', ['show', `${rev}:${relPath}`], { cwd: repo, encoding: 'utf8', maxBuffer: 1 << 26 });
  } catch (e) {
    throw new Error(
      `parity: could not read ${relPath} at ${rev} from ${repo}.\n${String(e.message).trim()}\n` +
      'Seven of the thirteen twins have been deleted from the working tree, so the implementation this ' +
      'corpus has to run only exists in history. Without it the harness would be comparing the core ' +
      'against the call site that replaced it, which cannot fail.',
    );
  }
  /* THE TRAP THIS RUN WAS BUILT TO AVOID, ONE COMMIT LATER. See
   * [`assertNotACoreCallSite`] for what it is and how to get out of it. It lives up
   * there rather than inline here so that `--prove` can fire it on real bytes. */
  assertNotACoreCallSite(text, relPath, rev);
  return text;
}

/** The URL a quoted twin is loaded under: the real path, plus a query that makes it a
 * distinct module. The query is load-bearing twice over — it keeps the quoted copy
 * from colliding with the working-tree copy in node's module cache, and it leaves the
 * DIRECTORY intact so `../stores/addons` inside it resolves to the same file, and
 * therefore the same live zustand instance, that the working-tree copy reaches. */
const quotedUrl = (abs) => `${pathToFileURL(abs).href}?parity-twin=${PRE_PORT_REV}`;

/* ------------------------------------------------------------------------- */
/* which implementation is the twin — decided by calling both                 */
/* ------------------------------------------------------------------------- */

/** One probe per binding the corpus drives.
 *
 * Each is a REAL call with a fixed input, run against the working tree and against the
 * quoted pre-port copy. The comparison of those two answers is the whole classifier:
 *
 *   DELETED   the working tree does not expose the binding at all.
 *   REPLACED  it does, and it answers differently. In this increment that is always
 *             the same cause — the body is now `callData(...)` and node has no wasm
 *             shell for `heart.ts` to load — but the probe does not assume the cause;
 *             it records the two answers and lets the report show them.
 *   IN TREE   it answers identically, so the file on disk is still the twin and is
 *             what the fixtures are driven through.
 *
 * The point of doing it by call rather than by reading the source is that it cannot go
 * stale. Restore a deleted function and its family switches back to the working tree
 * on the next run, with no edit here. */
const PROVENANCE = [
  {
    name: 'addonBaseUrl', of: 'client', at: 'addonClient.ts:45', driven: /addonBaseUrl\(/,
    probe: (m) => m.addonBaseUrl('https://a.co/x/manifest.json'),
  },
  {
    name: 'addonHasResource', of: 'client', at: 'addonClient.ts:49', driven: /addonHasResource\(/,
    probe: (m) => m.addonHasResource({ manifest: { id: 'org.x', name: 'X', resources: ['stream'], types: ['movie'] } }, 'stream', 'movie'),
  },
  {
    name: 'orderLangs', of: 'client', at: 'addonClient.ts:32-38', driven: /orderLangs\(/,
    probe: (m) => m.orderLangs(['zz', 'aa', 'en']),
  },
  {
    name: 'qualityRank', of: 'client', at: 'addonClient.ts:41', driven: /qualityRank/,
    probe: (m) => [m.qualityRank('1080p'), m.qualityRank('nonsense')],
  },
  {
    name: 'langName', of: 'client', at: 'addonClient.ts:23-28', driven: /langName\(/,
    probe: (m) => [m.langName('en'), m.langName('xx')],
  },
  {
    name: 'listAddonCatalogs', of: 'client', at: 'addonClient.ts:147-156', driven: /listAddonCatalogs\(/,
    probe: (m, h) => {
      h.setInstalled([{ id: 'org.a', url: 'https://a.co/manifest.json', manifest: { id: 'org.a', name: 'Alpha', resources: ['catalog'], types: ['movie'], catalogs: [{ type: 'movie', id: 'top', name: 'Top' }] } }]);
      return m.listAddonCatalogs();
    },
  },
  {
    name: 'collectAddonStreams', of: 'client', at: 'addonClient.ts:93-126', driven: /collectAddonStreams/,
    probe: async (m, h) => {
      h.setInstalled([{ id: 'org.a', url: 'https://a.co/manifest.json', manifest: { id: 'org.a', name: 'Alpha', resources: ['stream'], types: ['movie'] } }]);
      h.plan({ body: { streams: [{ name: 'X 1080p', url: 'https://a.co/v.mp4' }] } });
      return m.collectAddonStreams('tt1', 'movie');
    },
  },
  {
    name: 'fetchAddonCatalog', of: 'client', at: 'addonClient.ts:130-163', driven: /fetchAddonCatalog/,
    probe: async (m, h) => {
      h.plan({ body: { metas: [{ id: 'tt1', name: 'A', poster: 'p.jpg' }] } });
      return m.fetchAddonCatalog({ addonId: 'org.a', addonName: 'Alpha', type: 'movie', id: 'top', name: 'Top', base: 'https://a.co/' });
    },
  },
  {
    name: 'normalizeManifestUrl', of: 'store', at: 'stores/addons.ts:203', driven: /normalizeManifestUrl\(/,
    probe: (m) => m.normalizeManifestUrl('https://a.co/addon?x=1'),
  },
  {
    /* Not a function on either side — it is the accept/reject decision inside
     * `install()`, which is the only way the manifest check has ever been reachable.
     * The probe serves a manifest with an id and a name and NOTHING else: the deleted
     * two-rule check accepts it, the four-rule core rejects it, and a core that is not
     * there rejects it too. So this probe distinguishes "the check is gone" from "the
     * check is here", which is all it is asked to do. */
    name: 'install (the manifest check)', of: 'store', at: 'stores/addons.ts:233', binding: 'useAddons', driven: /install\(url\)/,
    probe: async (m, h) => {
      m.useAddons.setState({ installed: [], unlinked: [] });
      h.plan({ body: { id: 'org.x', name: 'N' } });
      try { await m.useAddons.getState().install('https://a.co/x/manifest.json'); } catch (e) { return `rejected: ${e.message}`; }
      return 'accepted';
    },
  },
];

/* ------------------------------------------------------------------------- */
/* the half that is still on disk behind an entry point that is not           */
/* ------------------------------------------------------------------------- */

/** Declarations the corpus reaches THROUGH a rerouted entry point, which are still in
 * the working tree.
 *
 * `collectAddonStreams` now begins `if (!(await loadCore())) return []`, so its entry
 * point is a core call site and the family has to be driven from the quoted revision.
 * But the four functions that family is actually ABOUT — the quality detector, the size
 * extractor, the flag table, the mapper — were not deleted with it: U-coerce blocked
 * them, and although U-coerce is now FIXED (`Stream.name`/`title`/`description` coerce
 * through `de::opt_string_or_number`, and the fixture is a `match`) the deletion the
 * corpus now authorises has not been performed yet. They are still on disk, so driving
 * the quoted copy proves something about the shipping code ONLY IF the two are the same
 * text — and the day someone acts on that authorisation, this list is what has to be
 * emptied along with them.
 *
 * So it is checked, not assumed, and it is checked the way `assertSourceLine` checks a
 * transcription: re-read both, compare, and refuse to run on a mismatch. A corpus that
 * quietly gated a copy the app has since edited is the same failure as a transcription
 * nobody re-checks, one file further along. */
const STILL_ON_DISK = [
  {
    file: 'src/lib/addonClient.ts',
    names: ['detectQuality', 'extractSize', 'FLAG_LANG', 'parseStreamLangs', 'mapAddonStream'],
    why: 'the stream_parse family is driven through the quoted `collectAddonStreams`, but these are what it compares and they were kept',
  },
];

/** One top-level declaration's source text, or null. Handles the two spellings the
 * twins use: a `function` (brace-matched) and a `const` whose initialiser opens a
 * bracket on the same line. Anything else returns null and is reported as unfindable
 * rather than silently passing. */
function declarationText(source, name) {
  const fn = source.indexOf(`function ${name}(`);
  if (fn >= 0) {
    let depth = 0;
    let i = source.indexOf('{', fn);
    const start = i;
    for (; i < source.length; i += 1) {
      if (source[i] === '{') depth += 1;
      else if (source[i] === '}' && --depth === 0) break;
    }
    return depth === 0 ? source.slice(fn, i + 1) : null;
  }
  const decl = source.search(new RegExp(`^(export )?const ${name}\\b`, 'm'));
  if (decl < 0) return null;
  const open = source.slice(decl).search(/[[{(]/);
  if (open < 0) return null;
  let depth = 0;
  let i = decl + open;
  const pairs = { '[': ']', '{': '}', '(': ')' };
  const opens = new Set(Object.keys(pairs));
  const closes = new Set(Object.values(pairs));
  for (; i < source.length; i += 1) {
    if (opens.has(source[i])) depth += 1;
    else if (closes.has(source[i]) && --depth === 0) break;
  }
  return depth === 0 ? source.slice(decl, i + 1) : null;
}

/** Compare every declaration in [`STILL_ON_DISK`] between the working tree and the
 * quoted revision. Throws on the first difference: the run cannot report parity for a
 * family whose subject it is no longer reading. */
function assertKeptHalvesUnchanged(quoted) {
  const checked = [];
  for (const group of STILL_ON_DISK) {
    const cur = readFileSync(join(WEB, group.file), 'utf8');
    const pre = quoted[group.file];
    for (const name of group.names) {
      const a = declarationText(cur, name);
      const b = declarationText(pre, name);
      if (a === null || b === null) {
        throw new Error(
          `parity: could not find \`${name}\` in ${group.file} ` +
          `${a === null ? 'on disk' : `at ${PRE_PORT_REV}`}.\n  ${group.why}\n` +
          'The corpus drives the quoted copy of this family, so it can only speak for the working tree ' +
          'while the two are the same text. It cannot check that with the declaration missing.',
        );
      }
      // Whitespace-insensitive at the line ends only: git hands back LF, the working
      // tree may be CRLF, and that difference is a checkout setting rather than an edit.
      if (a.replace(/\r/g, '') !== b.replace(/\r/g, '')) {
        throw new Error(
          `parity: \`${name}\` in ${group.file} is NOT the text this run compares.\n` +
          `  on disk and at ${PRE_PORT_REV} they differ, and the corpus drives the ${PRE_PORT_REV} copy ` +
          `because the entry point above it is now a core call site.\n  ${group.why}\n` +
          'Every verdict this family produces would be about a version of the code the app no longer runs. ' +
          'Re-point PRE_PORT_REV, or reach the working-tree copy directly, before trusting the numbers.',
        );
      }
      checked.push(`${group.file.split('/').pop()}:${name}`);
    }
  }
  return checked;
}

/** Run one probe and reduce it to a comparable string. A throw is an answer too — a
 * binding that is present but explodes is not the twin either, and the message is what
 * tells the reader why. */
async function probeAnswer(fn) {
  try {
    const v = await fn();
    return v === undefined ? '<undefined>' : JSON.stringify(v) ?? '<undefined>';
  } catch (e) {
    return `<threw: ${String(e && e.message).split('\n')[0]}>`;
  }
}

/** Silence the shell's own logging for the duration of a probe.
 *
 * `heart.ts` writes a paragraph and a stack trace to the console every time
 * `loadCore()` fails, and under node it fails on every REPLACED binding by design. The
 * information is not lost — that the working-tree copy needed a core and could not get
 * one is exactly what the provenance table reports, in one line, in the right place.
 * What is suppressed is a stack trace repeated nine times above the report. */
async function quietly(fn) {
  const saved = { warn: console.warn, error: console.error, log: console.log };
  const said = [];
  console.warn = (...a) => said.push(String(a[0]));
  console.error = (...a) => said.push(String(a[0]));
  console.log = (...a) => said.push(String(a[0]));
  try { return { value: await fn(), said }; } finally { Object.assign(console, saved); }
}

/* ------------------------------------------------------------------------- */
/* the TypeScript twins                                                        */
/* ------------------------------------------------------------------------- */

/** Everything the corpus drives, resolved once. */
export async function loadTwins() {
  if (!existsSync(ADDON_CLIENT_TS) || !existsSync(ADDONS_STORE_TS)) {
    throw new Error(
      `the TypeScript twins are not where this harness expects them:\n  ${ADDON_CLIENT_TS}\n  ${ADDONS_STORE_TS}\n` +
      'This corpus exists to compare the core against THOSE files. It cannot run without them, and it must ' +
      'not fall back to a copy — a parity proof against a copy proves nothing about what ships.',
    );
  }
  installLocalStorage();
  const fetchState = installFetch();

  /* The quoted twins are read BEFORE the loader is registered, because the hook
   * thread receives them once, as registration data, and cannot be handed more
   * afterwards. Both files are read whether or not anything turns out to be missing
   * from the working tree: which of the two copies is the twin is a question the
   * probes answer below, and answering it needs both in hand. */
  const rev = execFileSync('git', ['rev-parse', '--short', PRE_PORT_REV], { cwd: WEB, encoding: 'utf8' }).trim();
  const quoted = {
    'src/lib/addonClient.ts': gitShow(WEB, PRE_PORT_REV, 'src/lib/addonClient.ts'),
    'src/stores/addons.ts': gitShow(WEB, PRE_PORT_REV, 'src/stores/addons.ts'),
  };
  const kept = assertKeptHalvesUnchanged(quoted);
  const virtual = {
    [quotedUrl(ADDON_CLIENT_TS)]: quoted['src/lib/addonClient.ts'],
    [quotedUrl(ADDONS_STORE_TS)]: quoted['src/stores/addons.ts'],
  };
  register('./hooks.mjs', pathToFileURL(`${HERE}/`), { data: { virtual } });

  let client;
  let store;
  let preClient;
  let preStore;
  try {
    client = await import(pathToFileURL(ADDON_CLIENT_TS).href);
    store = await import(pathToFileURL(ADDONS_STORE_TS).href);
    preClient = await import(quotedUrl(ADDON_CLIENT_TS));
    preStore = await import(quotedUrl(ADDONS_STORE_TS));
  } catch (e) {
    if (String(e && e.code) === 'ERR_MODULE_NOT_FOUND' && /zustand/.test(String(e.message))) {
      throw new Error(
        `the twins import zustand and ${join(WEB, 'node_modules')} does not provide it.\n` +
        'Run `npm install` in Groloo-Web first. The corpus deliberately imports the REAL store rather ' +
        'than stubbing it — `addonClient.ts` reads `useAddons.getState().installed`, and a stub there ' +
        'would mean the fixtures drove the harness\'s idea of an installed add-on instead of the app\'s.',
      );
    }
    throw e;
  }

  const harness = {
    fetchState,
    /** Replace the device's installed add-ons for one fixture.
     *
     *  BOTH instances, because `install()`'s twin lives in the quoted store while the
     *  add-on client reads the working-tree one, and a fixture that set only one of
     *  them would drive two implementations off two different devices. */
    setInstalled(records) {
      store.useAddons.setState({ installed: records, unlinked: [] });
      preStore.useAddons.setState({ installed: records, unlinked: [] });
    },
    /** Queue the bodies the twin's fetches will receive, and clear the log. */
    plan(...responses) { fetchState.plan = responses; fetchState.seen = []; },
    /** The URLs the twin actually requested since the last `plan`. */
    requested() { return fetchState.seen.slice(); },
  };

  /* ---- decide, per binding, which copy is the twin ---- */

  const provenance = [];
  for (const p of PROVENANCE) {
    const cur = p.of === 'client' ? client : store;
    const pre = p.of === 'client' ? preClient : preStore;
    // `binding` names the EXPORT to look for when it is not the twin's own name —
    // `install` is a method on the store object, not a module export, so its presence
    // is the store's. An absent export is DELETED without needing to be called.
    const present = cur[p.binding || p.name] !== undefined;
    const preAnswer = (await quietly(() => probeAnswer(() => p.probe(pre, harness)))).value;
    const run = await quietly(() => probeAnswer(() => p.probe(cur, harness)));
    const curAnswer = present ? run.value : '<not exported>';
    const verdict = !present ? 'DELETED' : curAnswer === preAnswer ? 'IN TREE' : 'REPLACED';
    provenance.push({
      ...p, verdict, curAnswer, preAnswer,
      // What the working-tree copy said on its way to that answer. `[core] unavailable`
      // here is the evidence that the binding is now a core call site rather than a
      // TypeScript implementation that happens to have changed.
      shellSaid: run.said.filter((s) => s.startsWith('[core]')),
    });
  }
  harness.provenance = provenance;
  harness.rev = rev;
  /** The two texts the circular-comparison guard exists to tell apart, kept so
   *  `--prove` can fire it on real bytes rather than on a string written to trip it.
   *  `working` is a core call site TODAY and is precisely what `git show HEAD:` starts
   *  returning the moment the deletions are committed; `quoted` is what the corpus is
   *  entitled to compare against and must NOT trip the guard. */
  harness.sources = {
    working: {
      'src/lib/addonClient.ts': readFileSync(ADDON_CLIENT_TS, 'utf8'),
      'src/stores/addons.ts': readFileSync(ADDONS_STORE_TS, 'utf8'),
    },
    quoted,
  };
  /** Declarations the corpus reaches through a quoted entry point and has just proved
   *  are the same text on disk. Printed, because "driven from git" and "gating code the
   *  app no longer runs" are not the same claim and the difference is these. */
  harness.kept = kept;
  const twinOf = (name) => {
    const p = provenance.find((q) => q.name === name);
    const src = p && p.verdict === 'IN TREE' ? 'cur' : 'pre';
    return { p, src };
  };
  /** The module a given binding's twin lives in — the file on disk while it still
   *  implements the rule, and the quoted revision once it stops. */
  const pick = (name, of) => {
    const { src } = twinOf(name);
    const mods = of === 'client' ? { cur: client, pre: preClient } : { cur: store, pre: preStore };
    return mods[src];
  };

  /** `addonClient.ts` — addonBaseUrl, addonHasResource, orderLangs, qualityRank,
   *  collectAddonStreams, listAddonCatalogs, fetchAddonCatalog, langName. Assembled
   *  binding by binding rather than taken wholesale from one module: `qualityRank`
   *  and `langName` are still on disk and are driven from there, while the seven that
   *  were deleted are driven from the revision that still has them. */
  harness.client = Object.fromEntries(
    PROVENANCE.filter((p) => p.of === 'client')
      .map((p) => [p.name, pick(p.name, 'client')[p.name]]),
  );

  /** `stores/addons.ts` — `normalizeManifestUrl` (still TypeScript, still on disk) and
   *  the zustand store through which `install()`'s manifest check is reached. */
  harness.store = {
    normalizeManifestUrl: pick('normalizeManifestUrl', 'store').normalizeManifestUrl,
    useAddons: pick('install (the manifest check)', 'store').useAddons,
  };

  return harness;
}

/* ------------------------------------------------------------------------- */
/* the server twin, lifted as source text                                     */
/* ------------------------------------------------------------------------- */

/** Pull one top-level `function name(...) { ... }` out of a source file by
 * brace-matching, and evaluate it.
 *
 * Brace-matching a whole file is crude and would be indefensible against arbitrary
 * text — but the two functions taken here are the first two declarations at their
 * indent level, contain no braces inside strings or regex literals that are not
 * balanced, and the result is checked by calling it. The alternative was importing
 * `server.js`, which opens listeners and a database connection, or transcribing the
 * function, which is the thing the twin rule forbids. */
function liftFunction(source, name, where) {
  const head = source.indexOf(`function ${name}(`);
  if (head < 0) {
    throw new Error(
      `parity: \`function ${name}(\` is no longer in ${where}. The corpus asserts the core against the ` +
      'SERVER copy of this rule, so it cannot proceed without it — and it must not substitute a ' +
      'transcription, which would test the transcription rather than the server.',
    );
  }
  let depth = 0;
  let i = source.indexOf('{', head);
  const start = i;
  for (; i < source.length; i += 1) {
    const c = source[i];
    if (c === '{') depth += 1;
    else if (c === '}') {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  if (depth !== 0) throw new Error(`parity: could not brace-match ${name} in ${where}`);
  const body = source.slice(start, i + 1);
  const params = source.slice(head + `function ${name}(`.length, source.indexOf(')', head));
  // eslint-disable-next-line no-new-func -- the point is to run the server's own text
  return new Function(params, body.slice(1, -1));
}

/** `server.js`'s `normalizeManifestUrl` (:1992) and `validateManifest` (:1947),
 * running from the server's own bytes.
 *
 * These two are the copies the core ADOPTED — the unification ledger in
 * `crate::addon` says the server won on the query axis and that `validate_manifest`
 * took the server's four rules and its four message strings verbatim. A parity
 * corpus that only checked the client would prove the core matches the copy it
 * deliberately does not match. */
export function loadServerTwin() {
  if (!existsSync(SERVER_JS)) {
    throw new Error(`parity: ${SERVER_JS} is missing — the server half of the D6 ledger cannot be asserted.`);
  }
  const src = readFileSync(SERVER_JS, 'utf8');
  return {
    normalizeManifestUrl: liftFunction(src, 'normalizeManifestUrl', 'server.js'),
    validateManifest: liftFunction(src, 'validateManifest', 'server.js'),
  };
}

/* ------------------------------------------------------------------------- */
/* the expressions that cannot be imported                                    */
/* ------------------------------------------------------------------------- */

/** Assert that a transcribed expression is still what the file says.
 *
 * `DetailModal.tsx` holds the two stream-sort expressions and the three media-key
 * builders inline, in JSX, inside a component that imports React. There is no
 * honest way to call them from node. So they are transcribed into `fixtures.mjs`
 * and pinned here: the real line is re-read on every run and the run fails if the
 * substring it was transcribed from has gone. A transcription nobody re-checks
 * silently becomes a description of an old build — which is exactly the failure
 * mode this whole increment exists to close. */
export function assertSourceLine(file, line, needle, why) {
  const path = join(WEB, file);
  if (!existsSync(path)) throw new Error(`parity: ${file} is missing; ${why}`);
  const text = readFileSync(path, 'utf8').split(/\r?\n/);
  const got = text[line - 1] ?? '';
  if (!got.includes(needle)) {
    const found = text.findIndex((l) => l.includes(needle));
    throw new Error(
      `parity: ${file}:${line} no longer contains\n    ${needle}\n  it now reads\n    ${got.trim()}\n` +
      (found >= 0 ? `  (the text is at :${found + 1} — update the transcription's line number)\n` : '') +
      `  ${why}`,
    );
  }
}

/* ------------------------------------------------------------------------- */
/* the core                                                                    */
/* ------------------------------------------------------------------------- */

/* Deliberately re-exported from the differential harness rather than re-loaded
 * here. `cores.mjs` verifies the vendored artifact against its own manifest, then
 * against a fresh build of this tree, and reports whether `heart.ts` points at it —
 * three checks this corpus needs for exactly the same reason that one does, and a
 * second loader would be a second place for them to rot. */
export { loadNew as loadCore, SHIPPED, shellPinProblems } from '../differential/cores.mjs';

/** Is `cargo` reachable? Only used to print a clearer message when the shared
 * loader decides it needs to rebuild and cannot. */
export function toolchainNote() {
  try {
    execFileSync('cargo', ['--version'], { stdio: 'pipe' });
    return null;
  } catch {
    return 'cargo is not on PATH; the shared loader can only gate the artifact as vendored.';
  }
}
