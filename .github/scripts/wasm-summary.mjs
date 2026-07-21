/*
 * A deterministic, host-independent structural summary of a WebAssembly module.
 *
 * WHY this exists rather than `wasm-objdump` or `wasm-tools`: neither is present
 * on a GitHub runner, both would have to be installed from the network on every
 * run, and the artifact-freshness gate is the one job in this repo that must not
 * depend on anything it cannot verify. The wasm binary format's section framing
 * is a few hundred bytes of LEB128 — cheaper to read than to trust a download.
 *
 * WHY host-independent matters, concretely. rustc bakes the source path of every
 * potentially-panicking line into the module's data section. Those paths are
 * absolute and therefore differ between the machine that built the committed
 * artifact and the CI runner that rebuilds it:
 *
 *   C:\Users\...\.cargo\registry\src\index.crates.io-<hash>\serde_json-1.0.150\...
 *   /home/runner/.cargo/registry/src/index.crates.io-<hash>/serde_json-1.0.150/...
 *
 * `.github/scripts/build-wasm.sh` remaps that prefix to a fixed `/cargo/registry/
 * src`, which makes the two strings the same LENGTH — so every offset, every
 * section size and the entire code section line up byte for byte. What survives
 * the remap is the path SEPARATOR: `\` on Windows, `/` on Linux, one byte each,
 * inside the data section only. `normalisedHash` below folds exactly that away
 * and nothing else, so the resulting digest is a true content hash of the module
 * that is nevertheless stable across the OS that produced it.
 */

import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

/* Section ids from the core spec, plus 13 (tag) from exception handling. Names
 * are cosmetic — the gate compares ids — but a diff that says "code" instead of
 * "10" is a diff a human can act on at 2am. */
const SECTION_NAMES = {
  0: 'custom', 1: 'type', 2: 'import', 3: 'function', 4: 'table',
  5: 'memory', 6: 'global', 7: 'export', 8: 'start', 9: 'element',
  10: 'code', 11: 'data', 12: 'data-count', 13: 'tag',
};

const EXTERN_KINDS = ['func', 'table', 'memory', 'global', 'tag'];

const DATA_SECTION_ID = 11;

/* The stamp `build.rs` bakes in: `<semver>+g<rev>`, where <rev> is a 7-char git
 * short sha or the literal `unknown`. It lives in the data section as plain
 * UTF-8, which is why a naive byte comparison of two builds of the SAME source
 * fails: the committed artifact necessarily records the revision BEFORE the
 * commit that contains it. The gate pins it instead of ignoring it. */
const VERSION_STAMP = /(\d+\.\d+\.\d+)\+g([0-9a-f]{7,40}|unknown)/g;

class Reader {
  constructor(buf, pos = 0) {
    this.buf = buf;
    this.pos = pos;
  }

  u8() {
    if (this.pos >= this.buf.length) throw new Error('truncated module');
    return this.buf[this.pos++];
  }

  /* LEB128. Accumulated with multiplication rather than `<<`, because a shift of
   * 28 or more silently wraps in JS and a wrapped section length would be read
   * as a valid-looking module. */
  varu32() {
    let result = 0;
    let shift = 0;
    for (;;) {
      const byte = this.u8();
      result += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) break;
      shift += 7;
      if (shift > 35) throw new Error('LEB128 varuint32 too long');
    }
    if (!Number.isSafeInteger(result)) throw new Error('LEB128 varuint32 out of range');
    return result;
  }

  bytes(n) {
    const end = this.pos + n;
    if (end > this.buf.length) throw new Error('truncated module');
    const out = this.buf.subarray(this.pos, end);
    this.pos = end;
    return out;
  }

  name() {
    return Buffer.from(this.bytes(this.varu32())).toString('utf8');
  }

  /* limits := 0x00 min | 0x01 min max | 0x04/0x05 (64-bit memory). Only the
   * shape matters here: a core that suddenly grew a maximum, or lost one, has
   * changed its memory contract with the shell. */
  limits() {
    const flags = this.u8();
    const min = this.varu32();
    const max = (flags & 0x01) !== 0 ? this.varu32() : null;
    return { flags, min, max };
  }
}

/*
 * Parse the framing only. Function bodies are never decoded — the gate hashes
 * them, it does not need to understand them.
 */
export function parseWasm(buf) {
  if (buf.length < 8 || buf.readUInt32BE(0) !== 0x0061736d) {
    throw new Error('not a WebAssembly module (bad magic)');
  }
  const version = buf.readUInt32LE(4);

  const sections = [];
  const exports = [];
  const imports = [];
  const memories = [];
  const tables = [];
  const counts = {};

  let i = 8;
  while (i < buf.length) {
    const id = buf[i];
    const head = new Reader(buf, i + 1);
    const size = head.varu32();
    const start = head.pos;
    const end = start + size;
    if (end > buf.length) throw new Error(`section ${id} overruns the module`);

    const section = {
      id,
      name: SECTION_NAMES[id] ?? `unknown-${id}`,
      size,
      start,
      end,
    };

    const r = new Reader(buf, start);
    switch (id) {
      case 0:
        /* Custom sections carry `producers` (rustc + wasm-bindgen versions) and
         * `target_features`. Both are stable under a pinned toolchain, so they
         * are signal: if one changes, someone's toolchain drifted. */
        section.customName = r.name();
        break;
      case 2: {
        const n = r.varu32();
        for (let k = 0; k < n; k++) {
          const module = r.name();
          const field = r.name();
          const kind = r.u8();
          imports.push({ module, name: field, kind: EXTERN_KINDS[kind] ?? `kind-${kind}` });
          /* Skip the descriptor; its bytes are covered by the section hash. */
          if (kind === 0) r.varu32();
          else if (kind === 1) { r.u8(); r.limits(); }
          else if (kind === 2) r.limits();
          else if (kind === 3) { r.u8(); r.u8(); }
          else k = n; // unknown import kind: stop walking, hash still covers it
        }
        break;
      }
      case 5: {
        const n = r.varu32();
        for (let k = 0; k < n; k++) memories.push(r.limits());
        break;
      }
      case 7: {
        const n = r.varu32();
        for (let k = 0; k < n; k++) {
          const name = r.name();
          const kind = r.u8();
          const index = r.varu32();
          exports.push({ name, kind: EXTERN_KINDS[kind] ?? `kind-${kind}`, index });
        }
        break;
      }
      case 3: case 6: case 9: case 10: case 11:
        counts[section.name] = r.varu32();
        break;
      default:
        break;
    }

    sections.push(section);
    i = end;
  }

  return { version, sections, exports, imports, memories, tables, counts };
}

/*
 * sha256 of the whole module with the data section's path separators folded to
 * `/`. See the header for why this, and only this, is normalised.
 *
 * The blind spot, stated so nobody has to guess at it: a source change whose
 * ENTIRE binary effect is turning a `/` into a `\` (or back) inside a string
 * literal is invisible to this digest. Nothing else is.
 */
export function normalisedHash(buf, sections) {
  const copy = Buffer.from(buf);
  for (const s of sections) {
    if (s.id !== DATA_SECTION_ID) continue;
    for (let k = s.start; k < s.end; k++) {
      if (copy[k] === 0x5c) copy[k] = 0x2f;
    }
  }
  return createHash('sha256').update(copy).digest('hex');
}

export function rawHash(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

/*
 * Every distinct `<semver>+g<rev>` in the module. More than one means two
 * differently-stamped builds were somehow linked together, which is a defect in
 * its own right — so the caller demands exactly one rather than picking the
 * first and hoping.
 */
export function versionStamps(buf) {
  const text = buf.toString('latin1');
  const found = new Set();
  VERSION_STAMP.lastIndex = 0;
  for (const m of text.matchAll(VERSION_STAMP)) found.add(m[0]);
  return [...found].sort();
}

/*
 * The comparable projection. Deliberately excludes absolute file offsets (they
 * are implied by the ordered sizes) and includes everything a shell can observe:
 * the export table it calls through, the imports it must satisfy, and the memory
 * it shares.
 */
export function summarise(buf) {
  const m = parseWasm(buf);
  return {
    wasmVersion: m.version,
    byteLength: buf.length,
    sections: m.sections.map((s) => ({
      id: s.id,
      name: s.name,
      ...(s.customName === undefined ? {} : { customName: s.customName }),
      size: s.size,
    })),
    counts: m.counts,
    memories: m.memories,
    imports: [...m.imports].sort((a, b) =>
      `${a.module} ${a.name}`.localeCompare(`${b.module} ${b.name}`)),
    exports: [...m.exports].sort((a, b) => a.name.localeCompare(b.name)),
    versionStamps: versionStamps(buf),
    normalisedSha256: normalisedHash(buf, m.sections),
    rawSha256: rawHash(buf),
  };
}

/* CLI. `--rev` and `--stamp` exist so the workflow can feed the committed
 * artifact's own revision back into the rebuild via GROLOO_CORE_GIT_SHA. */
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  const file = args.find((a) => !a.startsWith('--'));
  if (!file) {
    console.error('usage: wasm-summary.mjs <module.wasm> [--rev | --stamp | --json]');
    process.exit(2);
  }
  const buf = readFileSync(file);
  if (args.includes('--rev') || args.includes('--stamp')) {
    const stamps = versionStamps(buf);
    if (stamps.length !== 1) {
      console.error(`expected exactly one version stamp in ${file}, found ${stamps.length}: ${stamps.join(', ') || '(none)'}`);
      process.exit(1);
    }
    const stamp = stamps[0];
    process.stdout.write(args.includes('--rev') ? stamp.split('+g')[1] : stamp);
    process.stdout.write('\n');
  } else {
    console.log(JSON.stringify(summarise(buf), null, 2));
  }
}
