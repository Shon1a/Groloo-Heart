/* THE GATE ON DELETING THE TYPESCRIPT.
 *
 *     node tests/parity/run.mjs            # summary
 *     node tests/parity/run.mjs --verbose  # every fixture, every note
 *     node tests/parity/run.mjs --prove    # prove the gate can go red
 *     node tests/parity/run.mjs --strict   # also fail on UNPLANNED divergences (CI)
 *
 * `tests/differential/run.mjs` compares the old vendored core against the new one.
 * Both sides of that comparison are Rust, which makes it structurally incapable of
 * saying anything about the five add-on-protocol functions: `addon.rs` shipped in
 * 0.1.0 with no binding of any kind, so they have no old-core counterpart and pass
 * that gate trivially. Its own coverage list says as much.
 *
 * This run is the other half. It compares the TYPESCRIPT THAT SHIPS against the
 * Rust that is meant to replace it — the real `addonClient.ts`, the real
 * `stores/addons.ts`, and the server's own `server.js` source text — and it exists
 * because of one rule:
 *
 *   THE TWIN RULE. A TypeScript implementation may only be deleted once a
 *   differential corpus proves the Rust replacement produces BYTE-IDENTICAL output
 *   on that corpus. No parity proof, no deletion.
 *
 * So the exit code is not the interesting output. The interesting output is THE
 * TWIN VERDICT at the end: which TypeScript this run authorises deleting, and which
 * it does not. A wrong `DELETE` there becomes a silent production regression, so a
 * twin carrying even one unplanned divergence is `BLOCKED` regardless of how many
 * of its fixtures matched, and a twin with no replacement at the boundary is
 * `NO TARGET` however green it is.
 *
 * Five outcomes fail a fixture:
 *
 *   UNEXPECTED  declared `match`, and the bytes differed. Either the port is wrong
 *               or the divergence was never declared. Stop.
 *   STALE       declared a divergence, and the bytes agreed. The justification
 *               written next to it is now false.
 *   SIGNL       the envelope did not carry the `ok`/`code`/`detail` the fixture
 *               promised. The VALUE can be identical and this still fails: half of
 *               what the core adds is the ability to say a call went wrong.
 *   WARN        a fixture said the core must warn, and it did not. Same reasoning.
 *   THREW       a call panicked, returned something unparseable, or a transcription
 *               pin found its source line changed.
 *
 * Byte identity, not deep equality: see `compare.mjs` for why this run is stricter
 * than the differential one about key order. */

import { FAMILIES, CANARY, UNCOMPARABLE } from './fixtures.mjs';
import { verdict, bytes } from './compare.mjs';
import {
  loadTwins, loadServerTwin, loadCore, SHIPPED, shellPinProblems, WEB, SERVER,
} from './twins.mjs';

const VERBOSE = process.argv.includes('--verbose');

const C = process.stdout.isTTY
  ? { r: '\x1b[31m', g: '\x1b[32m', y: '\x1b[33m', b: '\x1b[36m', d: '\x1b[2m', B: '\x1b[1m', x: '\x1b[0m' }
  : { r: '', g: '', y: '', b: '', d: '', B: '', x: '' };

const log = (s = '') => process.stdout.write(`${s}\n`);

/* ------------------------------------------------------------------------- */

const twins = await loadTwins();
twins.server = loadServerTwin();
const core = await loadCore();

const shippedWasm = SHIPPED.manifest.files['groloo_core_bg.wasm'];
const pinProblems = shellPinProblems();

log();
log(`${C.B}GROLOO parity — the TypeScript that ships vs the core that replaces it${C.x}`);
log(`${C.d}  core    ${SHIPPED.manifest.build}   ${core.core_version()}${C.x}`);
log(`${C.d}          sha256 ${shippedWasm.sha256}  (${shippedWasm.bytes} bytes)${C.x}`);
log(`${C.d}  client  ${WEB}\\src\\lib\\addonClient.ts        imported and run${C.x}`);
log(`${C.d}          ${WEB}\\src\\stores\\addons.ts          imported and run${C.x}`);
log(`${C.d}  server  ${SERVER}\\server\\server.js  two functions lifted as source text${C.x}`);
if (pinProblems.length) {
  log(`  ${C.r}${C.B}SHELL PIN${C.x} ${C.r}src/lib/heart.ts does not name this build — see the end of the run${C.x}`);
}
log();

/* ---- which copy of each twin this run actually compared ---- */

/* THE FIRST THING TO READ, and the reason this run's numbers can be compared with
 * another's at all. A twin that has already been deleted from the working tree is not
 * reachable there, and importing what replaced it would compare the core with its own
 * call site — a comparison that cannot fail, dressed as one that can. So `twins.mjs`
 * probes each binding, drives the copy that still implements the rule, and this table
 * says which copy that was. Anything marked REPLACED or DELETED is a parity claim about
 * code the browser no longer loads, which is exactly what a deletion review needs and
 * exactly what a "the shipping code matches" reading would get wrong. */
const replaced = twins.provenance.filter((p) => p.verdict !== 'IN TREE');
const PMARK = {
  'IN TREE': `${C.g}IN TREE ${C.x}`,
  REPLACED: `${C.y}REPLACED${C.x}`,
  DELETED: `${C.y}DELETED ${C.x}`,
};
log(`${C.B}WHICH COPY OF EACH TWIN WAS COMPARED${C.x}  ${C.d}working tree vs ${twins.rev} (the last revision that still had them)${C.x}`);
for (const p of twins.provenance) {
  log(`  ${PMARK[p.verdict]} ${p.at.padEnd(26)} ${C.d}${p.name}${C.x}`);
  if (p.verdict !== 'IN TREE' && VERBOSE) {
    log(`           ${C.d}on disk  ${p.curAnswer}${C.x}`);
    log(`           ${C.d}quoted   ${p.preAnswer}${C.x}`);
    for (const s of p.shellSaid) log(`           ${C.d}heart.ts says ${s.slice(0, 88)}${C.x}`);
  }
}
if (replaced.length) {
  log();
  log(`  ${C.y}${replaced.length} of ${twins.provenance.length} twins are no longer in the working tree.${C.x} ${C.d}They were quoted from ${twins.rev}`);
  log(`  ${C.d}and run from those bytes. Every verdict below about one of them is a statement about the${C.x}`);
  log(`  ${C.d}TypeScript AS IT WAS WHEN IT WAS DELETED — which is the only thing that can justify the${C.x}`);
  log(`  ${C.d}deletion — and NOT about the file the browser loads today.${C.x}`);
  log();
  /* The exception, and it is the one that keeps `stream_parse` honest: an entry point
   * can be a core call site while the rule underneath it is untouched. Those
   * declarations are re-read from both copies and compared before the run starts, so
   * this line is a check that passed and not a claim being made. */
  log(`  ${C.g}still on disk and byte-identical to ${twins.rev}${C.x}  ${C.d}${twins.kept.join(', ')}${C.x}`);
  log(`  ${C.d}— so the families driven through a quoted entry point still speak for the shipping rule.${C.x}`);
  if (!VERBOSE) log(`  ${C.d}(--verbose to see what each copy answered)${C.x}`);
}
log();

/* ------------------------------------------------------------------------- */

/** Push one fixture through both implementations and classify the outcome.
 * Extracted so `--prove` runs the canary through the EXACT path the real corpus
 * takes — a self-test that exercised a copy of the pipeline would prove nothing
 * about the pipeline. */
async function evaluate(family, f, divs) {
  const notes = [];
  let r;
  try {
    r = await family.run(twins, core, f);
  } catch (e) {
    return { status: 'threw', notes, error: e };
  }
  notes.push(...(r.notes || []));

  const res = verdict(f, r.tsOut, r.coreOut, notes);
  const where = `${family.name.split('  ')[0]}/${f.id}`;

  /* The envelope assertions. The old TypeScript answered `[]` or a guessed URL on
   * every failure path, so "the add-on sent nothing" and "the response was not
   * JSON" were the same observation — and the `catch { return [] }` at
   * `addonClient.ts:123` could not tell them apart either. Where a fixture says the
   * core must raise `ok:false`, that is asserted BY VALUE, and the degraded `data`
   * is still compared byte for byte (which is why `expect` stays 'match'): the
   * answer is unchanged, the silence is what went away. */
  if (f.expectOk !== undefined && r.env) {
    const err = r.env.error || {};
    if (r.env.ok !== f.expectOk) {
      res.status = 'signal-mismatch';
      res.signalWanted = `ok=${f.expectOk}`;
      res.signalGot = `ok=${r.env.ok}`;
    } else if (f.expectErrorCode && err.code !== f.expectErrorCode) {
      res.status = 'signal-mismatch';
      res.signalWanted = `error.code=${f.expectErrorCode}`;
      res.signalGot = `error.code=${err.code}`;
    } else if (f.expectErrorDetail && err.detail !== f.expectErrorDetail) {
      res.status = 'signal-mismatch';
      res.signalWanted = `error.detail=${bytes(f.expectErrorDetail)}`;
      res.signalGot = `error.detail=${bytes(err.detail)}`;
    } else if (f.expectOk === false) {
      note(divs, 'N-loud', WHY_LOUD, where);
    }
  }

  if (f.expectWarning && r.env) {
    if (!(r.env.codes || []).includes(f.expectWarning)) {
      res.status = 'warning-mismatch';
      res.signalWanted = `a ${f.expectWarning} warning`;
      res.signalGot = `warnings [${(r.env.codes || []).join(', ')}]`;
    } else {
      note(divs, 'N-warn', WHY_WARN, where);
    }
  }

  if (res.divergence && res.status === 'diverged') note(divs, res.divergence.d, res.divergence.why, where);
  return res;
}

const WHY_LOUD = 'The same degraded value the TypeScript returned, now with `ok:false` and an error code attached. `addonClient.ts:123` and `:163` are two of the four blind `catch { return [] }` blocks this phase exists to close: an add-on that returned HTML, an add-on that 500ed and an add-on that genuinely has no sources for a title were one observation, and the UI said "no sources" for all three. `data` is unchanged, so nothing has to be rewritten to adopt it.';

const WHY_WARN = 'A record the core could not read is reported rather than silently absent. The TypeScript has no concept of a warning, so "the add-on sent 9 sources and you can see 7" was unanswerable from anywhere in the app.';

/* ---- --prove ---- */

if (process.argv.includes('--prove')) {
  log(`${C.B}--prove: ${CANARY.fixtures.length} real runs, each of which MUST be caught${C.x}`);
  log(`${C.d}  four are real comparisons with deliberately wrong expectations; the rest fire the two${C.x}`);
  log(`${C.d}  guards that keep this corpus honest — the transcription pin and the circular-comparison${C.x}`);
  log(`${C.d}  guard on PRE_PORT_REV, the latter in both directions.${C.x}`);
  let broken = 0;
  for (const f of CANARY.fixtures) {
    const res = await evaluate(CANARY, f, new Map());
    const good = res.status === f.mustClassifyAs;
    if (!good) broken += 1;
    log(`  ${good ? `${C.g}  ok  ${C.x}` : `${C.r}BROKEN${C.x}`} ${f.id}`);
    log(`         ${C.d}must classify as ${f.mustClassifyAs}; classified as ${res.status}${C.x}`);
  }
  log();
  if (broken) {
    log(`${C.r}${C.B}THE HARNESS IS BROKEN${C.x} — ${broken} canary/canaries were not caught.`);
    log(`${C.r}Nothing this harness prints about the real corpus can be trusted, and no twin may be deleted on it.${C.x}\n`);
    process.exit(1);
  }
  log(`${C.g}${C.B}The gate has teeth${C.x} — every deliberately wrong expectation was caught.\n`);
  process.exit(0);
}

const divergences = new Map();
const results = [];
for (const family of FAMILIES) {
  const rows = [];
  for (const f of family.fixtures) rows.push({ f, res: await evaluate(family, f, divergences) });
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
  'signal-mismatch': `${C.r}SIGNL!${C.x}`,
  'warning-mismatch': `${C.r}WARN !${C.x}`,
  threw: `${C.r}THREW ${C.x}`,
};

const BAD = ['unexpected', 'stale-expectation', 'signal-mismatch', 'warning-mismatch', 'threw'];
const tally = { match: 0, diverged: 0, unexpected: 0, 'stale-expectation': 0, 'signal-mismatch': 0, 'warning-mismatch': 0, threw: 0 };

for (const { family, rows } of results) {
  log(`${C.B}${family.name}${C.x}  ${C.d}${rows.length} fixtures${C.x}`);
  log(`${C.d}  ts:   ${family.ts}${C.x}`);
  log(`${C.d}  rust: ${family.rust}${C.x}`);
  for (const { f, res } of rows) {
    tally[res.status] += 1;
    const bad = BAD.includes(res.status);
    if (VERBOSE || bad || res.status === 'diverged') {
      log(`  ${MARK[res.status]} ${f.id}${res.divergence ? `  ${C.d}[${res.divergence.d}]${C.x}` : ''}`);
    }
    if (res.status === 'threw') log(`        ${C.r}${String(res.error.message).split('\n').join('\n        ')}${C.x}`);
    // An agreement is worth reading, not just counting: two empty arrays agree too.
    if (VERBOSE && res.status === 'match') log(`        ${C.d}both ${res.ts}${C.x}`);
    if (res.diff && (bad || VERBOSE)) log(`        ${C.d}${res.diff}${C.x}`);
    if (res.status === 'stale-expectation') log(`        ${C.r}declared ${res.divergence.d} but the two agreed — the expectation is stale${C.x}`);
    if (res.signalWanted) log(`        ${C.r}wanted ${res.signalWanted}; got ${res.signalGot}${C.x}`);
    if (VERBOSE) for (const n of res.notes) log(`        ${C.d}${n}${C.x}`);
  }
  log();
}

/* ---- the deliberate differences, collected ---- */

const unplanned = [...divergences].filter(([id]) => id.startsWith('U')).sort();
const deliberate = [...divergences].filter(([id]) => !id.startsWith('U')).sort();

if (unplanned.length) {
  log(`${C.r}${C.B}!! UNPLANNED DIVERGENCES — DECIDE BEFORE DELETING THE TWIN !!${C.x}`);
  log(`${C.d}   Not in this increment's contract. Each is a behaviour change nobody signed off,${C.x}`);
  log(`${C.d}   and each one blocks its family's twin from being deleted.${C.x}`);
  for (const [id, d] of unplanned) {
    log(`  ${C.r}${C.B}${id}${C.x}  ${C.d}${d.hits.join(', ')}${C.x}`);
    log(`      ${wrap(d.why, 6)}`);
  }
  log();
}

log(`${C.B}DELIBERATE DIFFERENCES${C.x}  ${C.d}(each asserted by an exact-bytes fixture, not excused by one)${C.x}`);
if (!deliberate.length) log(`  ${C.d}(none observed)${C.x}`);
for (const [id, d] of deliberate) {
  log(`  ${C.y}${id}${C.x}  ${C.d}${d.hits.join(', ')}${C.x}`);
  log(`      ${wrap(d.why, 6)}`);
}
log();

log(`${C.B}NOT COMPARED (no counterpart on one side)${C.x}`);
for (const u of UNCOMPARABLE) {
  log(`  ${C.d}${u.side.padEnd(24)}${C.x} ${u.fn}`);
  if (VERBOSE) log(`      ${wrap(u.why, 6)}`);
}
if (!VERBOSE) log(`  ${C.d}(--verbose for why each one has no counterpart)${C.x}`);
log();

/* ---- the verdict that matters: which twins may be deleted ---- */

/* A twin is DELETABLE when every fixture in its family either matched or diverged
 * exactly as declared, AND none of those divergences is unplanned. The second
 * condition is the one that does the work: a `U`-prefixed divergence means the two
 * implementations disagree in a way nobody chose, and deleting the TypeScript would
 * ship that disagreement to a television with no way back. "Mostly matched" is not
 * a state this list has.
 *
 * Parity is NECESSARY and not always SUFFICIENT, which is why each family also
 * declares what kind of twin it has. Three of them are not delete candidates at all
 * however green they are — two live in the server, which runs no wasm, and one has
 * no boundary function to be replaced by — and printing them as "safe to delete"
 * would be the most expensive kind of wrong this file can be. */
const LABEL = {
  delete: [`${C.g}DELETE   `, 'proven byte-identical — the twin may go'],
  keep: [`${C.b}KEEP     `, 'not a delete candidate: the server runs no wasm. Parity here means the two agree.'],
  rewrite: [`${C.y}REWRITE  `, 'parity proven, but the response SHAPE changed — deleting this is a call-site rewrite'],
  unreachable: [`${C.y}NO TARGET`, 'parity proven, but nothing at the boundary replaces it — the twin stays'],
};

/** What a failure means depends on whether the twin was ever going to be deleted.
 * A `keep` copy that disagrees is not "blocked from deletion" — it is a server and
 * a core that answer differently, which is its own problem and a worse-sounding one
 * than the word BLOCKED would suggest. */
const FAILED_LABEL = {
  delete: [`${C.r}BLOCKED  `, 'decide it before deleting'],
  keep: [`${C.r}DISAGREE `, 'the core and this copy do not agree — unify them or record the difference'],
  rewrite: [`${C.r}BLOCKED  `, 'decide it before rewriting the call sites'],
  unreachable: [`${C.r}BLOCKED  `, 'decide it'],
};

log(`${C.B}THE TWIN VERDICT — what this run does and does not authorise${C.x}`);
log(`${C.d}  The twin rule: no TypeScript is deleted without byte-identical proof on this corpus.${C.x}`);
log(`${C.d}  Parity is necessary, not always sufficient — read the second line of each entry.${C.x}`);
log();
const verdicts = [];
for (const { family, rows } of results) {
  const failed = rows.filter((r) => BAD.includes(r.res.status));
  const unplannedHere = rows.filter((r) => r.res.divergence && r.res.divergence.d.startsWith('U') && r.res.status === 'diverged');
  const declared = rows.filter((r) => r.res.status === 'diverged').length;
  const proven = failed.length === 0 && unplannedHere.length === 0;
  const twin = family.twin || { at: family.name, verdict: 'delete' };
  const [mark, gloss] = LABEL[twin.verdict];
  const blocked = !proven;
  verdicts.push({ family, twin, proven, blocked, failed: failed.length, unplanned: unplannedHere.length, declared, total: rows.length });

  const [badMark, badGloss] = FAILED_LABEL[twin.verdict];
  log(`  ${blocked ? `${badMark}${C.x}` : `${mark}${C.x}`} ${twin.at}`);
  log(`           ${C.d}${rows.length - failed.length - declared} matched, ${declared} declared divergence(s), ${failed.length} failure(s), ${unplannedHere.length} unplanned${C.x}`);
  if (blocked) {
    for (const r of failed) log(`           ${C.r}${r.f.id}: ${r.res.status}${C.x}`);
    for (const r of unplannedHere) log(`           ${C.r}${r.f.id}: unplanned divergence ${r.res.divergence.d} — ${badGloss}${C.x}`);
  } else {
    log(`           ${C.d}${gloss}${C.x}`);
    /* "Proven byte-identical" is true of a family with no declared divergences and
     * OVERSTATED of one that has them: `catalog_metas` matches 21 fixtures and
     * deliberately answers differently on three, and a reader who takes the one-line
     * gloss at face value ships D-kind and D-lenient without knowing they exist. The
     * gate is unchanged — a declared divergence has always been allowed to pass — but
     * the sentence next to it now says what passing means. */
    const changes = [...new Set(rows.filter((r) => r.res.status === 'diverged').map((r) => r.res.divergence.d))];
    if (changes.length) {
      const carrier = twin.verdict === 'keep' ? 'the two copies already differ here' : 'these ship with the swap';
      log(`           ${C.y}NOT identical — ${changes.join(', ')}; ${carrier} (each argued above)${C.x}`);
    }
  }
  /* Which copy this family's verdict is a statement ABOUT. A DELETE on a twin that is
   * already gone is a review of a deletion that has happened; on one still on disk it
   * is permission for a deletion that has not. Those are different sentences and the
   * line that authorises the work has to say which.
   *
   * Matched on `family.ts` — the family's own one-line description of what it drives,
   * which names the function — rather than on `twin.at`, whose line numbers are the
   * twin's and drift from the entry point's. */
  const quoted = twins.provenance.filter((p) => p.driven.test(family.ts) && p.verdict !== 'IN TREE');
  if (quoted.length) {
    log(`           ${C.y}entry point is a core call site now — driven from ${twins.rev}'s ${quoted.map((p) => p.name).join(', ')}${C.x}`);
  }
  if (twin.obligation) log(`           ${C.y}obligation: ${wrap(twin.obligation, 23)}${C.x}`);
}
log();

/* ---- summary ---- */

const total = Object.values(tally).reduce((a, b) => a + b, 0);
const failed = BAD.reduce((n, k) => n + tally[k], 0);

log(`${C.B}SUMMARY${C.x}`);
log(`  fixtures                ${total}`);
log(`  ${C.g}matched (byte for byte)${C.x} ${tally.match}`);
log(`  ${C.y}diverged as declared${C.x}    ${tally.diverged}`);
log(`  ${tally.unexpected ? C.r : C.d}unexpected differences${C.x}  ${tally.unexpected}`);
log(`  ${tally['stale-expectation'] ? C.r : C.d}stale expectations${C.x}      ${tally['stale-expectation']}`);
log(`  ${tally['signal-mismatch'] ? C.r : C.d}envelope mismatches${C.x}     ${tally['signal-mismatch']}`);
log(`  ${tally['warning-mismatch'] ? C.r : C.d}warning mismatches${C.x}      ${tally['warning-mismatch']}`);
log(`  ${tally.threw ? C.r : C.d}threw${C.x}                   ${tally.threw}`);
log(`  ${unplanned.length ? C.r : C.d}unplanned divergences${C.x}   ${unplanned.length}${unplanned.length ? `  ${C.r}<- needs a decision${C.x}` : ''}`);
/* In the SUMMARY because the four numbers above it mean something different depending
 * on this one: 152 matched against the file the browser loads is a statement about what
 * ships, and 152 matched against a git blob is a statement about what was deleted. Both
 * are worth having; conflating them is how one deletion gets authorised twice. */
log(`  ${replaced.length ? C.y : C.d}twins quoted from git${C.x}   ${replaced.length} of ${twins.provenance.length}${replaced.length ? `  ${C.y}<- already deleted from the working tree${C.x}` : ''}`);
log(`  ${C.d}twins proven identical${C.x}  ${verdicts.filter((v) => v.proven).length} of ${verdicts.length}`);
log(`  ${C.d}twins clear to delete${C.x}   ${verdicts.filter((v) => v.proven && v.twin.verdict === 'delete').length} of ${verdicts.filter((v) => v.twin.verdict === 'delete').length}`);
log(`  ${pinProblems.length ? C.r : C.d}shell pin${C.x}               ${pinProblems.length ? `${C.r}STALE  <- heart.ts names another build${C.x}` : 'agrees'}`);
log();

if (pinProblems.length) {
  log(`${C.r}${C.B}!! THE SHELL DOES NOT POINT AT THE ARTIFACT THIS RUN GATED !!${C.x}`);
  for (const p of pinProblems) log(`  ${C.r}${p}${C.x}`);
  log();
  log(`${C.d}  Everything above is a statement about ${SHIPPED.manifest.build}. The browser loads whatever${C.x}`);
  log(`${C.d}  src/lib/heart.ts names, so until the two agree this run gated an artifact the app${C.x}`);
  log(`${C.d}  does not fetch — and the five new functions are not in its CoreExports interface either.${C.x}`);
  log();
  log(`    const CORE_BUILD = '${SHIPPED.manifest.build}';`);
  log(`    const CORE_WASM_SHA256 = '${shippedWasm.sha256}';`);
  log();
}

if (failed) {
  log(`${C.r}${C.B}GATE CLOSED${C.x} — ${failed} fixture(s) did not behave as this corpus says they should.`);
  log(`${C.r}Delete no TypeScript on the strength of this run.${C.x}`);
  log();
  process.exit(1);
}
if (unplanned.length || pinProblems.length) {
  const reservations = [
    unplanned.length ? `${unplanned.length} divergence(s) this increment did not plan` : null,
    pinProblems.length ? 'a shell pin that names a different build' : null,
  ].filter(Boolean);
  log(`${C.y}${C.B}GATE OPEN, WITH RESERVATIONS${C.x} — every fixture behaved as declared, but there is`);
  log(`${reservations.join(', and ')}. The twin verdict above is what to act on:`);
  log('a twin carrying an unplanned divergence is BLOCKED, whatever its match count, and only');
  log('the lines marked DELETE authorise deleting anything.');
  log(`${C.d}(--strict makes this exit non-zero, which is what CI should run.)${C.x}`);
  log();
  process.exit(process.argv.includes('--strict') ? 1 : 0);
}
log(`${C.g}${C.B}GATE OPEN${C.x} — every fixture either matched the TypeScript byte for byte or diverged`);
log('exactly as declared. Every twin above marked SAFE may be deleted.');
log();

/** Record one observation of a divergence. Collected across the whole run so the
 * report names each difference ONCE with every fixture that exhibits it. */
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
