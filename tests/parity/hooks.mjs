/* The module-resolution hook, and the two substitutions it makes.
 *
 * It exists because Vite lets an import omit its extension and node's ESM resolver
 * does not — `import { api } from '../lib/api'` is a hard resolution failure under
 * node. The hook appends `.ts` to an extensionless relative specifier and does
 * nothing else to it: no transform, no inlining, no rewriting of the module body.
 * Node 23+ strips the types natively, so the code that runs is the code in the
 * repository.
 *
 * ## The two modules that ARE substituted, and why each is unavoidable
 *
 *   src/lib/api.ts      reads `import.meta.env.VITE_API_BASE` at module scope,
 *                       which is a Vite construct — under node `import.meta.env`
 *                       is `undefined` and the file throws while being evaluated.
 *   src/stores/auth.ts  is the session store; `stores/addons.ts` calls
 *                       `useAuth.subscribe(...)` at module scope.
 *
 * Both are imported by `stores/addons.ts`, which this corpus needs for the real
 * `normalizeManifestUrl`. Neither is reachable from anything the corpus asserts:
 * `normalizeManifestUrl` is a pure string function, and the add-on client imports
 * neither. The substitutes are therefore the minimum required to make the real file
 * loadable, and they are listed here — in the file that performs the substitution —
 * so the claim can be checked rather than taken on trust.
 *
 * The auth stub reports a signed-OUT device. That is the honest default: it makes
 * every write in the store local, so nothing the corpus does can reach a network
 * call that would then need a second stub to swallow.
 *
 * ## The third thing it does: serve a twin that is no longer on disk
 *
 * The whole corpus exists to gate a DELETION, which means the implementation it has
 * to run is, by the time anyone reads the verdict, frequently the one that has just
 * been deleted. `twins.mjs` reads those bytes out of git and passes them in as
 * `virtual` — a map of module URL -> source text — and the `load` hook below hands
 * them to node instead of the file on disk.
 *
 * Two properties make this a QUOTATION rather than a transcription, which is the
 * distinction the twin rule turns on:
 *
 *   1. The source is `git show`n, never typed out here. Nothing in this repository
 *      describes what the twin used to do; the harness reads it.
 *   2. The virtual URL is the REAL path with a `?parity-twin=` query on the end, so
 *      every relative import inside it (`../stores/addons`) resolves against the real
 *      `src/lib/` directory and reaches the same module instance the working-tree copy
 *      would. The twin runs against the app's own store, not a second one.
 *
 * `format: 'module-typescript'` hands the text to node's OWN type stripper — the same
 * one it applies to a `.ts` file on disk — so the bytes executed are the bytes git
 * holds, minus exactly the annotations node would have removed anyway. This file
 * still transforms nothing. */

import { pathToFileURL } from 'node:url';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const stub = (name) => pathToFileURL(`${HERE}/stubs/${name}`).href;

/** Specifier suffix -> substitute. Suffix-matched because the same module is
 * imported as `../lib/api` from a store and as `./api` from a sibling. */
const SUBSTITUTED = [
  ['/lib/api', 'api.mjs'],
  ['/stores/auth', 'auth.mjs'],
];

/** URL -> source text, supplied by `twins.mjs` through `register(..., { data })`.
 * Empty unless the harness had to reach for a twin that is not on disk. Hooks run on
 * their own thread, so this cannot be a shared reference — it is handed over once, at
 * registration, and never mutated afterwards. */
let VIRTUAL = new Map();

export async function initialize(data) {
  VIRTUAL = new Map(Object.entries((data && data.virtual) || {}));
}

export async function resolve(specifier, context, next) {
  for (const [suffix, file] of SUBSTITUTED) {
    if (specifier === `.${suffix}` || specifier.endsWith(suffix)) {
      return { url: stub(file), shortCircuit: true, format: 'module' };
    }
  }
  // A virtual twin is imported by its full URL, query and all, and must not be sent
  // round the resolver again — the query is what distinguishes it from the file.
  if (VIRTUAL.has(specifier)) return { url: specifier, shortCircuit: true, format: 'module-typescript' };
  const relative = specifier.startsWith('./') || specifier.startsWith('../');
  // The extension test looks at the path only: a virtual twin's parent URL carries a
  // query, but its own relative imports do not, and `./types?x` is not a thing here.
  if (relative && !/\.[mc]?[jt]sx?$/.test(specifier.split('?')[0])) return next(`${specifier}.ts`, context);
  return next(specifier, context);
}

export async function load(url, context, next) {
  const source = VIRTUAL.get(url);
  if (source !== undefined) return { format: 'module-typescript', source, shortCircuit: true };
  return next(url, context);
}
