/* Deciding whether two answers agree — and, when they do not, whether that was
 * the plan.
 *
 * The temptation in a differential harness is to make the comparison forgiving
 * until it goes green. That is exactly the failure this file is built to prevent,
 * so it works the other way round: every fixture states UP FRONT whether it
 * expects agreement or expects a specific, named divergence, and BOTH directions
 * are failures.
 *
 *   expect: 'match'           and they differ  -> UNEXPECTED DIFFERENCE (loud)
 *   expect: { divergence }    and they agree   -> STALE EXPECTATION (also loud)
 *
 * The second half matters as much as the first. A divergence that has quietly
 * stopped happening means the document describing this phase is now wrong, and a
 * harness that only shouts in one direction will never say so.
 *
 * ## Normalisation is declared, never silent
 *
 * The new core answers in an envelope and its `Library` has two more fields and
 * its `Progress` one more key. Those are real differences and the harness must not
 * pretend otherwise, so every adjustment made before comparing is recorded as a
 * NOTE on the result and printed. `stripLang` in particular does not throw the
 * language away: it hands it back so the fixture can assert what it should have
 * been. A field that is removed from the comparison but asserted somewhere else is
 * still tested; a field that is merely removed is not. */

/** Deep structural equality over parsed JSON. Key ORDER is ignored for objects
 * (both cores emit BTreeMap-ordered keys, but relying on that would make the
 * harness fail for a reason nobody cares about) and honoured for arrays, where it
 * is the answer. */
export function deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null) return false;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }
  if (typeof a === 'object') {
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (ka.length !== kb.length || ka.some((k, i) => k !== kb[i])) return false;
    return ka.every((k) => deepEqual(a[k], b[k]));
  }
  return false;
}

/** A readable, bounded description of where two values first disagree. Printing
 * two 60-entry libraries side by side teaches nobody anything. */
export function describeDiff(a, b, path = '$') {
  if (deepEqual(a, b)) return null;
  const t = (v) => (Array.isArray(v) ? 'array' : v === null ? 'null' : typeof v);
  if (t(a) !== t(b)) return `${path}: ${t(a)} (old) vs ${t(b)} (new)`;
  if (Array.isArray(a)) {
    if (a.length !== b.length) return `${path}: length ${a.length} (old) vs ${b.length} (new)`;
    for (let i = 0; i < a.length; i += 1) {
      const d = describeDiff(a[i], b[i], `${path}[${i}]`);
      if (d) return d;
    }
  }
  if (a && typeof a === 'object' && !Array.isArray(a)) {
    for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) {
      if (!(k in a)) return `${path}.${k}: absent (old) vs ${json(b[k])} (new)`;
      if (!(k in b)) return `${path}.${k}: ${json(a[k])} (old) vs absent (new)`;
      const d = describeDiff(a[k], b[k], `${path}.${k}`);
      if (d) return d;
    }
  }
  return `${path}: ${json(a)} (old) vs ${json(b)} (new)`;
}

const json = (v) => {
  const s = JSON.stringify(v);
  return s === undefined ? 'undefined' : s.length > 160 ? `${s.slice(0, 157)}...` : s;
};

/* ------------------------------------------------------------------------- */
/* envelope handling                                                          */
/* ------------------------------------------------------------------------- */

/** Unwrap a new-core response, asserting the five-key shape on the way through.
 * A malformed envelope is a harness-level failure, not a data difference — the
 * shell parses this string on every call and cannot recover from a bad one. */
export function unwrap(raw, fn) {
  let v;
  try { v = JSON.parse(raw); } catch (e) {
    throw new Error(`${fn}: new core returned unparseable JSON: ${e.message}\n${raw}`);
  }
  for (const k of ['ok', 'core', 'data', 'warnings', 'error']) {
    if (!(k in v)) throw new Error(`${fn}: envelope is missing \`${k}\`: ${raw}`);
  }
  return {
    ok: v.ok,
    core: v.core,
    data: v.data,
    warnings: v.warnings,
    error: v.error,
    codes: (v.warnings || []).map((w) => w.code),
  };
}

/* ------------------------------------------------------------------------- */
/* declared normalisations                                                    */
/* ------------------------------------------------------------------------- */

/** Drop `mylist` / `mylistRemoved` from a new-core Library, but ONLY when they
 * are empty. A non-empty value here is a genuine extra output and must reach the
 * comparison so it can fail; silently discarding it would be the harness hiding a
 * real difference in order to stay green. */
export function stripMyList(lib, notes) {
  if (!lib || typeof lib !== 'object') return lib;
  const out = { ...lib };
  for (const k of ['mylist', 'mylistRemoved']) {
    const v = out[k];
    const empty = Array.isArray(v) ? v.length === 0 : v && Object.keys(v).length === 0;
    if (k in out && empty) {
      delete out[k];
      notes.push(`D-envelope: dropped empty \`${k}\` (new-only field, absent in 0.1.0)`);
    }
  }
  return out;
}

/** Lift `Progress.lang` OUT of the comparison and INTO the returned map, so the
 * fixture can assert it. The old core's `Progress` has three fields and no
 * `lang`, so leaving it in would fail every progress fixture for one known
 * reason; deleting it without reporting would mean D4 is never checked at all. */
export function stripLang(lib, notes) {
  const langs = {};
  if (!lib || typeof lib.progress !== 'object' || lib.progress === null) return { lib, langs };
  const progress = {};
  for (const [k, p] of Object.entries(lib.progress)) {
    if (p && typeof p === 'object' && 'lang' in p) {
      langs[k] = p.lang;
      const { lang, ...rest } = p;
      progress[k] = rest;
    } else {
      progress[k] = p;
    }
  }
  if (Object.keys(langs).length) {
    notes.push(`D4: lifted Progress.lang out of ${Object.keys(langs).length} record(s): ${json(langs)}`);
  }
  return { lib: { ...lib, progress }, langs };
}

/* ------------------------------------------------------------------------- */
/* the verdict                                                                */
/* ------------------------------------------------------------------------- */

/**
 * @param {object} f      the fixture (carries `expect`)
 * @param {*}      oldOut old core's answer, already parsed
 * @param {*}      newOut new core's answer, already parsed and normalised
 * @param {string[]} notes normalisations that were applied
 */
export function verdict(f, oldOut, newOut, notes) {
  const same = deepEqual(oldOut, newOut);
  const diff = same ? null : describeDiff(oldOut, newOut);
  const wanted = f.expect;

  if (wanted === 'match') {
    return same
      ? { status: 'match', notes }
      : { status: 'unexpected', notes, diff, oldOut, newOut };
  }
  // A declared divergence.
  if (!same) return { status: 'diverged', notes, diff, divergence: wanted };
  return { status: 'stale-expectation', notes, divergence: wanted };
}
