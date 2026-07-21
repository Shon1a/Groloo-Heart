/* Substitute for `Stredio-Web/src/lib/api.ts` — see `../hooks.mjs` for why.
 *
 * The real file computes `API_BASE` from `import.meta.env.VITE_API_BASE` while it is
 * being evaluated, which is a Vite construct: under node `import.meta.env` is
 * `undefined` and the module throws before anything can import it.
 *
 * `api()` throws rather than resolving. Nothing this corpus asserts calls it —
 * `stores/addons.ts` reaches it only from `serverInstall`/`serverRemove`, both
 * guarded by `authed()`, and the auth substitute reports a signed-out device — so a
 * stub that quietly returned `{}` would let a future fixture wander onto the network
 * path and pass without anybody noticing that it had. */

export const API_BASE = '';

export const api = async (path) => {
  throw new Error(`parity: the twin called api(${path}); the corpus asserts no server round-trip`);
};

export const apiFetch = async (path) => {
  throw new Error(`parity: the twin called apiFetch(${path}); the corpus asserts no server round-trip`);
};

export const apiPost = async (path) => {
  throw new Error(`parity: the twin called apiPost(${path}); the corpus asserts no server round-trip`);
};

export const getToken = () => '';
export const setSessionToken = () => {};
export class ApiError extends Error {}
export const errorCode = () => '';
export const errorMessage = (e) => String(e);
