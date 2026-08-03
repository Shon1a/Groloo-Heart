/*
 * The artifact-freshness gate, and the size budget that rides along with it.
 *
 * THE FAILURE THIS EXISTS TO MAKE IMPOSSIBLE. Groloo-Heart committed its
 * wasm-bindgen output to `web/` by hand and its CI never rebuilt it. A source
 * change with no manual rebuild therefore shipped a STALE core to every client,
 * indistinguishably from a correct one: green CI, green tests, and a television
 * running last month's logic. Nothing in the repository could tell the two apart.
 *
 * WHAT THIS GATE CATCHES — stated precisely, because a gate whose reach is
 * misunderstood is worse than no gate at all:
 *
 *   [1] file set      A file added to, or vanished from, the artifact directory.
 *   [2] glue          ANY byte-level difference in the wasm-bindgen JS/TS glue.
 *                     This is pure codegen from the crate's #[wasm_bindgen]
 *                     metadata: no paths, no timestamps, no git sha. It moves if
 *                     an export is added, removed, renamed or re-signatured, or
 *                     if the wasm-bindgen CLI version drifts.
 *   [3] content       The module's own bytes, path-separator-normalised (see
 *                     wasm-summary.mjs). Byte-for-byte equality of everything
 *                     including the code section — so a change to the BODY of an
 *                     existing function, which moves no signature and no export,
 *                     is caught. This is the gate that closes Heart's hole.
 *   [4] interface     Exports, imports, section inventory, memory limits. Fully
 *                     redundant with [3] when [3] passes; its job is to turn a
 *                     red [3] into a sentence a human can act on.
 *   [5] stamp         Exactly one `<semver>+g<rev>` in the module, and its semver
 *                     equal to the workspace version — so bumping the version
 *                     without rebuilding is a build failure rather than a
 *                     mislabelled folder in Groloo-Web's `public/assets/`.
 *   [6] budget        Byte ceilings on the .wasm and on the JS glue.
 *
 * WHAT IT DOES NOT CATCH, exhaustively:
 *
 *   - A source change with NO effect on generated code (a comment, a doc line
 *     that is not a #[wasm_bindgen] doc comment, a test). Correct: such a change
 *     does not make the artifact stale.
 *   - A change whose entire binary effect is swapping `/` for `\` inside a string
 *     literal in the data section. That single byte class is normalised away.
 *     Nothing else is normalised.
 *
 * THE ARTIFACT MUST BE BUILT ON LINUX, and this note replaces an earlier claim
 * here that the separator normalisation was enough to let "a Windows-built
 * artifact and a Linux rebuild be compared at all". It is not, and the first CI
 * run to reach this gate proved it. A Windows host and the ubuntu-latest runner,
 * both on the pinned 1.96.1 and both through build-wasm.sh, produce:
 *
 *     data 40,675 == 40,675      the remapping works; paths are fully handled
 *     code 391,438 vs 391,666    228 bytes apart
 *     func 1,054   vs 1,053
 *     total 434,662 vs 434,901
 *
 * Identical exports, identical imports, identical data — so this is host codegen
 * drift in rustc/LLVM, not a path leak and not something normalisation can reach.
 * Cross-host reproducibility is not a property rustc offers, and gate [3] is
 * doing exactly its job when it refuses the pair.
 *
 * So: pkg/ is produced on Linux. Do not commit a module built on Windows — it
 * will fail [3] against every rebuild forever, for a reason that looks like a
 * source defect and is not. The `check`/`wasm` jobs and the whole test suite run
 * fine on Windows; it is only these bytes that are host-bound. The practical
 * route needs no local Linux: push, let the artifact job go red, and take the
 * `pkg-rebuilt-<sha>` upload it produces on failure for precisely this purpose —
 * that is what the "Upload the rebuilt artifact / if: always()" step is for.
 *   - Anything at all if the artifact was NOT built by build-wasm.sh. Without its
 *     --remap-path-prefix flags the two modules differ in data-section LENGTH,
 *     every downstream offset shifts, and gate [3] fails loudly. It fails
 *     closed — it does not pass — but the reported cause will be "content
 *     differs" when the real cause is "you used the wrong build command".
 *   - A defect that is present identically in both the source and the artifact.
 *     That is what `cargo test` and the differential harness are for.
 */

import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { summarise } from './wasm-summary.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..', '..');

const LIB = 'groloo_core_wasm';
const WASM_FILE = `${LIB}_bg.wasm`;
const GLUE_FILE = `${LIB}.js`;

/* ------------------------------------------------------------------ args --- */

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1 || i + 1 >= process.argv.length) return fallback;
  return process.argv[i + 1];
}

const committedDir = arg('committed', join(ROOT, 'pkg'));
const rebuiltDir = arg('rebuilt');
/* Defaults mirror WASM_BUDGET_BYTES / JS_BUDGET_BYTES in .github/workflows/ci.yml,
 * so a bare local run reaches the same verdict CI does. ci.yml is the source of
 * truth and carries the reasoning; these are here only so the script is usable
 * without remembering two numbers. */
const maxWasmBytes = Number(arg('max-wasm-bytes', '409600'));
const maxJsBytes = Number(arg('max-js-bytes', '32768'));

if (!rebuiltDir) {
  console.error('usage: verify-artifact.mjs --rebuilt <dir> [--committed <dir>]' +
    ' [--max-wasm-bytes N] [--max-js-bytes N]');
  process.exit(2);
}

/* A budget that silently became NaN would compare false against everything and
 * hand back a green tick, which is the one outcome this file exists to prevent.
 * Exit 2 (misuse) rather than 1 (gate failed) so the two are never confused. */
if (!Number.isFinite(maxWasmBytes) || !Number.isFinite(maxJsBytes)) {
  console.error('--max-wasm-bytes and --max-js-bytes must be numbers; got ' +
    `${maxWasmBytes} and ${maxJsBytes}`);
  process.exit(2);
}

/* The workspace version, read from the manifest rather than passed in, so the
 * gate cannot be told a convenient lie by the caller. Only the [workspace.package]
 * table is searched; a crate-local `version.workspace = true` is not a version. */
function workspaceVersion() {
  const toml = readFileSync(join(ROOT, 'Cargo.toml'), 'utf8');
  const block = toml.split(/^\[/m).find((s) => s.startsWith('workspace.package]'));
  const m = block && block.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error('could not read [workspace.package] version from Cargo.toml');
  return m[1];
}

/* --------------------------------------------------------------- reporting --- */

const failures = [];
const lines = [];

function report(gate, ok, detail) {
  lines.push(`${ok ? 'PASS' : 'FAIL'}  [${gate}] ${detail}`);
  console.log(`${ok ? '  ok  ' : ' FAIL '} [${gate}] ${detail}`);
  if (!ok) {
    failures.push(`[${gate}] ${detail}`);
    /* A workflow annotation, so the failure lands on the PR's Files tab rather
     * than only in a log nobody opens. */
    console.log(`::error::[${gate}] ${detail.replace(/\r?\n/g, ' ')}`);
  }
}

function detailBlock(title, body) {
  console.log(`\n--- ${title} ---\n${body}\n`);
}

function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

function listFiles(dir) {
  return readdirSync(dir).filter((f) => statSync(join(dir, f)).isFile()).sort();
}

function kib(n) {
  return `${n} B (${(n / 1024).toFixed(1)} KiB)`;
}

/* ------------------------------------------------------------------ gates --- */

if (!existsSync(committedDir)) {
  console.error(`::error::The committed artifact directory ${committedDir} does not exist.`);
  console.error('The wasm-bindgen output is a versioned deliverable in this repo, not a');
  console.error('build by-product: Groloo-Web loads exactly these bytes. Produce it with');
  console.error('  .github/scripts/build-wasm.sh pkg');
  console.error('and commit the result.');
  process.exit(1);
}

if (!existsSync(rebuiltDir)) {
  console.error(`::error::The rebuild directory ${rebuiltDir} does not exist — ` +
    'build-wasm.sh did not produce anything. This is a broken gate, not a stale ' +
    'artifact; do not read the absence of failures below as a pass.');
  process.exit(2);
}

const committedFiles = listFiles(committedDir);
const rebuiltFiles = listFiles(rebuiltDir);

/* [1] file set */
{
  const missing = rebuiltFiles.filter((f) => !committedFiles.includes(f));
  const extra = committedFiles.filter((f) => !rebuiltFiles.includes(f));
  const ok = missing.length === 0 && extra.length === 0;
  report('1 file set', ok, ok
    ? `${committedFiles.length} files, identical set: ${committedFiles.join(', ')}`
    : `missing from the commit: [${missing.join(', ') || 'none'}]; ` +
      `committed but not produced: [${extra.join(', ') || 'none'}]`);
}

/* [2] glue — every non-wasm file, byte for byte */
{
  const glue = rebuiltFiles.filter((f) => !f.endsWith('.wasm'));
  const differing = [];
  for (const f of glue) {
    if (!committedFiles.includes(f)) continue;
    const a = readFileSync(join(committedDir, f));
    const b = readFileSync(join(rebuiltDir, f));
    if (!a.equals(b)) {
      differing.push(`${f} (committed ${a.length} B / ${sha256(a).slice(0, 12)}, ` +
        `rebuilt ${b.length} B / ${sha256(b).slice(0, 12)})`);
    }
  }
  report('2 glue', differing.length === 0, differing.length === 0
    ? `${glue.length} generated JS/TS files byte-identical to a rebuild`
    : `generated glue is stale: ${differing.join('; ')}`);
}

/* Both modules are needed by [3], [4] and [5]. */
const committedWasm = existsSync(join(committedDir, WASM_FILE))
  ? readFileSync(join(committedDir, WASM_FILE)) : null;
const rebuiltWasm = existsSync(join(rebuiltDir, WASM_FILE))
  ? readFileSync(join(rebuiltDir, WASM_FILE)) : null;

if (!committedWasm || !rebuiltWasm) {
  report('3 content', false, `${WASM_FILE} missing from ` +
    `${!committedWasm ? committedDir : rebuiltDir}`);
} else {
  const a = summarise(committedWasm);
  const b = summarise(rebuiltWasm);

  /* [3] content */
  {
    const ok = a.normalisedSha256 === b.normalisedSha256;
    report('3 content', ok, ok
      ? `${WASM_FILE} matches a rebuild from source ` +
        `(normalised sha256 ${a.normalisedSha256.slice(0, 16)}…` +
        `${a.rawSha256 === b.rawSha256 ? ', and raw bytes are identical too' :
          '; raw bytes differ only in data-section path separators'})`
      : `${WASM_FILE} is NOT what this source builds. ` +
        `committed ${a.byteLength} B / ${a.normalisedSha256.slice(0, 16)}…, ` +
        `rebuilt ${b.byteLength} B / ${b.normalisedSha256.slice(0, 16)}…. ` +
        'Rebuild with .github/scripts/build-wasm.sh pkg and commit the result.');
  }

  /* [4] interface — diagnosis for a red [3] */
  {
    const project = (s) => ({
      sections: s.sections, counts: s.counts, memories: s.memories,
      imports: s.imports, exports: s.exports,
    });
    const pa = JSON.stringify(project(a), null, 2);
    const pb = JSON.stringify(project(b), null, 2);
    const ok = pa === pb;
    report('4 interface', ok, ok
      ? `${a.exports.length} exports, ${a.imports.length} imports, ` +
        `${a.sections.length} sections — unchanged`
      : 'the committed module\'s exports/imports/sections differ from a rebuild');
    if (!ok) {
      const names = (s) => s.exports.map((e) => `${e.kind} ${e.name}`);
      const addedExports = names(b).filter((n) => !names(a).includes(n));
      const goneExports = names(a).filter((n) => !names(b).includes(n));
      detailBlock('interface drift', [
        `exports the rebuild has and the commit lacks: ${addedExports.join(', ') || '(none)'}`,
        `exports the commit has and the rebuild lacks: ${goneExports.join(', ') || '(none)'}`,
        `committed sections: ${a.sections.map((s) => `${s.customName ?? s.name}:${s.size}`).join(' ')}`,
        `rebuilt   sections: ${b.sections.map((s) => `${s.customName ?? s.name}:${s.size}`).join(' ')}`,
      ].join('\n'));
    }
  }

  /* [5] stamp */
  {
    const version = workspaceVersion();
    const stamps = a.versionStamps;
    if (stamps.length !== 1) {
      report('5 stamp', false, `expected exactly one <semver>+g<rev> stamp in the ` +
        `committed module, found ${stamps.length}: ${stamps.join(', ') || '(none)'}`);
    } else {
      const [semver] = stamps[0].split('+g');
      report('5 stamp', semver === version, semver === version
        ? `committed module reports ${stamps[0]}, matching workspace version ${version}`
        : `committed module reports ${stamps[0]} but the workspace is at ${version}. ` +
          'The version was bumped without rebuilding the artifact.');
    }
  }

  /* [6] budget — on the committed bytes, because those are the bytes a TV
   * downloads. Reported unconditionally so the trend is visible in every run. */
  {
    const wasmBytes = committedWasm.length;
    const jsBytes = existsSync(join(committedDir, GLUE_FILE))
      ? statSync(join(committedDir, GLUE_FILE)).size : 0;
    const pct = (n, max) => `${((n / max) * 100).toFixed(1)}% of budget`;

    report('6 budget/wasm', wasmBytes <= maxWasmBytes,
      `${WASM_FILE} ${kib(wasmBytes)}, ceiling ${kib(maxWasmBytes)} — ${pct(wasmBytes, maxWasmBytes)}` +
      (wasmBytes <= maxWasmBytes ? '' :
        '. Either justify the growth and raise WASM_BUDGET_BYTES in ci.yml in its own ' +
        'reviewable commit, or find out what got linked in.'));

    report('6 budget/js', jsBytes <= maxJsBytes,
      `${GLUE_FILE} ${kib(jsBytes)}, ceiling ${kib(maxJsBytes)} — ${pct(jsBytes, maxJsBytes)}`);
  }
}

/* ---------------------------------------------------------------- summary --- */

if (process.env.GITHUB_STEP_SUMMARY) {
  const { appendFileSync } = await import('node:fs');
  appendFileSync(process.env.GITHUB_STEP_SUMMARY,
    `## Artifact freshness\n\n\`\`\`\n${lines.join('\n')}\n\`\`\`\n`);
}

if (failures.length > 0) {
  console.error(`\n${failures.length} gate(s) failed:\n${failures.map((f) => `  - ${f}`).join('\n')}`);
  process.exit(1);
}

console.log('\nAll artifact gates passed: the committed pkg/ is what this source builds.');
