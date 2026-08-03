# Where the 434,901 bytes go

`ci.yml` has carried a note since the budget was first set saying the module is
larger than the Heart build it replaces, that this "points at the serde derive
and formatting surface rather than at the binding layer", and that the right
response is to measure it rather than raise the ceiling. The ceiling was raised
once, to 480 KiB, because five exports had been added since it was set and the
gate was blocking on a number measured against a smaller surface. Raising it did
not answer the question. This file answers it.

## Method

`cargo build --release --target wasm32-unknown-unknown -p groloo-core-wasm`
with `profile.release.strip=false` and nothing else changed, so the code is the
shipped code and only the name section is added. `twiggy top` / `twiggy monos`
over the result. Attributed code totals 432,682 B against the shipped module's
434,901 B, so these figures map onto the real artifact essentially 1:1 — they are
not a debug build's inflated numbers.

Reproduce with:

```sh
CARGO_TARGET_DIR=/tmp/diag cargo build --release \
  --target wasm32-unknown-unknown -p groloo-core-wasm \
  --config 'profile.release.strip=false'
twiggy top  -n 30 /tmp/diag/wasm32-unknown-unknown/release/groloo_core_wasm.wasm
twiggy monos       /tmp/diag/wasm32-unknown-unknown/release/groloo_core_wasm.wasm
```

## The breakdown

| area | bytes | share |
| --- | ---: | ---: |
| **serde** (`serde_json` runtime 59,996 + derive/traits 61,153) | **121,149** | **28.0%** |
| std/core miscellany | 177,416 | 41.0% |
| `groloo_core` + the wasm binding — *our own code* | 68,816 | 15.9% |
| sorting (`quicksort` + `drift::sort`) | 37,259 | 8.6% |
| float formatting and parsing | 20,511 | 4.7% |
| allocator (dlmalloc) | 7,531 | 1.7% |

The standing hypothesis in `ci.yml` was right, and worth stating plainly: **serde
is 28% of the module and our own logic is 16%.** The binding layer was never the
problem. Nothing stray is linked in — 4 imports, and the dependency graph is
serde, serde_json, wasm-bindgen and console_error_panic_hook.

## Duplicate generics — ~62 KB, and the cheapest thing to fix

`twiggy monos` measures the same generic function compiled once per type it is
used with. The "bloat" column is what the extra copies cost over keeping one.

| generic | wasted | total | note |
| --- | ---: | ---: | --- |
| `FilterMap::next` | 14,119 | 22,370 | 11 copies |
| `slice::sort::quicksort` | 13,962 | 16,195 | |
| **`Envelope<T>::into_json`** | **13,000** | **15,189** | **ours** |
| `slice::sort::drift::sort` | 10,696 | 12,196 | |
| `SerializeMap::serialize_entry` | 6,381 | 8,105 | |
| `deserialize_seq` | 4,230 | 11,319 | |

`Envelope<T>::into_json` is the one to look at first, because it is ours and
because 13,000 wasted bytes is 3% of the module for a function whose generic
parameter does no work after the value is serialised. The standard shape is to
keep the generic wrapper trivial and have it call a single non-generic inner
function taking the already-serialised value, so `T` stops multiplying the body.
The two `slice::sort` entries are likely to fall out of the same change or of
sorting concrete types in fewer places.

## What has NOT been done, and why

**No code has been changed to act on any of this.** The two behavioural gates —
`tests/differential` and `tests/parity` — cannot run: `SIBLING_REPOS_TOKEN` is
not configured, so the `gates` job fails at its first step. Those gates exist
precisely to catch "a change that compiles, is formatted, lints clean, passes
every unit test and rebuilds to the right bytes, and STILL changes what a user
sees". A size refactor of the serialisation path is exactly that class of change.

The 260 unit tests pass, and they are not sufficient — the repository already
says so, at length, and the U-slash defect that survived a green CI is the reason
it says so. Optimising the serialisation path while the only gates that could
catch a regression are switched off would be the same mistake in a new coat.

So: measured, recorded, and left alone until the gates can run.

## Order of work, once the gates are green

1. Turn on `SIBLING_REPOS_TOKEN` and get `differential + parity` running. Settle
   the open `U1` divergence in `tests/differential/DIVERGENCES.md` while there.
2. `Envelope<T>::into_json` — ~13 KB, our own code, lowest risk.
3. Re-measure. Ratchet `WASM_BUDGET_BYTES` **down** to the new figure plus the
   same ~13% headroom. The ceiling only ever moving up is how a budget stops
   being a budget.
4. `wasm-opt -Oz` is the other lever `ci.yml` names, and it carries a condition:
   the artifact gate compares bytes, so the wasm-opt version would have to be
   pinned and installed in CI exactly as the wasm-bindgen CLI already is.
   Unpinned, it breaks reproducibility rather than improving anything.

## The JS glue has no room left

30,395 B against a 32,768 B ceiling — 92.8%, where it was 74% when the budget was
written. One or two more exports breach it. That ceiling was deliberately **not**
raised alongside the wasm one. When it does breach, the first move is to read
this file again, not to add 8 KB.
