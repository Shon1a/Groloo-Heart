/* The gate. Run it before anything in Stredio-Web is repointed:
 *
 *     node tests/differential/run.mjs            # summary
 *     node tests/differential/run.mjs --verbose  # every fixture, every note
 *     node tests/differential/run.mjs --prove    # prove the gate can go red
 *     node tests/differential/run.mjs --strict   # also fail on UNPLANNED divergences (CI)
 *
 * Exit 0 means every fixture either agreed with the live 0.1.0-5ecaa78 core or
 * disagreed in exactly the way this phase says it should. Exit 1 means it did not,
 * and the swap is not safe yet.
 *
 * BOTH CORES ARE VENDORED ARTIFACTS. The old one is the wasm serving the app
 * today; the new one is the wasm in `public/assets/groloo-core/<build>/`, hashed
 * against its own manifest and against a fresh build of this tree before a single
 * fixture runs (see cores.mjs). The harness used to build its own copy, which
 * differed from the shipped one by 176 bytes — a gate that validates a build
 * nobody deployed is not validating the deployment, and that is the whole reason
 * this phase exists.
 *
 * Five outcomes fail a run, and only the first is the obvious one. The other four
 * are why this file exists rather than a pile of `assert.deepEqual`s — each closes
 * a way a differential harness can be green and worthless:
 *
 *   UNEXPECTED  a fixture declared `match` and the two cores disagreed. The new
 *               core changes behaviour the app depends on. Stop.
 *   STALE       a fixture declared a divergence and the two cores AGREED. The
 *               divergence has quietly stopped happening, so the document
 *               describing this phase is now wrong about it.
 *   LANG        a fixture`s `Progress.lang` was not the value it promised.
 *               `lang` is lifted OUT of the structural comparison (nothing on the
 *               old side could ever equal it), so without this check it would be
 *               the one field the harness pretends to cover and does not.
 *   SIGNL       a fixture said the new core must raise `ok:false` on input the old
 *               one swallowed, and it did not. The degraded VALUE is unchanged on
 *               these — what is being asserted is that the silence is gone.
 *   THREW       a call panicked or returned something unparseable. On
 *               wasm32-unknown-unknown a panic traps the instance permanently, so
 *               there is no such thing as a recoverable one.
 *
 * Warnings are reported and never compared: they are a pure addition, and
 * comparing them against a core with no concept of them would be theatre. */

import { loadOld, loadNew, OLD_DIR, SHIPPED, shellPinProblems } from './cores.mjs';
import { FAMILIES, UNCOMPARABLE, CANARY } from './fixtures.mjs';
import { unwrap, stripMyList, stripLang, verdict, deepEqual } from './compare.mjs';

const VERBOSE = process.argv.includes('--verbose');

const C = process.stdout.isTTY
  ? { r: '\x1b[31m', g: '\x1b[32m', y: '\x1b[33m', b: '\x1b[36m', d: '\x1b[2m', B: '\x1b[1m', x: '\x1b[0m' }
  : { r: '', g: '', y: '', b: '', d: '', B: '', x: '' };

const log = (s = '') => process.stdout.write(`${s}\n`);

/* ------------------------------------------------------------------------- */

const old = await loadOld();
/* An ES module namespace object is frozen, so the envelope helper is hung off a
 * plain copy rather than off the namespace itself. The fixtures reach it as
 * `neu.__unwrap` and never have to import the comparison module. */
const neu = { ...(await loadNew()), __unwrap: unwrap };

const coreVersion = neu.core_version();

/* Both sides are vendored artifacts read out of Stredio-Web/public/assets — the
 * bytes that serve the app, not a tree that compiles to something similar. The
 * sha-256 is printed because it is the only line of this header that cannot be
 * true of the wrong file: `0.2.0+gunknown` is what every build of this repo
 * answers, and a folder name is a claim anyone can write. */
const shippedWasm = SHIPPED.manifest.files['groloo_core_bg.wasm'];
const pinProblems = shellPinProblems();

log();
log(`${C.B}GROLOO differential — new core vs the live vendored artifact${C.x}`);
log(`${C.d}  old  0.1.0-5ecaa78   ${OLD_DIR}${C.x}`);
log(`${C.d}  new  ${SHIPPED.manifest.build}   ${SHIPPED.dir}${C.x}`);
log(`${C.d}       ${coreVersion}   sha256 ${shippedWasm.sha256}  (${shippedWasm.bytes} bytes)${C.x}`);
log(`${C.d}       verified against its manifest, and rebuilt from source to the same bytes${C.x}`);
if (pinProblems.length) {
  log(`  ${C.r}${C.B}SHELL PIN${C.x} ${C.r}src/lib/heart.ts does not name this build — see the end of the run${C.x}`);
} else {
  log(`${C.d}       src/lib/heart.ts pins this build and this digest${C.x}`);
}
log();

const results = [];
const divergences = new Map();

/** Push one fixture through both cores and classify the outcome. Extracted so
 * `--prove` can run the canary through the EXACT code path the real corpus takes
 * — a self-test that exercised a copy of the pipeline would prove nothing about
 * the pipeline. */
function evaluate(family, f, divs) {
  const notes = [];
  let r;
  try {
    r = family.run(old, neu, f);
  } catch (e) {
    return { status: 'threw', notes, error: e };
  }
  notes.push(...(r.notes || []));

  let oldOut = r.oldOut;
  let newOut = r.newOut;
  let langs = {};

  // Declared, reported normalisations. Only the library families produce a
  // `Library`, and only there do the two extra fields and the extra key exist.
  if (r.library) {
    newOut = stripMyList(newOut, notes);
    const s = stripLang(newOut, notes);
    newOut = s.lib;
    langs = s.langs;
  }

  const res = verdict(f, oldOut, newOut, notes);
  const where = `${family.name}/${f.id}`;

  /* D4 is asserted here rather than structurally, because `lang` cannot exist on
   * the old side at all — there is nothing for it to be equal to. Lifting it out
   * of the comparison and checking it BY VALUE is what keeps it tested rather
   * than merely excluded. A fixture that declares a language and does not get it
   * fails the run even when every other byte matches. */
  if (f.expectLang) {
    if (!deepEqual(langs, f.expectLang)) {
      res.status = 'lang-mismatch';
      res.langWanted = f.expectLang;
      res.langGot = langs;
    } else {
      note(divs, 'D4', WHY_D4, where);
    }
  }

  /* The other half of problem 7: 0.1.0 returned a success-shaped value on every
   * failure path, so "the core is broken" and "the core has nothing to say" were
   * the same observation — and the four `catch { return null }` call sites in
   * Stredio-Web could not tell them apart either. Where a fixture says the new
   * core must raise `ok:false` on input the old one swallowed, that is asserted.
   * The degraded VALUE is unchanged (which is why `expect` stays 'match'); the
   * silence is what went away. */
  if (f.expectOk !== undefined && r.env) {
    const code = (r.env.error || {}).code;
    if (r.env.ok !== f.expectOk) {
      res.status = 'signal-mismatch';
      res.signalWanted = `ok=${f.expectOk}`;
      res.signalGot = `ok=${r.env.ok}`;
    } else if (f.expectErrorCode && code !== f.expectErrorCode) {
      res.status = 'signal-mismatch';
      res.signalWanted = `error.code=${f.expectErrorCode}`;
      res.signalGot = `error.code=${code}`;
    } else if (f.expectOk === false) {
      note(divs, 'N-loud', WHY_LOUD, where);
    }
  }

  if (res.divergence && res.status === 'diverged') {
    note(divs, res.divergence.d, res.divergence.why, where);
  }
  return res;
}

const WHY_D4 = 'Progress.lang is new, and `merge_progress` carries a language forward from the LOSING record when the winner has none. 0.1.0 had nowhere to put it, so `history.ts::keepLangs()` re-attached it after every core call — and being called from only two of the four write paths, it silently discarded the saved audio track on the other two. `pos`/`dur`/`at` are still compared byte for byte; only `lang` is asserted separately, by value.';

const WHY_LOUD = 'The same degraded value 0.1.0 returned, now with `ok:false` and an error code attached. 0.1.0 could not distinguish "nothing to show" from "broken", and neither could the four `catch { return null }` call sites reading it — so this has been failing invisibly in production. `data` is unchanged, so nothing has to be rewritten to adopt it.';

/* ---- --prove: does the classifier actually catch anything? ---- */

if (process.argv.includes('--prove')) {
  log(`${C.B}--prove: four real comparisons with deliberately wrong expectations${C.x}`);
  let broken = 0;
  for (const f of CANARY.fixtures) {
    const res = evaluate(CANARY, f, new Map());
    const good = res.status === f.mustClassifyAs;
    if (!good) broken += 1;
    log(`  ${good ? `${C.g}  ok  ${C.x}` : `${C.r}BROKEN${C.x}`} ${f.id}`);
    log(`         ${C.d}must classify as ${f.mustClassifyAs}; classified as ${res.status}${C.x}`);
  }
  log();
  if (broken) {
    log(`${C.r}${C.B}THE HARNESS IS BROKEN${C.x} — ${broken} canary/canaries were not caught.`);
    log(`${C.r}Nothing this harness prints about the real corpus can be trusted.${C.x}\n`);
    process.exit(1);
  }
  log(`${C.g}${C.B}The gate has teeth${C.x} — every deliberately wrong expectation was caught.\n`);
  process.exit(0);
}

for (const family of FAMILIES) {
  const rows = family.fixtures.map((f) => ({ f, res: evaluate(family, f, divergences) }));
  results.push({ family, rows });
}

/* ------------------------------------------------------------------------- */
/* report                                                                     */
/* ------------------------------------------------------------------------- */

const MARK = {
  match: `${C.g}  ok  ${C.x}`,
  diverged: `${C.y} diff ${C.x}`,
  unexpected: `${C.r}FAIL !${C.x}`,
  'stale-expectation': `${C.r}STALE ${C.x}`,
  'lang-mismatch': `${C.r}LANG !${C.x}`,
  'signal-mismatch': `${C.r}SIGNL!${C.x}`,
  threw: `${C.r}THREW ${C.x}`,
};

const BAD = ['unexpected', 'stale-expectation', 'lang-mismatch', 'signal-mismatch', 'threw'];
const tally = { match: 0, diverged: 0, unexpected: 0, 'stale-expectation': 0, 'lang-mismatch': 0, 'signal-mismatch': 0, threw: 0 };

for (const { family, rows } of results) {
  log(`${C.B}${family.name}${C.x}  ${C.d}${rows.length} fixtures${C.x}`);
  log(`${C.d}  old: ${family.old}${C.x}`);
  log(`${C.d}  new: ${family.neu}${C.x}`);
  for (const { f, res } of rows) {
    tally[res.status] += 1;
    const bad = BAD.includes(res.status);
    if (VERBOSE || bad || res.status === 'diverged') {
      log(`  ${MARK[res.status]} ${f.id}${res.divergence ? `  ${C.d}[${res.divergence.d}]${C.x}` : ''}`);
    }
    if (res.status === 'threw') log(`        ${C.r}${res.error.stack.split('\n').slice(0, 3).join('\n        ')}${C.x}`);
    if (res.diff && (bad || VERBOSE)) log(`        ${C.d}${res.diff}${C.x}`);
    if (res.status === 'stale-expectation') log(`        ${C.r}declared ${res.divergence.d} but the two cores agreed — the expectation is stale${C.x}`);
    if (res.status === 'lang-mismatch') log(`        ${C.r}lang wanted ${JSON.stringify(res.langWanted)} got ${JSON.stringify(res.langGot)}${C.x}`);
    if (res.status === 'signal-mismatch') log(`        ${C.r}envelope wanted ${res.signalWanted} got ${res.signalGot}${C.x}`);
    if (VERBOSE) for (const n of res.notes) log(`        ${C.d}${n}${C.x}`);
  }
  log();
}

/* ---- the deliberate differences, collected ---- */

/* A `U`-prefixed id is a difference this phase did NOT plan for. It is declared
 * so the run can be read at a glance, not so it can be forgotten — a divergence
 * nobody chose is a behaviour change nobody reviewed, and burying it in the same
 * list as the intended ones is exactly how it would get shipped. It gets its own
 * section, above the deliberate ones, and its own line in the summary. */
const unplanned = [...divergences].filter(([id]) => id.startsWith('U')).sort();
const deliberate = [...divergences].filter(([id]) => !id.startsWith('U')).sort();

if (unplanned.length) {
  log(`${C.r}${C.B}!! UNPLANNED DIVERGENCES — DECIDE BEFORE SWAPPING !!${C.x}`);
  log(`${C.d}   Not in this phase's contract. Each is a behaviour change nobody signed off.${C.x}`);
  for (const [id, d] of unplanned) {
    log(`  ${C.r}${C.B}${id}${C.x}  ${C.d}${d.hits.join(', ')}${C.x}`);
    log(`      ${wrap(d.why, 6)}`);
  }
  log();
}

log(`${C.B}DELIBERATE DIFFERENCES${C.x}  ${C.d}(the register, with the reasoning: tests/differential/DIVERGENCES.md)${C.x}`);
if (!deliberate.length) log(`  ${C.d}(none observed)${C.x}`);
for (const [id, d] of deliberate) {
  log(`  ${C.y}${id}${C.x}  ${C.d}${d.hits.join(', ')}${C.x}`);
  log(`      ${wrap(d.why, 6)}`);
}
log();

/* Always-on differences that are not per-fixture: they are true of every single
 * call, so listing them once is more honest than tagging 90 fixtures with them. */
log(`${C.B}ALWAYS-ON DIFFERENCES (true of every call, not fixture-specific)${C.x}`);
for (const [id, text] of [
  ['N-envelope', 'Every new function except core_version answers {ok, core, data, warnings, error} instead of a bare value. `data` is always the graceful-degradation value, so a shell that destructures `data` and ignores `ok` behaves bit-identically to 0.1.0 — which is exactly what makes the envelope adoptable in one step and why this harness compares `data`.'],
  ['N-core', `Every envelope carries core:"${coreVersion}". 0.1.0 could not report its own identity at all, which is why a mispointed vendored folder presented as "the app behaves like an old build" rather than as an error.`],
  ['N-warnings', 'Truncation, tombstone expiry, rejected CDN records and dropped icons are now reported as warnings. 0.1.0 did all of these silently — a user syncing a 90-entry history saw 60 and had no way to learn the other 30 were capped rather than lost.'],
]) {
  log(`  ${C.y}${id}${C.x}`);
  log(`      ${wrap(text, 6)}`);
}
log();

/* ---- coverage the harness cannot provide ---- */

log(`${C.B}NOT COMPARED (no counterpart on one side)${C.x}`);
for (const u of UNCOMPARABLE) {
  log(`  ${C.d}${u.side.padEnd(9)}${C.x} ${u.fn}`);
  if (VERBOSE) log(`      ${wrap(u.why, 6)}`);
}
if (!VERBOSE) log(`  ${C.d}(--verbose for why each one has no counterpart)${C.x}`);
log();

/* ---- verdict ---- */

const total = Object.values(tally).reduce((a, b) => a + b, 0);
const failed = BAD.reduce((n, k) => n + tally[k], 0);

log(`${C.B}SUMMARY${C.x}`);
log(`  fixtures                ${total}`);
log(`  ${C.g}matched${C.x}                 ${tally.match}`);
log(`  ${C.y}diverged as declared${C.x}    ${tally.diverged}`);
log(`  ${tally.unexpected ? C.r : C.d}unexpected differences${C.x}  ${tally.unexpected}`);
log(`  ${tally['stale-expectation'] ? C.r : C.d}stale expectations${C.x}      ${tally['stale-expectation']}`);
log(`  ${tally['lang-mismatch'] ? C.r : C.d}lang mismatches${C.x}         ${tally['lang-mismatch']}`);
log(`  ${tally['signal-mismatch'] ? C.r : C.d}envelope mismatches${C.x}     ${tally['signal-mismatch']}`);
log(`  ${tally.threw ? C.r : C.d}threw${C.x}                   ${tally.threw}`);
log(`  ${unplanned.length ? C.r : C.d}unplanned divergences${C.x}   ${unplanned.length}${unplanned.length ? `  ${C.r}<- needs a decision${C.x}` : ''}`);
log(`  ${pinProblems.length ? C.r : C.d}shell pin${C.x}               ${pinProblems.length ? `${C.r}STALE  <- heart.ts names another build${C.x}` : 'agrees'}`);
log();

/* The shell-pin cross-check. It is not a fixture and it cannot be: no comparison
 * between two cores can notice that the app is configured to load a THIRD one. It
 * sits here, in the verdict, because "which artifact does the browser actually
 * fetch" is the last question standing between a green gate and a working deploy —
 * and the answer lives in a file this repository does not own, which is exactly
 * why it has to be asserted rather than assumed. */
if (pinProblems.length) {
  log(`${C.r}${C.B}!! THE SHELL DOES NOT POINT AT THE ARTIFACT THIS RUN GATED !!${C.x}`);
  for (const p of pinProblems) log(`  ${C.r}${p}${C.x}`);
  log();
  log(`${C.d}  Everything above is a statement about ${SHIPPED.manifest.build}. The browser loads whatever${C.x}`);
  log(`${C.d}  src/lib/heart.ts names, so until the two agree this run gated an artifact the app${C.x}`);
  log(`${C.d}  does not fetch. Two lines, in Stredio-Web/src/lib/heart.ts:${C.x}`);
  log();
  log(`    const CORE_BUILD = '${SHIPPED.manifest.build}';`);
  log(`    const CORE_WASM_SHA256 = '${shippedWasm.sha256}';`);
  log();
}

if (failed) {
  log(`${C.r}${C.B}GATE CLOSED${C.x} — ${failed} fixture(s) did not behave as this phase says they should.`);
  log(`${C.r}Do not repoint Stredio-Web at the new core.${C.x}`);
  log();
  process.exit(1);
}
if (unplanned.length || pinProblems.length) {
  /* `--strict` is what CI should run. Interactively the run still exits 0,
   * because both of these are questions for a person and not a broken build — an
   * unplanned divergence needs a decision, and a stale pin needs two lines pasted
   * into a repository this one does not own. Neither is ever allowed to be quiet. */
  const reservations = [
    unplanned.length ? `${unplanned.length} divergence(s) this phase did not plan` : null,
    pinProblems.length ? 'a shell pin that names a different build' : null,
  ].filter(Boolean);
  log(`${C.y}${C.B}GATE OPEN, WITH RESERVATIONS${C.x} — every fixture behaved as declared, but there is`);
  log(`${reservations.join(', and ')}. See above; neither is settled by this run.`);
  log(`${C.d}(--strict makes this exit non-zero, which is what CI should run.)${C.x}`);
  log();
  process.exit(process.argv.includes('--strict') ? 1 : 0);
}
log(`${C.g}${C.B}GATE OPEN${C.x} — every fixture either matched 0.1.0-5ecaa78 or diverged exactly as declared,`);
log(`and the artifact gated is the one src/lib/heart.ts loads.`);
log();

/** Record one observation of a divergence. Collected across the whole run so the
 * report names each difference ONCE with every fixture that exhibits it, rather
 * than repeating the same paragraph nine times. */
function note(map, id, why, hit) {
  if (!map.has(id)) map.set(id, { why, hits: [] });
  map.get(id).hits.push(hit);
}

function wrap(s, indent) {
  const width = 96 - indent;
  const pad = ' '.repeat(indent);
  const out = [];
  let line = '';
  for (const word of s.split(' ')) {
    if (line.length + word.length + 1 > width) { out.push(line); line = word; } else { line = line ? `${line} ${word}` : word; }
  }
  if (line) out.push(line);
  return out.join(`\n${pad}`);
}
