/* Deciding whether the TypeScript and the Rust produced the SAME BYTES — and, when
 * they did not, whether that was the plan.
 *
 * ## Why bytes and not `deepEqual`
 *
 * `tests/differential/compare.mjs` compares parsed values with key order ignored,
 * and that is right for what it does: both sides there are `serde_json` output, so
 * key order carries no information and failing on it would fail for a reason nobody
 * cares about.
 *
 * Here it would hide the thing the twin rule is about. The rule is that a
 * TypeScript implementation may be deleted once the Rust replacement produces
 * BYTE-IDENTICAL output on the corpus — and the shell round-trips several of these
 * values through `JSON.stringify` into `localStorage` and back. A record whose keys
 * come out in a different order is a different string in storage, a different cache
 * key, and a different diff for the next person reading a bug report. The Rust
 * structs were written with this in mind (`stream.rs`: "Field ORDER is part of the
 * contract, not cosmetics"), so byte identity is a claim the core already makes and
 * this is where it gets checked.
 *
 * So: the verdict is `JSON.stringify(ts) === JSON.stringify(rust)`, and when the
 * bytes differ the diff says whether the VALUES also differ or only their spelling.
 * Both are failures against a fixture that declared `match`; the message is what
 * changes, because "same value, different key order" and "different value" send the
 * next reader to two very different places.
 *
 * ## Nothing is ever loosened
 *
 * Every fixture declares up front whether it expects agreement or a specific, named
 * divergence, and BOTH directions fail:
 *
 *   expect: 'match'        and the bytes differ  -> UNEXPECTED (loud)
 *   expect: { divergence } and the bytes agree   -> STALE EXPECTATION (also loud)
 *
 * The second is not a formality. A declared divergence that has stopped happening
 * means the justification written next to it is now false, and the next person to
 * read it will make a decision on a fact that expired. */

import { deepEqual, describeDiff } from '../differential/compare.mjs';

export { deepEqual, describeDiff };

/** The canonical serialisation both sides are compared through.
 *
 * `undefined` is spelled out rather than left to `JSON.stringify`, which answers
 * `undefined` (not a string) for a bare `undefined` and would make two absent
 * values compare as equal to a bare `null` under `String()`. The twin returns
 * `undefined` in one place (`AddonStream.subtitles`) and `null` in another
 * (`AddonStream.size`), and those are different values. */
export function bytes(v) {
  if (v === undefined) return '<undefined>';
  const s = JSON.stringify(v);
  return s === undefined ? '<undefined>' : s;
}

/** Unwrap a core envelope, asserting the five-key shape. A malformed envelope is a
 * harness-level failure and not a data difference: the shell parses this string on
 * every call and has no recovery from a bad one. */
export function unwrap(raw, fn) {
  let v;
  try { v = JSON.parse(raw); } catch (e) {
    throw new Error(`${fn}: the core returned unparseable JSON: ${e.message}\n${raw}`);
  }
  for (const k of ['ok', 'core', 'data', 'warnings', 'error']) {
    if (!(k in v)) throw new Error(`${fn}: envelope is missing \`${k}\`: ${raw}`);
  }
  return { ...v, codes: (v.warnings || []).map((w) => w.code) };
}

/**
 * @param {object} f      the fixture (carries `expect`)
 * @param {*} tsOut       what the TypeScript/JavaScript twin answered
 * @param {*} coreOut     what the core answered, envelope already unwrapped
 * @param {string[]} notes
 */
export function verdict(f, tsOut, coreOut, notes) {
  const a = bytes(tsOut);
  const b = bytes(coreOut);
  const same = a === b;
  const wanted = f.expect;

  // Distinguish "the value differs" from "only the spelling differs". Both fail a
  // `match`, but they are different bugs and the report has to say which.
  let diff = null;
  if (!same) {
    diff = deepEqual(tsOut, coreOut)
      ? `same VALUE, different BYTES (key order or number formatting)\n          ts   ${a}\n          rust ${b}`
      : `${describeDiff(tsOut, coreOut) ?? 'differ'}\n          ts   ${a}\n          rust ${b}`;
  }

  /* `ts`/`rust` ride along on EVERY verdict, agreement included, so `--verbose` can
   * show what was compared and not merely that it agreed.
   *
   * That is not decoration. Two empty arrays agree, and so do thirty proven fields; the
   * run that mistakenly compared the core against its own call site reported both as
   * `ok` and the difference was invisible from the report. A fixture whose two sides
   * are `[]` is a fixture that proves nothing, and the only way to notice is to be able
   * to read the bytes. */
  const seen = { ts: a, rust: b };
  if (wanted === 'match') {
    return same ? { status: 'match', notes, ...seen } : { status: 'unexpected', notes, diff, ...seen };
  }
  if (!same) return { status: 'diverged', notes, diff, divergence: wanted, ...seen };
  return { status: 'stale-expectation', notes, divergence: wanted, ...seen };
}
