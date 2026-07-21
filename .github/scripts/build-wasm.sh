#!/usr/bin/env bash
#
# The ONE sanctioned way to produce the WebAssembly artifact that Groloo-Web
# ships. CI runs this script; a human rebuilding `pkg/` must run this script too.
# Typing the two raw commands out of the README by hand produces bytes CI will
# reject, and that is deliberate rather than unfortunate — read on.
#
# WHAT THIS ADDS over `cargo build && wasm-bindgen`:
#
#   1. --remap-path-prefix. rustc bakes the absolute source path of every
#      potentially-panicking line into the module's data section, including the
#      paths of dependencies inside CARGO_HOME. Those paths are per-machine:
#
#        C:\Users\you\.cargo\registry\src\index.crates.io-<hash>\serde_json-…
#        /home/runner/.cargo/registry/src/index.crates.io-<hash>/serde_json-…
#
#      Different LENGTHS mean different data-section offsets, which means
#      different i32.const immediates throughout the code section, which means
#      two builds of identical source share almost no bytes. Remapping both the
#      registry root and the workspace root to fixed, machine-independent
#      strings collapses that difference to the path SEPARATOR alone — one byte
#      each, same length, inside the data section only. That residue is what
#      `wasm-summary.mjs` normalises away, and it is the reason the freshness
#      gate can compare content rather than merely shape.
#
#   2. A hard equality check between the wasm-bindgen CLI on PATH and the
#      wasm-bindgen crate version in Cargo.lock. A mismatch does not always
#      error; it can emit a JS shim that is subtly wrong at runtime, on a
#      television, months later.
#
#   3. GROLOO_CORE_GIT_SHA pass-through. `build.rs` stamps `<semver>+g<rev>`
#      into the binary, and the committed artifact necessarily records the
#      revision that existed BEFORE the commit carrying it — so a rebuild at HEAD
#      can never reproduce it. CI reads the stamp out of the committed artifact
#      and feeds it back here, which pins the one field that is legitimately
#      allowed to differ instead of averting its eyes from the whole file.
#
# Usage: build-wasm.sh <out-dir> [git-sha]
#   out-dir  wiped and recreated; `pkg` for the committed artifact.
#   git-sha  optional; overrides what build.rs would read from git.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${1:-}"
GIT_SHA="${2:-}"

if [ -z "$OUT_DIR" ]; then
  echo "usage: $(basename "$0") <out-dir> [git-sha]" >&2
  exit 2
fi

CRATE='groloo-core-wasm'
LIB='groloo_core_wasm'
TARGET='wasm32-unknown-unknown'

# ---------------------------------------------------------------- toolchain ---

# Cargo.lock is the source of truth, not the manifest: the manifest pins `=0.2.126`
# today but a `version.workspace` refactor could quietly loosen it, whereas the
# lockfile records what was actually linked.
LOCK_WBG="$(awk '
  /^name = "wasm-bindgen"$/ { seen = 1; next }
  seen && /^version = / { gsub(/[",]/, ""); print $3; exit }
' "$ROOT/Cargo.lock")"

if [ -z "$LOCK_WBG" ]; then
  echo "error: could not read the wasm-bindgen version from Cargo.lock" >&2
  exit 1
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "error: wasm-bindgen CLI not on PATH; install exactly $LOCK_WBG" >&2
  exit 1
fi

CLI_WBG="$(wasm-bindgen --version | awk '{ print $2 }')"
if [ "$CLI_WBG" != "$LOCK_WBG" ]; then
  echo "error: wasm-bindgen CLI is $CLI_WBG but Cargo.lock links $LOCK_WBG." >&2
  echo "       These must be equal. A mismatch can emit a silently wrong JS shim." >&2
  exit 1
fi

# ------------------------------------------------------------ deterministic ---

# rustc is a native binary, so on Windows it sees native paths even when this
# script runs under Git Bash. Remap prefixes must therefore be given in the host's
# own form or they simply never match anything.
CARGO_ROOT="${CARGO_HOME:-$HOME/.cargo}"
if command -v cygpath >/dev/null 2>&1; then
  REGISTRY_SRC="$(cygpath -w "$CARGO_ROOT")\\registry\\src"
  ROOT_NATIVE="$(cygpath -w "$ROOT")"
else
  REGISTRY_SRC="$CARGO_ROOT/registry/src"
  ROOT_NATIVE="$ROOT"
fi

# CARGO_ENCODED_RUSTFLAGS rather than RUSTFLAGS: the latter is split on spaces, so
# a checkout under "C:\My Projects" would silently produce two broken half-flags.
# The separator is a literal 0x1f.
US=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${REGISTRY_SRC}=/cargo/registry/src${US}--remap-path-prefix=${ROOT_NATIVE}=/groloo-core"

if [ -n "$GIT_SHA" ]; then
  export GROLOO_CORE_GIT_SHA="$GIT_SHA"
fi

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

echo "==> workspace     $ROOT"
echo "==> wasm-bindgen  $CLI_WBG (matches Cargo.lock)"
echo "==> rustc         $(rustc --version)"
echo "==> git stamp     ${GROLOO_CORE_GIT_SHA:-<read from git by build.rs>}"

# ------------------------------------------------------------------- build ---

( cd "$ROOT" && cargo build --release --target "$TARGET" -p "$CRATE" )

# Wiped, not merged. A stale file left behind from an earlier export would sail
# through a file-by-file comparison and ship to every client.
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

wasm-bindgen "$TARGET_DIR/$TARGET/release/$LIB.wasm" \
  --out-dir "$OUT_DIR" \
  --target web

echo "==> wrote $OUT_DIR"
ls -l "$OUT_DIR"
