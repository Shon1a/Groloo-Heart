/* tslint:disable */
/* eslint-disable */

/**
 * See [`groloo_core::api::addon_base_url`].
 */
export function addon_base_url(manifest_url: string): string;

/**
 * See [`groloo_core::api::addon_catalogs`].
 */
export function addon_catalogs(records_json: string): string;

/**
 * See [`groloo_core::api::addon_resource_path`]. `media_type` is the **wire**
 * vocabulary (`movie` / `series`), not the `/api/` one (`movie` / `tv`).
 */
export function addon_resource_path(resource: string, media_type: string, id: string): string;

/**
 * See [`groloo_core::api::catalog_metas`].
 */
export function catalog_metas(response_json: string): string;

/**
 * See [`groloo_core::api::continue_watching`].
 */
export function continue_watching(library_json: string, options_json: string): string;

/**
 * See [`groloo_core::api::core_constants`].
 */
export function core_constants(): string;

/**
 * See [`groloo_core::api::core_version`]. Returns a bare string, never an
 * envelope — it is the probe everything else is trusted relative to.
 *
 * NOTE for whoever reads this expecting it to be the load-time pin: it is not,
 * any more. The shell verifies the artifact by the **sha-256 of these bytes**,
 * recorded in the `manifest.json` beside the vendored `.wasm` (see
 * `scripts/build-wasm.mjs`), because a version string cannot detect the failure
 * it was guarding against — a stale or truncated artifact reports the version it
 * was compiled with quite happily. This stays exported as human-readable
 * provenance and as the value stamped into every envelope's `core` field.
 */
export function core_version(): string;

/**
 * See [`groloo_core::api::library_record_watch`].
 */
export function library_record_watch(library_json: string, item_json: string): string;

/**
 * See [`groloo_core::api::library_remove`].
 */
export function library_remove(library_json: string, id: string, now: number): string;

/**
 * See [`groloo_core::api::manifest_has_resource`]. `typ` is `""` for "any type"
 * — an empty string rather than `null`, so the signature stays plain `&str`.
 */
export function manifest_has_resource(manifest_json: string, resource: string, typ: string): string;

/**
 * See [`groloo_core::api::merge_library`]. `now` crosses as `f64` because
 * wasm-bindgen has no `u64` in the JS ABI; it is clamped, never trusted.
 */
export function merge_library(local_json: string, remote_json: string, now: number): string;

/**
 * See [`groloo_core::api::merge_official`].
 */
export function merge_official(inline_json: string, payload_json: string): string;

/**
 * See [`groloo_core::api::mylist_toggle`].
 */
export function mylist_toggle(library_json: string, item_json: string, now: number): string;

/**
 * See [`groloo_core::api::normalize_manifest_url`].
 */
export function normalize_manifest_url(raw: string): string;

/**
 * See [`groloo_core::api::official_payload_file`].
 */
export function official_payload_file(index_json: string): string;

/**
 * See [`groloo_core::api::order_langs`].
 */
export function order_langs(langs_json: string): string;

/**
 * See [`groloo_core::api::rank_streams`]. `caps_json` is the device profile the
 * shell probed; `"{}"` is the fully permissive one, and it is what a shell that
 * has not probed yet should send — an *empty* profile means "no constraint on
 * that axis", never "allow nothing".
 */
export function rank_streams(streams_json: string, caps_json: string): string;

/**
 * See [`groloo_core::api::reconcile_install_state`].
 */
export function reconcile_install_state(request_json: string): string;

/**
 * See [`groloo_core::api::resume_position`].
 */
export function resume_position(library_json: string, key: string): string;

/**
 * Runs on module instantiation, before the shell calls anything.
 *
 * The hook is the only thing standing between a core panic and a completely
 * opaque `RuntimeError: unreachable executed` in a living-room devtools console.
 * It cannot make the failure recoverable — nothing can, on this target — but it
 * makes it *reportable*, which is the difference between "the app silently
 * stopped syncing" and a bug report with a line number.
 */
export function start(): void;

/**
 * See [`groloo_core::api::stream_parse`]. `addon_name` is the label the shell
 * wants shown as the source; the core neither knows it nor guesses it.
 */
export function stream_parse(response_json: string, addon_name: string): string;

/**
 * See [`groloo_core::api::validate_manifest`].
 */
export function validate_manifest(manifest_json: string): string;

/**
 * See [`groloo_core::api::visible_rows`].
 */
export function visible_rows(rows_json: string, gating_json: string, config_json: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly addon_base_url: (a: number, b: number, c: number) => void;
    readonly addon_catalogs: (a: number, b: number, c: number) => void;
    readonly addon_resource_path: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly catalog_metas: (a: number, b: number, c: number) => void;
    readonly continue_watching: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly core_constants: (a: number) => void;
    readonly core_version: (a: number) => void;
    readonly library_record_watch: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly library_remove: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly manifest_has_resource: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly merge_library: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly merge_official: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly mylist_toggle: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly normalize_manifest_url: (a: number, b: number, c: number) => void;
    readonly official_payload_file: (a: number, b: number, c: number) => void;
    readonly order_langs: (a: number, b: number, c: number) => void;
    readonly rank_streams: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly reconcile_install_state: (a: number, b: number, c: number) => void;
    readonly resume_position: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly start: () => void;
    readonly stream_parse: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly validate_manifest: (a: number, b: number, c: number) => void;
    readonly visible_rows: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly __wbindgen_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export2: (a: number, b: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
