/* Substitute for `Groloo-Web/src/stores/auth.ts` — see `../hooks.mjs` for why.
 *
 * `stores/addons.ts` calls `useAuth.subscribe(...)` at module scope and reads
 * `useAuth.getState().user` from `email()` and `authed()`. The real store is a
 * zustand store wired to `lib/api`, session tokens and a rehydrate-on-load effect;
 * importing it would drag the whole session machinery into a corpus about string
 * parsing.
 *
 * It reports a SIGNED-OUT device, which is the honest default rather than a
 * convenient one: `authed()` is then false, every write in the add-on store stays
 * local, and no path the corpus touches can reach `api()`. A signed-in stub would
 * make `serverInstall` fire on the `validate_manifest` fixtures and force a second
 * stub to swallow it. */

export const useAuth = {
  getState: () => ({ user: null, token: null, loading: false }),
  setState: () => {},
  subscribe: () => () => {},
};
