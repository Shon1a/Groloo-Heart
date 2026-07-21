# groloo-core

The domain logic every [GROLOO](https://github.com/Shon1a/Groloo) shell shares —
web (WASM), LG webOS, Android TV, and the node server. **Pure and I/O-free**
(`#![forbid(unsafe_code)]`): bytes in, bytes out. Network, storage, clocks and
rendering belong to the shell, so a rule fixed once holds identically everywhere.

## The boundary

**Free functions. JSON string in, JSON string out.**

```rust
pub fn merge_library(local_json: &str, remote_json: &str, now: f64) -> String
```

No state held in Rust. No handles, no lifetimes, no `Msg` enum, nothing to
marshal but `&str` and `String`. Every response is an envelope:

```json
{ "ok": true, "core": "0.2.0+g1a2b3c4", "data": {}, "warnings": [], "error": null }
```

`data` is **always present and never null** — on failure it holds the
graceful-degradation value (the unmodified input, the inline defaults, `[]`).
A shell that ignores `ok` behaves exactly as it does today; a shell that reads it
can finally tell *empty* from *broken*.

## Relationship to Groloo-Heart

This crate is the successor to
[`Groloo-Heart`](https://github.com/Shon1a/Groloo-Heart) (MIT, same copyright).
Heart's ~600 lines of real domain logic are carried across **behaviour-frozen**,
and a differential harness pins the new core against the exact wasm build
Groloo-Web ships today before anything is repointed. What does not come across is
the Elm runtime around them:

| Heart | Here | Why |
|---|---|---|
| `runtime.rs` — `Model` / `Msg` / `update` / `Effect` | gone | The shell already performs every "effect". `official.ts` parsed the effect *array* purely to read a filename out of it. |
| three disconnected reducers, `CatMsg::SetGating` forwarded by the shell | free functions | Gating was a message one reducer had to relay to another because they could not see each other. It is now the second argument to a function. |
| `wasm.rs` — 304 hand-written lines, 27 methods (the shell called 11) | ~35-line forwarding shim | Per-message bindings cost the same 304 lines again for JNI, and again for webOS. `&str -> String` costs nothing on any of them. |
| `ffi.rs` | gone as a name | It contained no FFI — no `extern "C"`, no `#[no_mangle]`, no `repr(C)`. The name is reserved for `groloo-core-ffi`, the real UniFFI crate in Phase 5. |
| `hydrate` → op → `snapshot` on every playback tick | single-shot merges | The runtime was being used as a stateless JSON transform anyway. The boundary is now shaped like the way it is actually used. |

Heart also `unwrap_or_default()`s a failed library parse, so a single
undeserializable record wipes the user's whole history through the
hydrate/snapshot round trip. Here, a parse failure returns the input unchanged
and says so.

## Layout

```
crates/groloo-core/         rlib    — the domain logic. serde + serde_json, nothing else.
crates/groloo-core-wasm/    cdylib  — wasm-bindgen shim. One line per function, zero logic.
(crates/groloo-core-ffi/            — RESERVED: UniFFI / Android, Phase 5. Not created yet.)
```

Everything public is tested **through the boundary**, against the exact strings JS
receives — not against the typed internals. The wasm crate is untestable by
`cargo test` by construction, which is precisely why it is required to contain
nothing worth testing.

## Build

```bash
cargo test                                  # the boundary, via cargo
cargo clippy --all-targets -- -D warnings   # includes the no-panic lint wall
cargo build --release

# browser (WebAssembly) — no wasm-pack, same two steps Heart's README used:
cargo build --release --target wasm32-unknown-unknown -p groloo-core-wasm
wasm-bindgen target/wasm32-unknown-unknown/release/groloo_core_wasm.wasm \
  --out-dir pkg --target web
```

The `wasm-bindgen` **CLI version must equal the `wasm-bindgen` crate version**
(pinned `=0.2.126` in the workspace manifest). A mismatch produces a wrong JS shim,
sometimes silently.

## Delivery rules

These exist because Heart got each of them wrong, and each failure was invisible:

- **`Cargo.lock` is committed.** Heart gitignored it, so no two builds of the
  shipped `.wasm` provably used the same dependency graph.
- **`rust-toolchain.toml` pins 1.96.1.** Compiler upgrades change codegen; bump it
  deliberately and re-verify the artifact.
- **The artifact is rebuilt in CI and asserted byte-identical** to the committed
  one. Heart's `web/` was hand-committed and never verified.
- **Tags are real.** Heart shipped `0.1.0` forever from a mutable `@master`
  jsDelivr ref.

## Panics

`wasm32-unknown-unknown` has no unwinding on stable Rust, so `catch_unwind` there
catches nothing and a panic traps the instance **permanently**. Retrying a poisoned
instance is the silent-death path this crate exists to close. The defence is
layered rather than reactive:

1. `crates/groloo-core` **denies** `unwrap` / `expect` / `panic` / indexing /
   overflowing arithmetic at the crate root. Panicking paths fail the build.
   `#[allow]` is sanctioned only inside `#[cfg(test)]`.
2. The wasm crate installs `console_error_panic_hook`, so anything that escapes (1)
   is legible in a TV devtools console instead of `RuntimeError: unreachable`.
3. The shell treats any thrown value as terminal — `heartStatus.state = 'panicked'`
   — and stops calling the instance rather than retrying it.
4. `panic = "unwind"` is set on the release profile. It is inert on wasm and
   load-bearing on Android, where `abort` would SIGABRT the whole app.

## Versioning

`build.rs` stamps the git revision, and `CORE_VERSION` is `<semver>+g<rev>`
(`+gunknown` when there is no revision to read). The shell calls `core_version()`
immediately after instantiate and compares it to the pinned
`public/assets/heart/<version>/` folder it loaded from — a stale or mispointed
vendored copy is otherwise completely invisible.

## License

[MIT](./LICENSE) — code only; no media, no stream sources.
Copyright (c) 2026 Shon1a / GROLOO.
