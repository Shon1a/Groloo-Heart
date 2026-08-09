/*
 * Build the WebAssembly artifact, prove it, CONTENT-ADDRESS it, and vendor it
 * into Groloo-Web.
 *
 *   node scripts/build-wasm.mjs                 # build + vendor + write manifest
 *   node scripts/build-wasm.mjs --prune         # …and delete superseded builds
 *   node scripts/build-wasm.mjs --out <dir>     # vendor somewhere else
 *   node scripts/build-wasm.mjs --no-vendor     # leave it in tests/.artifacts/build
 *
 * ── RE-VENDOR ON LINUX. THE BYTES ARE HOST-DEPENDENT. ─────────────────────────
 *
 * The same source, the same rustc (1.96.1) and the same wasm-bindgen (0.2.126)
 * produce DIFFERENT wasm on different hosts:
 *
 *   linux    434,880 bytes
 *   windows  434,641 bytes      — 239 bytes smaller, every time
 *
 * Measured, not theorised: CI rebuilt the tree and got the linux figure while a
 * local Windows build of the identical commit got the other, and the harness had
 * already ruled out the build stamp (it pins the rebuild to the revision the
 * vendored artifact records).
 *
 * THE CONSEQUENCE IS A GATE THAT CANNOT PASS. `assertShippedIsCurrent` rebuilds
 * and compares against the vendored artifact, so vendoring from Windows makes
 * `differential + parity` fail on every subsequent push — the artifact is not
 * what CI's tree builds, and CI is the tree that matters. It reads exactly like a
 * stale pin, which is how an hour went into re-vendoring a core that was fine.
 *
 * So: run this on linux (or WSL, or a container). A Windows run is still useful
 * for `--no-vendor`, where nothing is pinned to the output. */
 * ── WHY THIS EXISTS, when .github/scripts/build-wasm.sh already builds ─────────
 *
 * That script produces bytes. It does not produce an IDENTITY, and the identity is
 * the thing that was broken.
 *
 * The shell pins the core it loads so that a stale or mispointed artifact is an
 * error rather than "the app behaves like an old build". Until now that pin was
 * `core_version()`, which is `<semver>+g<git rev>` — and this repository has no
 * commits, so every build answers `0.2.0+gunknown` and `heart.ts` was pinned
 * against a revision that does not exist. Three things are wrong with that, and
 * only the first is fixed by committing:
 *
 *   1. `unknown` is not an identity. Two different trees answer the same string.
 *   2. Even a real git sha names the SOURCE, not the ARTIFACT. The failure being
 *      guarded against is a vendored folder whose bytes are not the bytes it
 *      claims — a stale copy, a truncated transfer, a half-finished deploy. Such
 *      an artifact reports the version it was compiled with, cheerfully and
 *      correctly, while being the wrong file. A version string cannot catch it.
 *   3. It makes the release chore "commit, then rebuild, then commit the rebuild",
 *      which is how Heart ended up shipping a hand-committed artifact nothing
 *      verified. The point of this phase was to kill that loop, not relocate it.
 *
 * So the identity is the **sha-256 of the built `.wasm`**. It needs no commit, it
 * is reproducible from source by anyone, it changes exactly when the shipped bytes
 * change, and verifying it detects the actual failure: hash what was fetched,
 * compare, done. `manifest.json` beside the artifact carries it, plus the glue's
 * hash and both byte sizes, so the shell can check the file it does NOT hash at
 * load time (the .js it just imported) by size at least, and a human can check
 * both with `sha256sum` and no tooling.
 *
 * `core_version()` stays exported and stays stamped into every envelope — it is
 * good provenance and it is what a bug report should quote. It is simply no longer
 * load-bearing, which is the only honest place for a string that can say `unknown`.
 *
 * ── REPRODUCIBILITY IS A PRECONDITION, not a nice-to-have ─────────────────────
 *
 * A content hash that differs per machine is worse than no hash: it turns "these
 * bytes are wrong" into noise everybody learns to ignore. rustc bakes the absolute
 * source path of every potentially-panicking line into the data section, including
 * paths inside CARGO_HOME, and those differ in LENGTH between machines — which
 * shifts every downstream i32.const and changes the whole code section. The two
 * `--remap-path-prefix` flags below are byte-identical to the ones in
 * .github/scripts/build-wasm.sh precisely so the two scripts agree; if you change
 * one, change both or the hashes stop meaning anything.
 *
 * Node built-ins only, on purpose: this runs on a clean checkout with no install
 * step, on Windows and on CI alike.
 */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { mkdir, readdir, readFile, rename, rm, writeFile, cp } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
import { webRepo } from './sibling.mjs';
const ROOT = resolve(HERE, '..');

const CRATE = 'groloo-core-wasm';
const TARGET = 'wasm32-unknown-unknown';
/* The cargo lib name, which is what lands in target/. */
const LIB = 'groloo_core_wasm';
/* The SHIPPED name. `--out-name` renames the pair, and it must stay `groloo_core`
 * because the glue resolves the .wasm relative to its own import.meta.url and
 * `heart.ts` imports `groloo_core.js` by that literal path. Renaming either half
 * breaks a reference baked into generated code. */
const OUT_NAME = 'groloo_core';

/* Default vendor target: the sibling checkout. Resolved, not configured, because a
 * two-repo layout that anybody has to describe in a README is a layout people get
 * wrong once and then work around forever. --out overrides it. */
const DEFAULT_ASSETS = resolve(webRepo(ROOT), 'public', 'assets', 'groloo-core');
const STAGE_DIR = join(ROOT, 'tests', '.artifacts', 'build');

const MANIFEST_SCHEMA = 1;

/* Floors, not estimates. The real module is ~350 KB and the glue ~24 KB, so
 * anything under these is a truncated write or an error body — never a build. */
const MIN_WASM = 100 * 1024;
const MIN_JS = 8 * 1024;

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name) => {
  const i = args.indexOf(name);
  if (i < 0) return null;
  const v = args[i + 1];
  if (!v || v.startsWith('--')) die(`${name} needs a value`);
  return v;
};
function die(msg) {
  console.error(`build-wasm: ${msg}`);
  process.exit(1);
}

/* ── toolchain: the CLI and the linked crate must be the SAME version ──────────
 *
 * Cargo.lock rather than the manifest, because the manifest's `=0.2.126` could be
 * loosened by a refactor while the lockfile records what was actually linked. A
 * mismatch here does not reliably error — it can emit a JS shim that is subtly
 * wrong at runtime, on a television, months later. */
function lockedWasmBindgen() {
  const lock = readFileSync(join(ROOT, 'Cargo.lock'), 'utf8');
  const m = /\bname = "wasm-bindgen"\r?\nversion = "([^"]+)"/.exec(lock);
  if (!m) die('could not read the wasm-bindgen version from Cargo.lock');
  return m[1];
}

function cliVersion(bin) {
  try {
    return execFileSync(bin, ['--version'], { encoding: 'utf8' }).trim().split(/\s+/)[1];
  } catch {
    return null;
  }
}

/* ── build ────────────────────────────────────────────────────────────────────
 *
 * CARGO_ENCODED_RUSTFLAGS rather than RUSTFLAGS: the latter is split on spaces, so
 * a checkout under "C:\My Projects" would silently produce two broken half-flags.
 * The separator is a literal 0x1f. */
function build() {
  const cargoHome = process.env.CARGO_HOME || join(homedir(), '.cargo');
  const US = '\x1f';
  const env = {
    ...process.env,
    CARGO_ENCODED_RUSTFLAGS: [
      `--remap-path-prefix=${join(cargoHome, 'registry', 'src')}=/cargo/registry/src`,
      `--remap-path-prefix=${ROOT}=/groloo-core`,
    ].join(US),
  };

  console.log(`==> workspace     ${ROOT}`);
  console.log(`==> rustc         ${cliVersion('rustc') ?? '?'}`);
  execFileSync('cargo', ['build', '--release', '--target', TARGET, '-p', CRATE], {
    cwd: ROOT, stdio: 'inherit', env,
  });
}

/* ── prove it ─────────────────────────────────────────────────────────────────
 *
 * Shape checks first, because each catches a different lie, and then the one that
 * actually matters: INSTANTIATE the module and call it. Everything cheap proves
 * the bytes are shaped like a core; only this proves they ARE one. It also means
 * the version recorded in the manifest is read out of the module rather than
 * transcribed from a Cargo.toml that may not be the tree these bytes came from.
 *
 * The glue is a `--target web` ESM module so it cannot fetch() under node, but it
 * also exports initSync(), which compiles raw bytes synchronously — handing them
 * over directly means nothing here is testing its own scaffolding. */
async function probe(dir, wasm) {
  if (wasm.subarray(0, 4).toString('binary') !== '\0asm') {
    die(`${OUT_NAME}_bg.wasm is not a WebAssembly module (starts "${wasm.subarray(0, 16).toString('utf8')}")`);
  }
  /* wasm-bindgen emits ESM with a `.js` extension and no package.json, which node
   * reparses with a warning on every run. Removed again before the folder ships —
   * a stray package.json inside public/assets is exactly the kind of thing a
   * bundler notices at the worst possible moment. */
  const pkg = join(dir, 'package.json');
  await writeFile(pkg, '{"type":"module"}\n');
  try {
    const mod = await import(pathToFileURL(join(dir, `${OUT_NAME}.js`)).href);
    mod.initSync({ module: wasm });
    const version = mod.core_version();
    if (typeof version !== 'string' || !/^\d+\.\d+\.\d+/.test(version)) {
      throw new Error(`core_version() answered ${JSON.stringify(version)}, which is not a semver`);
    }
    /* All 22, by name. A shim that dropped an export still builds, still
     * instantiates, and fails at the call site — on a device, as
     * `undefined is not a function`. */
    for (const fn of EXPORTS) {
      if (typeof mod[fn] !== 'function') die(`the module exports no ${fn}() — wrong artifact?`);
    }
    /* One real call through the boundary. `core_constants()` is enveloped, takes
     * no input and cannot be satisfied by a stub, so it proves the envelope format
     * and the JSON boundary are alive without needing a fixture. */
    const env = JSON.parse(mod.core_constants());
    if (env?.ok !== true || typeof env?.data?.historyCap !== 'number') {
      die(`core_constants() answered ${JSON.stringify(env).slice(0, 200)} — the boundary is not intact`);
    }
    /* And one call that passes strings IN. Everything above marshals only
     * outwards, so a broken `&str` argument path — the half of the ABI that
     * wasm-bindgen writes into the glue, and therefore the half a CLI/crate
     * version skew corrupts — would still probe clean. `addon_resource_path` is
     * the cheapest witness: three `&str` in, total, no fixture, and an answer
     * that is wrong in a *visible* way if the encoding drifts. */
    const path = JSON.parse(mod.addon_resource_path('stream', 'series', 'tt0903747:1:4'));
    if (path?.data !== 'stream/series/tt0903747%3A1%3A4.json') {
      die(`addon_resource_path() answered ${JSON.stringify(path?.data)} — string arguments do not cross intact`);
    }
    return version;
  } catch (err) {
    die(`the artifact would not instantiate: ${err?.message || err}`);
  } finally {
    await rm(pkg, { force: true });
  }
}

/* The boundary, by name, in `api.rs` order. This list is the reason a dropped
 * export is a build failure here rather than `undefined is not a function` in a
 * living room, so it is maintained by hand on purpose: it has to be able to
 * disagree with the module, and a list derived from the module never can. */
const EXPORTS = [
  'official_payload_file', 'merge_official', 'reconcile_install_state',
  'merge_library', 'library_record_watch', 'library_remove', 'mylist_toggle',
  'continue_watching', 'resume_position', 'normalize_manifest_url',
  'addon_base_url', 'validate_manifest', 'manifest_has_resource',
  'addon_catalogs', 'visible_rows', 'core_version', 'core_constants',
  'stream_parse', 'catalog_metas', 'addon_resource_path', 'order_langs',
  'rank_streams',
];

const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');

/* ── run ──────────────────────────────────────────────────────────────────── */

const locked = lockedWasmBindgen();
const cli = cliVersion('wasm-bindgen');
if (!cli) die(`wasm-bindgen CLI not on PATH; install exactly ${locked}`);
if (cli !== locked) {
  die(`wasm-bindgen CLI is ${cli} but Cargo.lock links ${locked}. These must be equal — ` +
      'a mismatch can emit a silently wrong JS shim.');
}
console.log(`==> wasm-bindgen  ${cli} (matches Cargo.lock)`);

build();

/* Wiped, not merged. A stale file left behind by an earlier run would sail through
 * a file-by-file comparison and ship to every client. */
await rm(STAGE_DIR, { recursive: true, force: true });
await mkdir(STAGE_DIR, { recursive: true });
execFileSync('wasm-bindgen', [
  join(ROOT, 'target', TARGET, 'release', `${LIB}.wasm`),
  '--out-dir', STAGE_DIR,
  '--out-name', OUT_NAME,
  '--target', 'web',
], { cwd: ROOT, stdio: 'inherit' });

const wasmName = `${OUT_NAME}_bg.wasm`;
const jsName = `${OUT_NAME}.js`;
const wasm = await readFile(join(STAGE_DIR, wasmName));
const js = await readFile(join(STAGE_DIR, jsName));
if (wasm.length < MIN_WASM) die(`${wasmName} is only ${wasm.length} bytes — truncated`);
if (js.length < MIN_JS) die(`${jsName} is only ${js.length} bytes — truncated`);

const version = await probe(STAGE_DIR, wasm);

const wasmSha = sha256(wasm);
const jsSha = sha256(js);

/* The folder name is derived from the content hash and nothing else. The path is
 * the cache key — served immutable, cached forever — so it MUST be unique per
 * bytes, and only the hash can promise that. The semver prefix is there so a human
 * reading a devtools network tab can tell at a glance which core they are looking
 * at; it carries no guarantee, and the 16 hex digits after it carry all of it.
 * Sixteen rather than eight because this string is a security-adjacent cache key
 * that outlives the release, and 64 bits costs nothing to type once. */
const semver = version.split('+')[0];
const build_ = `${semver}-${wasmSha.slice(0, 16)}`;

/* `+g<rev>` is provenance, not identity, and it is honest about being absent: this
 * repo has no commits, so `unknown` is the truthful answer and null is how JSON
 * says so. Nothing downstream may branch on it. */
const revSuffix = version.includes('+g') ? version.split('+g')[1] : '';
const gitSha = revSuffix && revSuffix !== 'unknown' ? revSuffix : null;

const manifest = {
  schema: MANIFEST_SCHEMA,
  name: 'groloo-core',
  build: build_,
  version: semver,
  coreVersion: version,
  gitSha,
  entry: jsName,
  files: {
    [wasmName]: { bytes: wasm.length, sha256: wasmSha },
    [jsName]: { bytes: js.length, sha256: jsSha },
  },
  builtAt: new Date().toISOString(),
  toolchain: { rustc: cliVersion('rustc') ?? null, wasmBindgen: cli },
};
await writeFile(join(STAGE_DIR, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);

console.log('');
console.log(`  ${wasmName.padEnd(24)} ${wasm.length} bytes  sha256 ${wasmSha}`);
console.log(`  ${jsName.padEnd(24)} ${js.length} bytes  sha256 ${jsSha}`);
console.log(`  core_version()           "${version}"`);

if (flag('--no-vendor')) {
  console.log(`\nbuilt ${build_} -> ${STAGE_DIR} (not vendored)`);
  process.exit(0);
}

const assets = opt('--out') ? resolve(opt('--out')) : DEFAULT_ASSETS;
if (!existsSync(dirname(assets))) die(`no such directory: ${dirname(assets)} (pass --out <dir>)`);
const outDir = join(assets, build_);

await mkdir(assets, { recursive: true });
await rm(outDir, { recursive: true, force: true });
await mkdir(outDir, { recursive: true });
/* Only the three files the browser needs. wasm-bindgen also emits .d.ts, which is
 * a build-time artifact: `heart.ts` hand-writes its own `CoreExports` interface on
 * purpose, so shipping the generated types into public/ would put a second,
 * unread declaration of the boundary on the origin. */
for (const f of [wasmName, jsName, 'manifest.json']) {
  await cp(join(STAGE_DIR, f), join(outDir, f));
}

/* Superseded builds are deleted only when asked. The previous folder is the
 * rollback until the new one has actually shipped, and a build script that
 * silently removes the thing you roll back to is a build script that turns a bad
 * deploy into an outage. */
let pruned = [];
if (flag('--prune')) {
  const siblings = await readdir(assets, { withFileTypes: true });
  for (const d of siblings) {
    if (!d.isDirectory() || d.name === build_) continue;
    await rm(join(assets, d.name), { recursive: true, force: true });
    pruned.push(d.name);
  }
}

console.log(`\nvendored -> ${outDir}`);
if (pruned.length) console.log(`pruned    ${pruned.join(', ')}`);
console.log('\nThe shell pins the CONTENT, not the version. In src/lib/heart.ts:\n');
console.log(`const CORE_BUILD = '${build_}';`);
console.log(`const CORE_WASM_SHA256 = '${wasmSha}';`);
console.log(`const CORE_JS = \`/assets/groloo-core/\${CORE_BUILD}/${jsName}\`;`);
console.log('');
