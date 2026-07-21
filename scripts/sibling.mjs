/* Resolve a sibling repository directory by trying several names.
 *
 * TEMPORARY SCAFFOLDING — delete once the folders are renamed.
 *
 * The product was renamed Stredio -> Groloo on 2026-07-21. The GitHub repositories
 * were renamed the same day, but the local working directories could not be: on
 * Windows an open editor holds a handle on them and `mv` fails with EPERM/EBUSY.
 * That left this repo's tooling — the wasm build's vendor target, the differential
 * harness and the parity harness — pointing at `../Groloo-Web` while the folder on
 * disk was still `../Stredio-Web`.
 *
 * Rather than order the rename against the tooling, every sibling lookup accepts
 * both names and takes whichever exists. The rename can then happen at any time,
 * in any order, with nothing to re-sequence.
 *
 * When every machine has the new folder names, drop the legacy entries below and
 * inline `resolve(from, '..', 'Groloo-Web')` again.
 */

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * @param {string} from   directory to resolve relative to
 * @param {...string} names candidate directory names, preferred first
 * @returns {string} the first candidate that exists, else the first candidate —
 *   so callers still produce their own, more specific "I could not find X" error
 *   instead of this helper inventing one.
 */
export function siblingRepo(from, ...names) {
  for (const name of names) {
    const p = resolve(from, '..', name);
    if (existsSync(p)) return p;
  }
  return resolve(from, '..', names[0]);
}

/** The web shell. Post-rename name first. */
export const webRepo = (from) => siblingRepo(from, 'Groloo-Web', 'Stredio-Web');

/** The API server. Post-rename name first. */
export const serverRepo = (from) => siblingRepo(from, 'Groloo-server', 'Stredio-server');
