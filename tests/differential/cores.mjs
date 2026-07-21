/* Loading BOTH cores into one node process, so a fixture can be pushed through
 * each of them in the same tick and the two answers compared as data rather than
 * as prose.
 *
 * THE OLD CORE is not a source tree — it is the exact artifact that is serving the
 * running app right now:
 *   Stredio-Web/public/assets/heart/0.1.0-5ecaa78/{stredio_heart.js, *_bg.wasm}
 * Reading it from there rather than rebuilding Stredio-Heart from source is the
 * whole point. What has to keep working is the bytes on the device, not a tree
 * that happens to compile to something similar. Nothing here writes to that
 * folder; it is opened read-only and the app is never touched.
 *
 * THE NEW CORE is now read from exactly the same place, for exactly the same
 * reason:
 *   Stredio-Web/public/assets/groloo-core/<build>/{groloo_core.js, *_bg.wasm}
 *
 * ## Why the harness no longer builds its own copy
 *
 * It used to. `cargo build` + `wasm-bindgen` into `tests/.artifacts/new/`, and the
 * fixtures ran against that. Those bytes were never the bytes anybody deployed —
 * they differed from the vendored artifact by 176 bytes, because
 * `scripts/build-wasm.mjs` passes two `--remap-path-prefix` flags (rustc bakes
 * absolute source paths into the data section) and the harness's own invocation
 * did not. Two builds of the same commit, one gated and one shipped.
 *
 * That is the failure this whole phase exists to eliminate, reproduced inside the
 * thing meant to catch it: a green gate is evidence about the artifact it ran, and
 * an artifact nobody deploys is evidence about nothing. So the corpus now runs
 * against the vendored folder, and three separate things have to agree before a
 * single fixture is evaluated:
 *
 *   1. THE FILES MATCH THEIR OWN MANIFEST. Every file `manifest.json` lists is
 *      hashed off disk and compared, byte count and sha-256. This catches a
 *      truncated copy, a half-finished vendor, a hand-edited glue file.
 *   2. THE SOURCE STILL PRODUCES THEM. `scripts/build-wasm.mjs --no-vendor` is run
 *      whenever the crates are newer than the staged build, and its sha-256 must
 *      equal the vendored one. This catches the case that actually bites: someone
 *      fixes a merge rule, runs the gate, sees green, and ships an artifact built
 *      before the fix. There is no flag to skip it.
 *   3. THE SHELL POINTS AT THEM. `src/lib/heart.ts` pins `CORE_BUILD` and
 *      `CORE_WASM_SHA256`; both are read and compared. A mispinned shell loads a
 *      404 or the wrong core, which is precisely the failure the content-addressed
 *      folder was introduced to make impossible to miss.
 *
 * (1) and (2) throw before any fixture runs — a comparison against unknown bytes
 * is worse than no comparison. (3) is reported by `run.mjs` as a reservation
 * rather than thrown, because `heart.ts` belongs to Stredio-Web and re-vendoring
 * legitimately makes its pin stale for as long as it takes to paste two lines; the
 * run still prints every fixture, and `--strict` still exits non-zero.
 *
 * ## Nothing is written into public/
 *
 * `public/assets/` is served to browsers, and a stray `package.json` beside the
 * glue is exactly the kind of file a bundler notices at the worst moment. So the
 * `.wasm` is read into memory and handed to `initSync({ module })` — the shipped
 * bytes reach the WebAssembly compiler untouched, never copied — and only the glue
 * is mirrored into `tests/.artifacts/shipped/` so node has a directory it can treat
 * as ESM. The mirrored copy is re-hashed after the copy, so "we tested the glue
 * that ships" is checked and not assumed.
 *
 * Both glue files are `--target web` ESM modules, so neither can `fetch()` under
 * node. Both, however, export `initSync({ module })`, which takes raw bytes and
 * compiles them synchronously. No shim, no fetch polyfill, no chance of the harness
 * accidentally testing its own scaffolding. */

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..', '..');
const WEB = resolve(REPO, '..', 'Stredio-Web');

/* The pinned, currently-shipping vendored artifacts. If either path stops
 * existing the harness must fail loudly rather than quietly compare the new core
 * against itself — an all-green run against a missing baseline is worse than a
 * red one. */
export const OLD_DIR = join(WEB, 'public', 'assets', 'heart', '0.1.0-5ecaa78');
const ASSETS = join(WEB, 'public', 'assets', 'groloo-core');
const HEART_TS = join(WEB, 'src', 'lib', 'heart.ts');

/** Where `scripts/build-wasm.mjs` stages a build before vendoring it. */
const STAGE_DIR = join(REPO, 'tests', '.artifacts', 'build');
/** Where the shipped glue is mirrored so node will parse it as ESM. */
const MIRROR_DIR = join(REPO, 'tests', '.artifacts', 'shipped');

const WASM = 'groloo_core_bg.wasm';
const GLUE = 'groloo_core.js';

const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');

/* ------------------------------------------------------------------------- */
/* 1. the shipped artifact, and whether it is what it says it is               */
/* ------------------------------------------------------------------------- */

/** The one vendored build under `public/assets/groloo-core/`.
 *
 * Exactly one is required, and that is a deliberate constraint rather than a
 * simplification: the shell loads one folder, so two folders means the gate has to
 * guess which one it is testing, and guessing is the thing being eliminated.
 * `scripts/build-wasm.mjs --prune` is how a superseded build is retired. */
function shippedDir() {
  if (!existsSync(ASSETS)) {
    throw new Error(
      `no vendored core at ${ASSETS}. The differential run compares against the ` +
      'artifact Stredio-Web actually ships — build and vendor one with ' +
      '`node scripts/build-wasm.mjs --prune` before running the gate.',
    );
  }
  const builds = readdirSync(ASSETS, { withFileTypes: true })
    .filter((d) => d.isDirectory() && existsSync(join(ASSETS, d.name, 'manifest.json')))
    .map((d) => d.name)
    .sort();
  if (builds.length === 0) {
    throw new Error(`${ASSETS} contains no build with a manifest.json — nothing to gate.`);
  }
  if (builds.length > 1) {
    throw new Error(
      `${ASSETS} contains ${builds.length} builds (${builds.join(', ')}). The shell loads ` +
      'exactly one, so the gate cannot know which of them it would be testing. Retire the ' +
      'superseded one with `node scripts/build-wasm.mjs --prune`.',
    );
  }
  return join(ASSETS, builds[0]);
}

/** Hash every file `manifest.json` claims and compare. Returns the manifest.
 *
 * The manifest is a claim ABOUT the files; this is the only step that turns it
 * into a statement about the files themselves. Without it, a truncated vendor
 * ships a manifest describing bytes that are not there and every downstream check
 * agrees with itself. */
function verifyAgainstManifest(dir) {
  const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8'));
  const files = manifest.files || {};
  for (const name of [WASM, GLUE]) {
    if (!files[name]) throw new Error(`${dir}/manifest.json does not list ${name}`);
  }
  for (const [name, claim] of Object.entries(files)) {
    const path = join(dir, name);
    if (!existsSync(path)) {
      throw new Error(`${dir}/manifest.json lists ${name}, but it is not there.`);
    }
    const bytes = readFileSync(path);
    if (bytes.length !== claim.bytes) {
      throw new Error(
        `${name} is ${bytes.length} bytes; its manifest says ${claim.bytes}. The vendored ` +
        'artifact is not the artifact its manifest describes — do not gate against it.',
      );
    }
    const got = sha256(bytes);
    if (got !== claim.sha256) {
      throw new Error(
        `${name} hashes ${got}; its manifest says ${claim.sha256}. The vendored artifact is ` +
        'not the artifact its manifest describes — do not gate against it.',
      );
    }
  }
  return manifest;
}

/* ------------------------------------------------------------------------- */
/* 2. does the source still produce those exact bytes?                        */
/* ------------------------------------------------------------------------- */

/* Rebuild when the staged build is missing or older than the crate sources.
 * Comparing against the CRATE SOURCES rather than against the compiled wasm is
 * what stops "I edited api.rs and the harness still passed" from being possible.
 *
 * `scripts/build-wasm.mjs` rather than a `cargo build` spelled out here: it is the
 * script that produces the vendored artifact, and a freshness check performed with
 * different flags than the release build is a freshness check that reports a
 * difference on every run and teaches everyone to ignore it. That is not
 * hypothetical — it is exactly how the harness ended up gating a 319,319-byte
 * build while 319,143 bytes shipped. */
function stagedBuild() {
  const manifestPath = join(STAGE_DIR, 'manifest.json');
  if (!existsSync(manifestPath) || newestMtime(crateSources()) > statSync(manifestPath).mtimeMs) {
    execFileSync('node', [join(REPO, 'scripts', 'build-wasm.mjs'), '--no-vendor'], {
      cwd: REPO, stdio: 'inherit',
    });
  }
  return JSON.parse(readFileSync(manifestPath, 'utf8'));
}

const crateSources = () => [
  join(REPO, 'crates', 'groloo-core', 'src'),
  join(REPO, 'crates', 'groloo-core-wasm', 'src'),
  join(REPO, 'Cargo.lock'),
  join(REPO, 'Cargo.toml'),
];

function newestMtime(paths) {
  return paths.reduce((max, p) => Math.max(max, newest(p)), 0);
  function newest(path) {
    if (!existsSync(path)) return 0;
    const st = statSync(path);
    if (!st.isDirectory()) return st.mtimeMs;
    return readdirSync(path).reduce((m, name) => Math.max(m, newest(join(path, name))), st.mtimeMs);
  }
}

/** The check with no escape hatch: what this tree builds must BE what ships. */
function assertShippedIsCurrent(shipped, staged) {
  const drift = [WASM, GLUE]
    .filter((n) => shipped.files[n].sha256 !== staged.files?.[n]?.sha256)
    .map((n) => `  ${n}\n    shipped  ${shipped.files[n].sha256} (${shipped.files[n].bytes} bytes)\n    built    ${staged.files?.[n]?.sha256} (${staged.files?.[n]?.bytes} bytes)`);
  if (!drift.length) return;
  throw new Error(
    'THE VENDORED ARTIFACT IS NOT WHAT THIS TREE BUILDS.\n\n' +
    `${drift.join('\n')}\n\n` +
    `Gating build ${shipped.build} would prove something about bytes nobody is going to run. ` +
    'Re-vendor first:\n\n    node scripts/build-wasm.mjs --prune\n\n' +
    '…then update the pin in Stredio-Web/src/lib/heart.ts and run the gate again.',
  );
}

/* ------------------------------------------------------------------------- */
/* 3. does the shell point at it?                                             */
/* ------------------------------------------------------------------------- */

/** Read `CORE_BUILD` / `CORE_WASM_SHA256` out of `heart.ts` and compare them to
 * the vendored build. Returns a list of human-readable problems, empty when the
 * pin is right.
 *
 * Text-scraped rather than imported because `heart.ts` is TypeScript in another
 * repo with its own imports, and a gate that needed a bundler to answer "which
 * folder does the app load" would be a gate nobody runs. The two constants are
 * single-quoted string literals by construction — `scripts/build-wasm.mjs` prints
 * the exact lines to paste. */
function shellPin(shipped) {
  if (!existsSync(HEART_TS)) {
    return { problems: [`${HEART_TS} is missing — the shell's pin could not be checked at all.`] };
  }
  const src = readFileSync(HEART_TS, 'utf8');
  const read = (name) => {
    const m = new RegExp(`^const ${name} = '([^']*)'`, 'm').exec(src);
    return m ? m[1] : null;
  };
  const build = read('CORE_BUILD');
  const wasmSha = read('CORE_WASM_SHA256');
  const problems = [];
  if (build === null) problems.push('heart.ts declares no CORE_BUILD.');
  else if (build !== shipped.build) {
    problems.push(`heart.ts pins CORE_BUILD '${build}'; the vendored build is '${shipped.build}'.`);
  }
  if (wasmSha === null) problems.push('heart.ts declares no CORE_WASM_SHA256.');
  else if (wasmSha !== shipped.files[WASM].sha256) {
    problems.push(
      `heart.ts pins CORE_WASM_SHA256 '${wasmSha}'; the vendored ${WASM} hashes ` +
      `'${shipped.files[WASM].sha256}'.`,
    );
  }
  return { build, wasmSha, problems };
}

/* ------------------------------------------------------------------------- */
/* loading                                                                     */
/* ------------------------------------------------------------------------- */

/** The vendored 0.1.0-5ecaa78 core, exactly as the browser loads it. */
export async function loadOld() {
  /* The pre-rename artifact keeps its ORIGINAL filenames on purpose: it is a frozen
   * historical build and the baseline this harness compares against. Renaming inside
   * it would change what "old" means. A project-wide rename corrupted it once — it
   * rewrote the glue's reference to its own .wasm sibling, so the pair stopped naming
   * each other and this loader could not find it. Leave these two literals alone. */
  const glue = join(OLD_DIR, 'stredio_heart.js');
  const bytes = join(OLD_DIR, 'stredio_heart_bg.wasm');
  if (!existsSync(glue) || !existsSync(bytes)) {
    throw new Error(
      `the live vendored core is not at ${OLD_DIR}. The differential run is ` +
      'meaningless without it — refusing to continue.',
    );
  }
  const mod = await import(pathToFileURL(glue).href);
  mod.initSync({ module: readFileSync(bytes) });
  return mod;
}

/** Everything known about the artifact under test, resolved once. */
export const SHIPPED = (() => {
  const dir = shippedDir();
  const manifest = verifyAgainstManifest(dir);
  return { dir, manifest };
})();

/** The core Stredio-Web ships — verified against its manifest, against a fresh
 * build of this tree, and cross-checked against the shell's pin. */
export async function loadNew() {
  assertShippedIsCurrent(SHIPPED.manifest, stagedBuild());

  mkdirSync(MIRROR_DIR, { recursive: true });
  // wasm-bindgen emits ESM with a `.js` extension. Without a package.json beside
  // it node re-parses the file and warns on every run, and the first thing anyone
  // sees under a real failure would be an irrelevant warning. It goes HERE and
  // never into public/assets, which is served to browsers.
  writeFileSync(join(MIRROR_DIR, 'package.json'), '{"type":"module"}\n');

  const mirroredGlue = join(MIRROR_DIR, GLUE);
  copyFileSync(join(SHIPPED.dir, GLUE), mirroredGlue);
  const mirroredSha = sha256(readFileSync(mirroredGlue));
  if (mirroredSha !== SHIPPED.manifest.files[GLUE].sha256) {
    throw new Error(
      `the mirrored copy of ${GLUE} hashes ${mirroredSha}, not ` +
      `${SHIPPED.manifest.files[GLUE].sha256}. The copy is not the shipped file.`,
    );
  }

  const mod = await import(pathToFileURL(mirroredGlue).href);
  // The .wasm is never copied: the shipped bytes go straight from public/assets
  // into the WebAssembly compiler, which is the strongest form of "these are the
  // bytes under test" available.
  mod.initSync({ module: readFileSync(join(SHIPPED.dir, WASM)) });
  return mod;
}

/** The shell-pin cross-check, for `run.mjs` to report. Not thrown: `heart.ts`
 * belongs to Stredio-Web, and re-vendoring makes its pin stale for exactly as long
 * as it takes to paste two lines. It is a reservation, and `--strict` treats every
 * reservation as a failure. */
export function shellPinProblems() {
  return shellPin(SHIPPED.manifest).problems;
}
