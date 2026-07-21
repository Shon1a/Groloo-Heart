//! The Stremio `stream` and `subtitles` wire types — a stream source as an add-on
//! actually sends it.
//!
//! Nothing in this crate modelled a stream before now. `addonClient.ts:91` had a
//! seven-field `RawStream` interface covering the subset one TypeScript app needs,
//! which is why a debrid add-on's `infoHash`, an add-on's `notWebReady` flag and
//! its `proxyHeaders` were all invisible to every decision the app made.
//!
//! ## Hand-ported, not vendored — and why
//!
//! `06-rust-core.md` §8 proposed vendoring `stremio-core`'s
//! `src/types/{addon,resource,streams}` + `src/addon_transport` (MIT, © 2019
//! SmartCode OOD). Read at
//! [`6555b464`](https://github.com/Stremio/stremio-core/commit/6555b464ac36ee71d39f4c91e775cfe99f809e29),
//! that premise does not survive the source. The licence is fine; the coupling is
//! not:
//!
//! - `src/types/resource/stream.rs:29` is
//!   `use crate::{runtime::EnvError, types::streams::ConvertedStreamSource}` — the
//!   1,332-line file that carries all the value is not separable from `runtime`,
//!   the module §8 explicitly excludes. It also reaches `types::streaming_server`
//!   and `types::torrent`, neither on the list.
//! - `src/addon_transport/http_transport/http_transport.rs:14` is
//!   `use crate::runtime::{Env, EnvError, EnvFutureExt, TryEnvFuture}`. The
//!   transport *is* the runtime's I/O abstraction, so vendoring it means adopting
//!   `Env` and `futures` — against §3's own rule that HTTP execution stays out.
//! - `manifest.rs`, `request.rs` and `stream.rs` all import `crate::constants`,
//!   one flat 149-line module that also carries `SCHEMA_VERSION = 25` and three
//!   `*.strem.io` URLs. Taking the types means taking that file or forking it.
//! - 20 new direct dependencies against this crate's two, and 183 lockfile entries
//!   against 23 — on an artifact a television downloads.
//! - `stremio-core/src/lib.rs:1` is `#![allow(clippy::all)]`, and `constants.rs`
//!   calls `.expect()` in a static initialiser. This crate denies `expect_used`
//!   and friends at the root, so vendored files would not compile here and the
//!   only ways out are forbidden by the ground rules.
//!
//! What §8 actually wanted from vendoring was "already debugged against the real
//! add-on population", and that value is in the **tolerance rules** — a page of
//! serde attributes — not in the 4,041 lines carrying them. Those are taken as
//! design and cited at each site below, at zero dependency cost. No code is
//! copied, so no MIT notice is owed on the Attributions screen for this module;
//! the reference is credited here because provenance of an *idea* is still worth
//! recording.
//!
//! ## Three deliberate divergences from upstream, each with a reason
//!
//! **1. `title` is its own field, not an alias of `description`.** Upstream is
//! `#[serde(default, alias = "title")] pub description` (`stream.rs:74`) — Stremio
//! treats them as one field. GROLOO does not: `addonClient.ts:94` builds its
//! label as `[name, title, description].filter(Boolean).join('\n')`, so a stream
//! setting both contributes both lines. Aliasing here would be worse than a
//! behaviour change — serde rejects a document that sets a field and its alias as
//! a *duplicate field*, so the whole stream would fail to parse. The twin rule
//! requires byte-identical output against the TypeScript, so both fields stay.
//! (Upstream's reading is arguably the correct one and the joined label
//! double-prints such a stream today; that is a divergence to declare in the
//! corpus and fix deliberately, not to smuggle in under a port.)
//!
//! **2. The source is flat optional fields, not an untagged enum.** Upstream
//! flattens an untagged `StreamSource` into `Stream`. Untagged means *first
//! matching variant wins* — and a debrid add-on that sends `url` **and**
//! `infoHash` together (the common case: a cached torrent with a direct link) is
//! matched as `Url` with the `infoHash` silently discarded. It also means one
//! unmodelled source shape fails the entire stream with an error message serde
//! renders as "Valid StreamSource". Flat fields keep every source the add-on sent,
//! never fail, and let [`Stream::source_kind`] state the precedence *out loud*.
//!
//! **3. Maps are `BTreeMap`, not `HashMap`.** Upstream's `HashMap` serialises in
//! an arbitrary order. Every output of this core is compared byte-for-byte by the
//! differential harness, so an unordered map is a flaky test waiting to happen.
//!
//! ## What is deliberately not modelled
//!
//! The archive sources (`rarUrls`, `zipUrls`, `7zipUrls`, `tarUrls`, `nzbUrl`),
//! `announce`/`sources` trackers and `fileMustInclude` are absent. Every one of
//! them exists to *open a transport*, and this core never opens one — the shell
//! plays the URL. They survive round-tripping anyway via
//! [`StreamBehaviorHints::other`]'s sibling behaviour only where the add-on put
//! them under `behaviorHints`; at top level they are dropped. If a shell ever
//! grows a torrent client, this is the paragraph that has to change, and
//! [`StreamSourceKind::Unknown`] is the arm those streams land in until it does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::de;

/// One stream source as an add-on's `stream` resource returns it.
///
/// Every field is optional or defaulted: a stream that omits everything but a URL
/// must parse, because most of them do. Unreadable *elements* are dropped by
/// [`StreamsResponse`] rather than failing the response — the same rule
/// [`crate::salvage`] applies everywhere else in this crate, and the same one
/// upstream spells `serde_with::VecSkipError` (`response.rs:3`).
///
/// ## The typing rule: coerce what is DISPLAYED, refuse what is DEREFERENCED
///
/// The wire here is a third party's JSON produced by untyped JavaScript, and the
/// row-level rule above has a hole one layer down: a field of the wrong type fails
/// the *record*, and `lenient_vec` then drops the record. "One bad row costs that
/// row" quietly became "one bad **field** costs the row", and on this type a row is
/// a playable source. That failure is invisible — the user sees a title with fewer
/// sources, or none, and there is nothing anywhere to say an add-on sent something
/// odd. It is the worst kind of bug to be handed in a report.
///
/// So every field states which of three things it is:
///
/// 1. **Displayed** — `name`, `title`, `description` are joined into the label.
///    JavaScript's `[…].filter(Boolean).join('\n')` stringifies whatever it is
///    given, so `name: 1080` labels the stream `"1080"` in the twin and the stream
///    plays. These coerce ([`de::opt_string_or_number`]), which is both the
///    byte-identical answer the twin rule needs and the one an add-on author
///    expects — a bare resolution or a release year written without quotes is the
///    single commonest way an untyped add-on hands over a number. **This is
///    U-coerce, fixed.**
/// 2. **Carried** — everything the core stores but never acts on. A wrong type
///    costs the field ([`de::optional`]), never the stream. `fileIdx: "2"` must not
///    delete a source the shell can play.
/// 3. **Dereferenced** — `url`, and only `url`. It is handed to a `<video>`
///    element, so a number there is not a lost hint, it is a source that appears in
///    the list and 404s. It stays strict and the stream is refused, which is the
///    already-declared divergence `D-typed-url` and the deliberate opposite of
///    rule 1: coercing `12345` to `"12345"` would resolve it against the page
///    origin and manufacture exactly the broken-looking source we are trying to
///    avoid.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    // ---- source ---------------------------------------------------------
    // Flat rather than an untagged enum; see the module doc, divergence 2.
    // `source_kind()` is the single place their precedence is decided.
    /// A directly playable URL — HTTP(S), an HLS manifest, or a data URI. The only
    /// source today's web shell can play, and the only one `mapAddonStream` looks
    /// at (`addonClient.ts:95`: a stream without one is dropped).
    ///
    /// **DEREFERENCED — deliberately strict.** See the type doc, rule 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A YouTube video id (upstream's `StreamSource::YouTube`).
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub yt_id: Option<String>,
    /// A BitTorrent infohash, hex, as the add-on spelled it — **not** normalised.
    /// Upstream parses it to `[u8; 20]` via `stremio_serde_hex`; a stream whose
    /// hash is malformed or oddly-cased would fail there, and here it costs
    /// nothing because the core never dials a swarm.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub info_hash: Option<String>,
    /// Which file inside the torrent/archive. Meaningless without a source that
    /// contains files; carried so a shell that has a torrent client can use it.
    ///
    /// The clearest case for rule 2 in the type doc: it is `u16`, so `fileIdx: 0.5`
    /// and `fileIdx: 70000` both fail the field, and neither is a reason to delete
    /// a source the shell can play.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_idx: Option<u16>,
    /// A link to open outside the player (a web page, a store entry).
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub external_url: Option<String>,
    /// An iframe-embedded player.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub player_frame_url: Option<String>,

    // ---- presentation ---------------------------------------------------
    // DISPLAYED — the label triple. See the type doc, rule 1 (U-coerce).
    /// Usually the add-on's own name or a short quality tag.
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// The release title. Its own field, not an alias of `description` — see the
    /// module doc, divergence 1.
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<String>,
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Carried, not displayed by this core — the shell renders it if it wants one.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub thumbnail: Option<String>,

    /// Subtitle tracks the add-on ships alongside this stream. One unreadable
    /// track costs that track — upstream's `DefaultOnNull<VecSkipError<_>>`
    /// (`stream.rs:81`), which this crate already had as
    /// [`crate::types::de::lenient_vec`].
    ///
    /// `Option`, not a bare `Vec`, because **an absent array and an empty array are
    /// different answers** and `addonClient.ts:109` branches on exactly that
    /// (`Array.isArray(s.subtitles) ? … : undefined`): an add-on that sent `[]`
    /// gets `"subtitles":[]` in the mapped record, one that sent nothing gets no
    /// key at all. Collapsing the two would be invisible in Rust and a byte
    /// difference at the boundary.
    #[serde(
        default,
        deserialize_with = "de::array_or_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub subtitles: Option<Vec<Subtitle>>,

    /// `null` here means "the defaults", not a parse error — upstream's
    /// `DefaultOnNull` (`stream.rs:83`). So does anything else that is not an
    /// object: `behaviorHints: "hls"` costs the hints, never the stream, which is
    /// what `s.behaviorHints?.streamType` does in the twin.
    #[serde(
        default,
        deserialize_with = "de::default_on_error",
        skip_serializing_if = "is_default"
    )]
    pub behavior_hints: StreamBehaviorHints,
}

impl Stream {
    /// Which kind of source this stream actually offers, by a **stated
    /// precedence**: a directly playable URL first, then YouTube, then a torrent,
    /// then an external link, then an embedded frame.
    ///
    /// The order is "what the most shells can play", which is also the order
    /// `mapAddonStream` implies by looking at nothing but `url`. Keeping it as one
    /// documented function — rather than an untagged enum's variant order, which
    /// is the same rule expressed where nobody reads it — is what makes it
    /// reviewable when a fourth shell disagrees.
    ///
    /// [`StreamSourceKind::Unknown`] is a *classification*, never a failure: an
    /// archive source, an `nzbUrl`, or a shape invented after this was written all
    /// land there with the stream intact. A ranker may then block it with a
    /// reason. Dropping it instead would be the MSE over-rejection bug with the
    /// sign flipped.
    pub fn source_kind(&self) -> StreamSourceKind {
        if non_empty(&self.url) {
            StreamSourceKind::Url
        } else if non_empty(&self.yt_id) {
            StreamSourceKind::YouTube
        } else if non_empty(&self.info_hash) {
            StreamSourceKind::Torrent
        } else if non_empty(&self.external_url) {
            StreamSourceKind::External
        } else if non_empty(&self.player_frame_url) {
            StreamSourceKind::PlayerFrame
        } else {
            StreamSourceKind::Unknown
        }
    }

    /// True when this stream carries a URL a `<video>` element could be pointed at.
    /// Exactly `addonClient.ts:96`'s `if (!url) return null`.
    pub fn is_directly_playable(&self) -> bool {
        self.source_kind() == StreamSourceKind::Url
    }
}

/// How a [`Stream`] delivers its bytes. Derived from which fields are populated
/// (see [`Stream::source_kind`]); never deserialised directly, so an add-on cannot
/// mislabel itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamSourceKind {
    Url,
    YouTube,
    Torrent,
    External,
    PlayerFrame,
    /// A source shape this core does not model — an archive, an NZB, or something
    /// newer. The stream is kept; only its playability is unknown.
    Unknown,
}

/// One subtitle track.
///
/// Field order is `url`, `lang`, then the optional pair, and that is not
/// cosmetic: with `id`/`label` absent this serialises as `{"url":…,"lang":…}`,
/// byte-identical to what `addonClient.ts:109`
/// (`.map(x => ({ url: x.url || '', lang: x.lang || '' }))`) emits. The twin rule
/// needs those bytes to match before the TypeScript can go.
///
/// `url` and `lang` default to `""` rather than being required, which is a
/// divergence from upstream's mandatory `id`/`lang`/`url` (`subtitles.rs`) and
/// matches the TypeScript's `|| ''`. The empty-`url` track is filtered out at the
/// mapping step, not here — dropping it during deserialisation would make the
/// count of tracks an add-on sent unreportable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Subtitle {
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub url: String,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub lang: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `behaviorHints` — the add-on SDK's out-of-band flags.
///
/// Upstream models the seven documented fields plus a `#[serde(flatten)] other`
/// catch-all; both are kept, because the catch-all is what stops an add-on's
/// vendor extension from being deleted on the way through. Two fields are GROLOO
/// additions with no upstream counterpart, and both are load-bearing today:
/// [`Self::stream_type`] and [`Self::lang`], read at `addonClient.ts:98` and
/// `:100`.
///
/// **Nothing in here is worth a source.** Every field is `de::optional` /
/// `de::default_on_error` — rule 2 of [`Stream`]'s typing rule — because these are
/// the fields an add-on author is least careful with (`videoSize: "9.2 GB"` is a
/// real thing add-ons send) and the twin reads them through `?.`, which cannot
/// fail. A wrong type costs the hint. It used to cost the whole stream, and then
/// the whole rank call, since [`crate::rank::RankCandidate`] deserialises this same
/// type from a plain `Vec` — one add-on's stray string emptied the entire source
/// list with `ok:false` and no indication of which stream did it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamBehaviorHints {
    /// The add-on's own warning that a browser cannot play this directly. A
    /// *self-declared* hint, and the only honest one — everything else a ranker
    /// knows about compatibility it has to infer from free text.
    #[serde(
        default,
        deserialize_with = "de::default_on_error",
        skip_serializing_if = "is_default"
    )]
    pub not_web_ready: bool,
    /// Streams sharing a binge group should auto-play consecutively from the same
    /// source.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub binge_group: Option<String>,
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub country_whitelist: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub proxy_headers: Option<StreamProxyHeaders>,
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub filename: Option<String>,
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub video_hash: Option<String>,
    /// Size in bytes. A *stated* size, unlike the `"2.3 GB"` a ranker scrapes out
    /// of a release title — worth preferring wherever both exist.
    ///
    /// Not coerced from a string, unlike the label triple: this one is *arithmetic*,
    /// and `"9.2 GB"` is not a byte count. Losing it falls back to the scraped size,
    /// which is what a stream that never stated one gets anyway.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub video_size: Option<u64>,

    /// GROLOO extension, no upstream equivalent: `'hls'` marks an HLS manifest.
    /// `addonClient.ts:98` checks this *before* falling back to sniffing the URL
    /// for `.m3u8`, so it is the authoritative signal and must survive the port.
    ///
    /// A non-string here degrades to `None`, which reaches the same answer the twin
    /// does: `5 === 'hls'` is false, so the URL sniff decides.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub stream_type: Option<String>,
    /// GROLOO extension, no upstream equivalent: an explicit audio language code.
    /// `addonClient.ts:100` gives it precedence over any flag emoji scraped from
    /// the label, so it too must survive.
    ///
    /// Not coerced either: this is a *language code*, and the twin's truthiness
    /// test would put the literal number into the `langs` array, where it matches
    /// no tab and no subtitle track. Falling through to the flags in the label is
    /// the useful answer.
    #[serde(
        default,
        deserialize_with = "de::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub lang: Option<String>,

    /// Everything else the add-on put here, preserved verbatim.
    ///
    /// `BTreeMap` rather than upstream's `HashMap` so serialisation is
    /// deterministic — the differential harness compares bytes, and an unordered
    /// map is a test that fails one run in six for no reason.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, serde_json::Value>,
}

/// Per-request/per-response headers an add-on wants the player to send or expect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StreamProxyHeaders {
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub request: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub response: BTreeMap<String, String>,
}

/// The body of an add-on's `stream/{type}/{id}.json` response.
///
/// One unreadable stream costs that stream, not the response — this is the type
/// where upstream's `VecSkipError` rule (`response.rs:3`) earns its keep, because
/// a single add-on returning one malformed entry would otherwise leave a title
/// with no sources at all and nothing to say why.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StreamsResponse {
    #[serde(default, deserialize_with = "de::lenient_vec")]
    pub streams: Vec<Stream>,
}

// ===========================================================================
// The mapping layer — `addonClient.ts:65-111`, ported
// ===========================================================================
//
// Everything below turns the wire types above into the single flat record the UI
// renders. It is deliberately a *port* and not an improvement: the twin rule says
// the TypeScript cannot be deleted until a corpus proves these bytes identical, so
// every quirk of the original is reproduced on purpose and the ones worth fixing
// are marked `DIVERGENCE CANDIDATE` in a comment rather than fixed here.

/// One playable source as the UI renders it — the output of [`map_addon_stream`],
/// and `addonClient.ts:10-19`'s `AddonStream` field for field.
///
/// Field ORDER is part of the contract, not cosmetics: this serialises to
/// `{"source","label","quality","size","kind","url","langs","subtitles"}`, which is
/// the order the JavaScript object literal at `addonClient.ts:101-110` produces,
/// and the differential harness compares bytes.
///
/// Two fields have deliberately different absence rules, both matching the twin:
/// `size` is `string | null` and is **always present** (`extractSize` returns
/// `null`); `subtitles` is omitted entirely when the add-on sent no array at all
/// (`: undefined`, which `JSON.stringify` drops) but is `[]` when it sent an array
/// whose every entry was unusable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AddonStream {
    /// The add-on's display name — which add-on offered this source.
    pub source: String,
    /// `[name, title, description]` joined with `\n`, or `"Source"` when the
    /// add-on labelled the stream with nothing at all.
    pub label: String,
    /// `"4K"` | `"1080p"` | `"720p"` | `"480p"` | `""`. See [`detect_quality`].
    pub quality: String,
    /// `"2.3 GB"`, or `null`. See [`extract_size`].
    pub size: Option<String>,
    /// `"hls"` or `"url"` — which player path the shell takes.
    pub kind: String,
    pub url: String,
    /// Never empty: falls back to `["en"]`. See [`map_addon_stream`].
    pub langs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<Vec<Subtitle>>,
}

/// Country code → language code for the flag emoji an add-on puts in a title.
/// `addonClient.ts:78`, all 22 entries, same order.
const FLAG_LANG: [(&str, &str); 22] = [
    ("GB", "en"),
    ("US", "en"),
    ("AU", "en"),
    ("CA", "en"),
    ("IE", "en"),
    ("NZ", "en"),
    ("RU", "ru"),
    ("UA", "uk"),
    ("GE", "ka"),
    ("FR", "fr"),
    ("DE", "de"),
    ("IT", "it"),
    ("ES", "es"),
    ("MX", "es"),
    ("PT", "pt"),
    ("BR", "pt"),
    ("JP", "ja"),
    ("KR", "ko"),
    ("CN", "zh"),
    ("TR", "tr"),
    ("PL", "pl"),
    ("NL", "nl"),
];

/// The vertical resolution a release title advertises, or `None` when it
/// advertises none.
///
/// This is the whole of `addonClient.ts:65-72` plus a `360` arm, and it is the
/// **only** producer of a resolution anywhere in the core —
/// [`detect_quality`] is derived from it so the label the UI prints and the number
/// the ranker sorts on can never disagree.
///
/// The matching rules are the TypeScript's, quirks included, because a corpus has
/// to pass before the TypeScript can go:
///
/// - `2160`, `1080`, `720`, `480` are **bare substring** tests, so `"10800"`
///   matches `1080` and `"114800"` matches `480`. Wrong, and preserved.
/// - `4k` alone is word-bounded (`\b4k\b`), so `"x4k"` does not match, and the
///   boundary is JavaScript's ASCII `\w` — `"ე4k"` *does* match because `ე` is not
///   a word character to a non-unicode regex.
/// - first match wins, in this order — a `"2160p ... 1080p"` label is 2160.
///
/// `360` is appended last, so it cannot change any answer the twin gave; it exists
/// because a ranker that reads `None` for a 360p stream would rank it above a
/// known 480p one on the "unknown never blocks" rule.
pub fn detect_resolution(text: &str) -> Option<u32> {
    let t = text.to_lowercase();
    if t.contains("2160") || t.contains("uhd") || has_word_4k(&t) {
        return Some(2160);
    }
    for (needle, height) in [("1080", 1080_u32), ("720", 720), ("480", 480), ("360", 360)] {
        if t.contains(needle) {
            return Some(height);
        }
    }
    None
}

/// `\b4k\b` against JavaScript's ASCII `\w` class.
fn has_word_4k(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    lower.match_indices("4k").any(|(i, _)| {
        let before_ok = match i.checked_sub(1).and_then(|j| bytes.get(j)) {
            Some(&c) => !is_js_word_byte(c),
            None => true,
        };
        let after_ok = match i.checked_add(2).and_then(|j| bytes.get(j)) {
            Some(&c) => !is_js_word_byte(c),
            None => true,
        };
        before_ok && after_ok
    })
}

/// JavaScript's `\w` without the `u` flag: `[A-Za-z0-9_]`, ASCII only.
fn is_js_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The quality badge the UI prints — `addonClient.ts:65`, exactly.
///
/// `""` when nothing was recognised, which is also what `qualityRank` scores 0.
/// Derived from [`detect_resolution`] so there is one table, not two; `360p`
/// resolves to `""` because the twin has no such badge.
pub fn detect_quality(text: &str) -> &'static str {
    match detect_resolution(text) {
        Some(2160) => "4K",
        Some(1080) => "1080p",
        Some(720) => "720p",
        Some(480) => "480p",
        _ => "",
    }
}

/// The human-readable size a release title advertises — `"2.3 GB"`, `"700 MB"` —
/// or `None`.
///
/// `addonClient.ts:73-76`'s `/(\d+(?:\.\d+)?)\s?(gb|mb)/i`, hand-rolled: leftmost
/// match, at most one whitespace character between number and unit, unit
/// uppercased, a single space inserted regardless of what the original used.
///
/// The number is returned as the add-on wrote it (`"2.3"`, not `2.3`) because this
/// value is *displayed*, never compared. [`crate::rank`] parses it into bytes
/// separately, and prefers `behaviorHints.videoSize` when the add-on stated one.
pub fn extract_size(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();

    for (start, _) in lower.char_indices() {
        let Some(&first) = bytes.get(start) else {
            continue;
        };
        if !first.is_ascii_digit() {
            continue;
        }
        // `\d+` is greedy and needs no backtracking here: a shorter run would
        // leave a digit next, and a digit can never begin `\.`, `\s` or `gb|mb`.
        let mut i = start;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i = i.saturating_add(1);
        }
        // `(?:\.\d+)?` then `\s?`, both greedy, both retried without.
        for want_fraction in [true, false] {
            let mut j = i;
            if want_fraction {
                if bytes.get(j) != Some(&b'.') {
                    continue;
                }
                let after_dot = j.saturating_add(1);
                let mut k = after_dot;
                while bytes.get(k).is_some_and(u8::is_ascii_digit) {
                    k = k.saturating_add(1);
                }
                if k == after_dot {
                    continue;
                }
                j = k;
            }
            let number = lower.get(start..j).unwrap_or("");
            for skip_space in [true, false] {
                let mut u = j;
                if skip_space {
                    match lower.get(u..).and_then(|s| s.chars().next()) {
                        Some(c) if is_js_space(c) => u = u.saturating_add(c.len_utf8()),
                        _ => continue,
                    }
                }
                for unit in ["gb", "mb"] {
                    if lower.get(u..).is_some_and(|s| s.starts_with(unit)) {
                        return Some(format!("{number} {}", unit.to_ascii_uppercase()));
                    }
                }
            }
        }
    }
    None
}

/// JavaScript's `\s`. `char::is_whitespace` covers all of it except the BOM, which
/// JS includes and Unicode does not classify as whitespace.
pub(crate) fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// Language codes implied by the regional-indicator flag emoji in a label —
/// `addonClient.ts:79-89`.
///
/// The scan reproduces `RegExp.prototype.exec` with the `g` flag, and that is
/// load-bearing rather than pedantic: the match is **non-overlapping and
/// left-to-right**, so three consecutive indicators yield ONE language (the first
/// pair is consumed, the third indicator is left with no partner) — a "find all the
/// flags" rewrite would find two and quietly change what a common
/// `🇬🇧🇺🇸🇷🇺`-style label means.
///
/// An unmapped country code degrades to its own lowercased letters (`🇸🇪` → `se`),
/// so a language the table does not know is never dropped. Order is first-seen and
/// duplicates are removed.
pub fn parse_stream_langs(text: &str) -> Vec<String> {
    const FIRST: u32 = 0x1F1E6;
    let is_indicator = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);

    let mut out: Vec<String> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !is_indicator(c) {
            continue;
        }
        // A lone indicator fails the match and the regex advances by one; only a
        // PAIR consumes two.
        if !chars.peek().copied().is_some_and(is_indicator) {
            continue;
        }
        let Some(d) = chars.next() else { break };
        let Some(cc) = country_code(c, d, FIRST) else {
            continue;
        };
        let lang = FLAG_LANG
            .iter()
            .find(|(code, _)| *code == cc)
            .map_or_else(|| cc.to_lowercase(), |(_, l)| (*l).to_string());
        if !lang.is_empty() && !out.contains(&lang) {
            out.push(lang);
        }
    }
    out
}

/// Two regional indicators → the two-letter country code they spell.
fn country_code(a: char, b: char, first: u32) -> Option<String> {
    let letter = |c: char| -> Option<char> {
        let offset = u32::from(c).checked_sub(first)?;
        char::from_u32(offset.checked_add(u32::from('A'))?)
    };
    Some(format!("{}{}", letter(a)?, letter(b)?))
}

/// Turn one wire [`Stream`] into the flat record the UI renders, or `None` when it
/// carries no directly playable URL — `addonClient.ts:93-111`.
///
/// The rules, in the order the twin applies them:
///
/// 1. `label` = `[name, title, description]`, empties dropped, joined with `\n`.
///    The **raw** label feeds quality/size/flag detection; the `"Source"` default
///    is applied only to what is displayed. A numeric field is stringified on the
///    way in ([`Stream`]'s typing rule 1) exactly as `join` stringifies it, so
///    `name: 1080` labels the stream `"1080"` here and in the twin.
/// 2. no `url` → `None`. A torrent-only or YouTube-only stream is discarded here,
///    which is the shell's real limitation and not a judgement — see
///    [`Stream::source_kind`] for what the core still knows about it.
/// 3. HLS if `behaviorHints.streamType == "hls"` (exact, case-sensitive) **or**
///    the URL ends a path segment in `.m3u8`/`.txt` **or** contains `/hls/`.
/// 4. languages: an explicit `behaviorHints.lang` wins outright; else the flags in
///    the label; else `["en"]` — a guess, and the only one in this function.
/// 5. subtitles: mapped and filtered to those with a URL, but only when the add-on
///    sent an array. No array at all is a different value from an empty one.
///
/// DIVERGENCE CANDIDATE (do not fix here): step 1 joins `title` and `description`,
/// and Stremio's own schema treats `title` as an *alias* of `description`
/// (`stremio-core/src/types/resource/stream.rs:74`). An add-on that sets both —
/// legal, and common among the ones that migrated — has its release name printed
/// twice. Fixing that changes bytes, so it belongs in the corpus as a declared
/// divergence, not in a port.
pub fn map_addon_stream(s: &Stream, addon_name: &str) -> Option<AddonStream> {
    let label = [&s.name, &s.title, &s.description]
        .into_iter()
        .filter_map(|v| v.as_deref().filter(|t| !t.is_empty()))
        .collect::<Vec<_>>()
        .join("\n");

    let url = s.url.clone().filter(|u| !u.is_empty())?;

    let bh = &s.behavior_hints;
    let is_hls = bh.stream_type.as_deref() == Some("hls") || url_looks_like_hls(&url);

    let langs = match bh.lang.as_deref().filter(|l| !l.is_empty()) {
        Some(l) => vec![l.to_string()],
        None => {
            let flags = parse_stream_langs(&label);
            if flags.is_empty() {
                vec!["en".to_string()]
            } else {
                flags
            }
        }
    };

    Some(AddonStream {
        source: addon_name.to_string(),
        quality: detect_quality(&label).to_string(),
        size: extract_size(&label),
        kind: if is_hls { "hls" } else { "url" }.to_string(),
        url,
        langs,
        subtitles: s.subtitles.as_ref().map(|list| {
            list.iter()
                // `id`/`label` are dropped, not carried: the twin maps to exactly
                // `{url, lang}` and the harness compares bytes. They survive on the
                // wire type for a shell that wants them.
                .map(|t| Subtitle {
                    url: t.url.clone(),
                    lang: t.lang.clone(),
                    id: None,
                    label: None,
                })
                .filter(|t| !t.url.is_empty())
                .collect()
        }),
        label: if label.is_empty() {
            "Source".to_string()
        } else {
            label
        },
    })
}

/// `addonClient.ts:98`'s two URL sniffs: `/\.(m3u8|txt)(\?|$)/i` and `/\/hls\//i`.
fn url_looks_like_hls(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.contains("/hls/") {
        return true;
    }
    [".m3u8", ".txt"].into_iter().any(|ext| {
        lower.match_indices(ext).any(|(i, _)| {
            match i
                .checked_add(ext.len())
                .and_then(|j| lower.get(j..))
                .and_then(|rest| rest.chars().next())
            {
                None => true,
                Some(c) => c == '?',
            }
        })
    })
}

/// Map a whole `stream/{type}/{id}.json` response — `addonClient.ts:122`'s
/// `(data.streams || []).map(mapAddonStream).filter(Boolean)`.
///
/// One unreadable stream costs that stream (see [`StreamsResponse`]) and one
/// stream without a URL costs that stream; neither costs the response. That is the
/// same rule twice, at two different layers, and it is why an add-on with a single
/// broken entry does not present as "this title has no sources".
pub fn map_addon_streams(response: &StreamsResponse, addon_name: &str) -> Vec<AddonStream> {
    response
        .streams
        .iter()
        .filter_map(|s| map_addon_stream(s, addon_name))
        .collect()
}

/// `Option<String>` that is present and not empty. `Some("")` is how JSON says
/// "this field exists but means nothing", and the TypeScript's `s.url || ''`
/// treats it exactly this way.
fn non_empty(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

/// `skip_serializing_if` for any defaulted field — keeps absent things absent
/// rather than writing `false`/`{}` into every record.
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_only_stream_parses() {
        let s: Stream = serde_json::from_str(r#"{"url":"https://a.co/v.mp4"}"#).unwrap();
        assert_eq!(s.url.as_deref(), Some("https://a.co/v.mp4"));
        assert_eq!(s.source_kind(), StreamSourceKind::Url);
        assert!(s.is_directly_playable());
        assert_eq!(s.subtitles, None);
        assert_eq!(s.behavior_hints, StreamBehaviorHints::default());
    }

    /// The `null` sweep. Upstream needs `DefaultOnNull` on three separate fields
    /// for this; a browser-authored payload is full of explicit nulls.
    #[test]
    fn explicit_nulls_mean_the_default() {
        let s: Stream = serde_json::from_str(
            r#"{"url":"https://a.co/v.mp4","name":null,"title":null,"description":null,
                "thumbnail":null,"subtitles":null,"behaviorHints":null}"#,
        )
        .unwrap();
        assert_eq!(s.name, None);
        // `null` is not an array, so it is "no list" — which is what the twin's
        // `Array.isArray` says too.
        assert_eq!(s.subtitles, None);
        assert_eq!(s.behavior_hints, StreamBehaviorHints::default());
    }

    /// The distinction the `Option` exists for. Absent, `null` and a non-array all
    /// mean "no list"; `[]` means "a list, and it is empty".
    #[test]
    fn an_empty_subtitle_array_is_not_the_same_as_no_array() {
        let subs = |json: &str| serde_json::from_str::<Stream>(json).unwrap().subtitles;
        assert_eq!(subs(r#"{"url":"u"}"#), None);
        assert_eq!(subs(r#"{"url":"u","subtitles":null}"#), None);
        // A non-array costs the field, never the stream.
        assert_eq!(subs(r#"{"url":"u","subtitles":"none"}"#), None);
        assert_eq!(subs(r#"{"url":"u","subtitles":[]}"#), Some(Vec::new()));
    }

    /// Divergence 1, pinned. Upstream aliases `title` onto `description`, which
    /// would make this document a duplicate-field error; GROLOO's label builder
    /// needs both, so both survive.
    #[test]
    fn title_and_description_are_separate_fields() {
        let s: Stream =
            serde_json::from_str(r#"{"url":"u","name":"N","title":"T","description":"D"}"#)
                .unwrap();
        assert_eq!(s.name.as_deref(), Some("N"));
        assert_eq!(s.title.as_deref(), Some("T"));
        assert_eq!(s.description.as_deref(), Some("D"));
    }

    /// Divergence 2, pinned. This is the debrid case: an untagged enum would bind
    /// the first matching variant and throw the other source away.
    #[test]
    fn a_stream_may_carry_more_than_one_source() {
        let s: Stream =
            serde_json::from_str(r#"{"url":"https://dbrid.co/x","infoHash":"abc123","fileIdx":2}"#)
                .unwrap();
        assert_eq!(s.url.as_deref(), Some("https://dbrid.co/x"));
        assert_eq!(s.info_hash.as_deref(), Some("abc123"));
        assert_eq!(s.file_idx, Some(2));
        // Precedence: the playable URL wins, but nothing was lost.
        assert_eq!(s.source_kind(), StreamSourceKind::Url);
    }

    #[test]
    fn source_kind_follows_the_stated_precedence() {
        let kind = |json: &str| serde_json::from_str::<Stream>(json).unwrap().source_kind();
        assert_eq!(kind(r#"{"ytId":"dQw4"}"#), StreamSourceKind::YouTube);
        assert_eq!(kind(r#"{"infoHash":"ab"}"#), StreamSourceKind::Torrent);
        assert_eq!(
            kind(r#"{"externalUrl":"https://a.co"}"#),
            StreamSourceKind::External
        );
        assert_eq!(
            kind(r#"{"playerFrameUrl":"https://a.co"}"#),
            StreamSourceKind::PlayerFrame
        );
        // An archive source is unmodelled — classified, not rejected.
        assert_eq!(
            kind(r#"{"rarUrls":["https://a.co/a.rar"]}"#),
            StreamSourceKind::Unknown
        );
        assert_eq!(kind(r#"{}"#), StreamSourceKind::Unknown);
        // An empty string is not a source.
        assert_eq!(kind(r#"{"url":""}"#), StreamSourceKind::Unknown);
        assert!(!serde_json::from_str::<Stream>(r#"{"url":""}"#)
            .unwrap()
            .is_directly_playable());
    }

    #[test]
    fn behavior_hints_keep_both_the_known_and_the_unknown() {
        let s: Stream = serde_json::from_str(
            r#"{"url":"u","behaviorHints":{"notWebReady":true,"bingeGroup":"g",
                "videoSize":123456789,"filename":"f.mkv","streamType":"hls","lang":"ka",
                "proxyHeaders":{"request":{"Referer":"https://a.co"}},
                "vendorThing":{"deep":1}}}"#,
        )
        .unwrap();
        let bh = &s.behavior_hints;
        assert!(bh.not_web_ready);
        assert_eq!(bh.binge_group.as_deref(), Some("g"));
        assert_eq!(bh.video_size, Some(123_456_789));
        assert_eq!(bh.stream_type.as_deref(), Some("hls"));
        assert_eq!(bh.lang.as_deref(), Some("ka"));
        assert_eq!(
            bh.proxy_headers
                .as_ref()
                .unwrap()
                .request
                .get("Referer")
                .map(String::as_str),
            Some("https://a.co")
        );
        // The vendor extension is preserved rather than dropped.
        assert!(bh.other.contains_key("vendorThing"));
        assert!(!bh.other.contains_key("notWebReady"));
    }

    /// Divergence 3, pinned: `other` must serialise in a stable order or the
    /// differential harness compares bytes against a coin flip.
    #[test]
    fn unknown_behavior_hints_serialise_deterministically() {
        let s: Stream =
            serde_json::from_str(r#"{"url":"u","behaviorHints":{"z":1,"a":2,"m":3}}"#).unwrap();
        let once = serde_json::to_string(&s).unwrap();
        assert_eq!(once, r#"{"url":"u","behaviorHints":{"a":2,"m":3,"z":1}}"#);
        for _ in 0..8 {
            assert_eq!(serde_json::to_string(&s).unwrap(), once);
        }
    }

    #[test]
    fn subtitles_tolerate_missing_halves_and_serialise_like_the_twin() {
        let s: Stream = serde_json::from_str(
            r#"{"url":"u","subtitles":[{"url":"https://a.co/s.srt","lang":"en"},
                                       {"lang":"ka"},
                                       {"url":"https://a.co/t.srt"}]}"#,
        )
        .unwrap();
        let subs = s.subtitles.unwrap();
        assert_eq!(subs.len(), 3);
        assert_eq!(subs[1].url, "");
        assert_eq!(subs[2].lang, "");
        assert_eq!(
            serde_json::to_string(&subs[0]).unwrap(),
            r#"{"url":"https://a.co/s.srt","lang":"en"}"#
        );
    }

    #[test]
    fn subtitles_keep_the_optional_protocol_fields_when_present() {
        let t: Subtitle = serde_json::from_str(
            r#"{"id":"sub1","url":"https://a.co/s.vtt","lang":"eng","label":"English (SDH)"}"#,
        )
        .unwrap();
        assert_eq!(t.id.as_deref(), Some("sub1"));
        assert_eq!(t.label.as_deref(), Some("English (SDH)"));
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"url":"https://a.co/s.vtt","lang":"eng","id":"sub1","label":"English (SDH)"}"#
        );
    }

    /// One bad track costs that track. Upstream: `VecSkipError`.
    #[test]
    fn one_unreadable_subtitle_does_not_cost_the_others() {
        let s: Stream = serde_json::from_str(
            r#"{"url":"u","subtitles":[{"url":"a","lang":"en"},"nonsense",{"url":"b","lang":"ka"}]}"#,
        )
        .unwrap();
        let urls: Vec<&str> = s
            .subtitles
            .iter()
            .flatten()
            .map(|t| t.url.as_str())
            .collect();
        assert_eq!(urls, vec!["a", "b"]);
    }

    /// The same rule one level up: an add-on returning one malformed stream must
    /// not cost a title every other source it had.
    #[test]
    fn one_unreadable_stream_does_not_cost_the_response() {
        let r: StreamsResponse =
            serde_json::from_str(r#"{"streams":[{"url":"a"},42,"nonsense",{"url":"c"}]}"#).unwrap();
        let urls: Vec<&str> = r.streams.iter().filter_map(|s| s.url.as_deref()).collect();
        assert_eq!(urls, vec!["a", "c"]);
    }

    /// **U-coerce, fixed.** `name: 1080` is what an untyped add-on sends when it
    /// labels a stream with a bare resolution, and the twin renders it: the label
    /// is `[name, title, description].filter(Boolean).join('\n')`, which
    /// stringifies. The core used to fail the field, fail the `Stream`, and have
    /// `StreamsResponse` drop it — the source disappeared and the title looked as
    /// though the add-on had nothing for it.
    ///
    /// Coercion is also what the twin rule needs: these bytes have to match
    /// `mapAddonStream`'s before it can be deleted.
    #[test]
    fn a_numeric_label_field_is_stringified_not_fatal() {
        let s: Stream =
            serde_json::from_str(r#"{"url":"https://a.co/v.mp4","name":1080,"title":2019}"#)
                .unwrap();
        assert_eq!(s.name.as_deref(), Some("1080"));
        assert_eq!(s.title.as_deref(), Some("2019"));

        // Through the mapper, which is where it is observable: the label reads as
        // JavaScript would have written it, and the detectors then see `1080`.
        let m = mapped(r#"{"url":"https://a.co/v.mp4","name":1080,"description":720}"#).unwrap();
        assert_eq!(m.label, "1080\n720");
        assert_eq!(m.quality, "1080p");

        // A whole number arriving as a float is `1080`, not `1080.0` — `String(x)`
        // in JavaScript, which is what the twin's `join` calls.
        assert_eq!(
            mapped(r#"{"url":"u","name":1080.0}"#).unwrap().label,
            "1080"
        );

        // And the source survives the response, which is the whole point.
        let r: StreamsResponse =
            serde_json::from_str(r#"{"streams":[{"name":5,"url":"a"},{"name":"good","url":"b"}]}"#)
                .unwrap();
        let urls: Vec<&str> = r.streams.iter().filter_map(|s| s.url.as_deref()).collect();
        assert_eq!(urls, vec!["a", "b"], "the numeric-name stream was dropped");
    }

    /// The other half of the same defect: **a field of the wrong type costs the
    /// field, never the stream** — for every field the core carries but does not
    /// dereference. Each entry here is a source that used to vanish out of a
    /// perfectly good response because of a value nothing ever reads.
    #[test]
    fn a_malformed_carried_field_costs_the_field_not_the_stream() {
        let cases = [
            r#"{"url":"a","fileIdx":"2"}"#,
            r#"{"url":"a","fileIdx":70000}"#,
            r#"{"url":"a","infoHash":12345}"#,
            r#"{"url":"a","thumbnail":{"src":"x"}}"#,
            r#"{"url":"a","ytId":42}"#,
            r#"{"url":"a","externalUrl":[1]}"#,
            r#"{"url":"a","behaviorHints":{"videoSize":"9.2 GB"}}"#,
            r#"{"url":"a","behaviorHints":{"notWebReady":"yes"}}"#,
            r#"{"url":"a","behaviorHints":{"countryWhitelist":"us"}}"#,
            r#"{"url":"a","behaviorHints":{"proxyHeaders":[]}}"#,
            r#"{"url":"a","behaviorHints":{"lang":5}}"#,
            r#"{"url":"a","behaviorHints":{"streamType":5}}"#,
            // Not even an object where one belongs — `s.behaviorHints?.x` shrugs.
            r#"{"url":"a","behaviorHints":"hls"}"#,
        ];
        for json in cases {
            let s: Stream =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} must parse: {e}"));
            assert_eq!(s.url.as_deref(), Some("a"), "{json}");
            assert!(s.is_directly_playable(), "{json}");
        }

        // The field is gone, not guessed at.
        let s: Stream =
            serde_json::from_str(r#"{"url":"a","fileIdx":"2","behaviorHints":{"videoSize":"9.2 GB","streamType":"hls"}}"#)
                .unwrap();
        assert_eq!(s.file_idx, None);
        assert_eq!(s.behavior_hints.video_size, None);
        // ...and its readable siblings are untouched, which is why the tolerance is
        // per field rather than on `behaviorHints` as a whole.
        assert_eq!(s.behavior_hints.stream_type.as_deref(), Some("hls"));

        // `url` is the stated exception: DEREFERENCED, so a number is still fatal
        // and the stream is still refused (declared divergence `D-typed-url`).
        let r: StreamsResponse = serde_json::from_str(
            r#"{"streams":[{"url":12345,"name":"x"},{"url":"b","behaviorHints":{"videoSize":"huge"}}]}"#,
        )
        .unwrap();
        let urls: Vec<&str> = r.streams.iter().filter_map(|s| s.url.as_deref()).collect();
        assert_eq!(urls, vec!["b"]);
    }

    #[test]
    fn an_absent_or_null_streams_array_is_an_empty_one() {
        assert!(serde_json::from_str::<StreamsResponse>(r#"{}"#)
            .unwrap()
            .streams
            .is_empty());
        assert!(
            serde_json::from_str::<StreamsResponse>(r#"{"streams":null}"#)
                .unwrap()
                .streams
                .is_empty()
        );
    }

    /// Absent things stay absent on the way out. The shell stores these records,
    /// and a defaulted `false`/`{}` in every one of them is new bytes for nothing.
    #[test]
    fn serialisation_omits_defaults() {
        let s: Stream = serde_json::from_str(r#"{"url":"u"}"#).unwrap();
        assert_eq!(serde_json::to_string(&s).unwrap(), r#"{"url":"u"}"#);
    }

    // -- the mapping layer ---------------------------------------------------

    fn mapped(json: &str) -> Option<AddonStream> {
        map_addon_stream(&serde_json::from_str::<Stream>(json).unwrap(), "Add-on")
    }

    /// Every branch of `detectQuality`, including the two the twin gets *wrong* —
    /// they are preserved because the corpus has to pass before the twin can go.
    #[test]
    fn detect_quality_reproduces_every_branch_including_the_wrong_ones() {
        assert_eq!(detect_quality("Movie 2160p HDR"), "4K");
        assert_eq!(detect_quality("Movie UHD"), "4K");
        assert_eq!(detect_quality("Movie 4K"), "4K");
        assert_eq!(detect_quality("movie [4k] rip"), "4K");
        assert_eq!(detect_quality("Movie 1080p"), "1080p");
        assert_eq!(detect_quality("Movie 720p"), "720p");
        assert_eq!(detect_quality("Movie 480p"), "480p");
        assert_eq!(detect_quality("Movie 360p"), "", "no badge below 480");
        assert_eq!(detect_quality(""), "");
        assert_eq!(detect_quality("Some Release"), "");

        // WRONG, AND PRESERVED (1): the numeric tests are bare substrings.
        assert_eq!(detect_quality("Frame 10800 of 20000"), "1080p");
        assert_eq!(detect_quality("bitrate 14800"), "480p");
        // WRONG, AND PRESERVED (2): `4k` is word-bounded, so this is NOT 4K...
        assert_eq!(detect_quality("x4k"), "");
        assert_eq!(detect_quality("4kids"), "");
        // ...but JavaScript's `\w` is ASCII-only, so a Georgian letter IS a
        // boundary and this one matches.
        assert_eq!(detect_quality("ე4k"), "4K");
        // First match wins, in the twin's order.
        assert_eq!(detect_quality("2160p downscaled from 1080p"), "4K");
    }

    #[test]
    fn detect_resolution_is_the_single_table_quality_is_derived_from() {
        assert_eq!(detect_resolution("2160p"), Some(2160));
        assert_eq!(detect_resolution("1080p"), Some(1080));
        assert_eq!(detect_resolution("720p"), Some(720));
        assert_eq!(detect_resolution("480p"), Some(480));
        assert_eq!(detect_resolution("360p"), Some(360));
        assert_eq!(detect_resolution("nothing"), None);
        // The 360 arm is last, so it cannot change an answer the twin gave.
        assert_eq!(detect_resolution("1080p from 360p source"), Some(1080));
    }

    #[test]
    fn extract_size_matches_the_twins_regex() {
        assert_eq!(extract_size("Movie 2.3 GB").as_deref(), Some("2.3 GB"));
        assert_eq!(extract_size("Movie 2.3GB").as_deref(), Some("2.3 GB"));
        assert_eq!(extract_size("Movie 700mb").as_deref(), Some("700 MB"));
        assert_eq!(extract_size("Movie 700 Mb").as_deref(), Some("700 MB"));
        // Leftmost match only, and a resolution is not a size.
        assert_eq!(
            extract_size("1080p 2.3 GB / 4.1 GB").as_deref(),
            Some("2.3 GB")
        );
        // `\s?` is ONE optional space — two spaces is not a match.
        assert_eq!(extract_size("Movie 2.3  GB"), None);
        // A second dot ends the number; the regex restarts after it.
        assert_eq!(extract_size("v2.3.4gb").as_deref(), Some("3.4 GB"));
        assert_eq!(extract_size("no size here"), None);
        assert_eq!(extract_size(""), None);
        // Units other than gb/mb are not sizes.
        assert_eq!(extract_size("900 kb"), None);
    }

    /// The scan order is the property, not the flag table: three consecutive
    /// indicators yield ONE language, because `exec` with `/g` consumes the first
    /// pair and leaves the third with no partner.
    #[test]
    fn parse_stream_langs_reproduces_the_regex_scan() {
        assert_eq!(parse_stream_langs("🇬🇧 1080p"), vec!["en"]);
        assert_eq!(parse_stream_langs("🇬🇪 ქართული"), vec!["ka"]);
        assert_eq!(parse_stream_langs("🇬🇧 / 🇷🇺"), vec!["en", "ru"]);
        // Dedup, first-seen order.
        assert_eq!(parse_stream_langs("🇺🇸 🇬🇧 🇷🇺"), vec!["en", "ru"]);
        // THE ONE THAT MATTERS: 🇬 🇧 🇺 — the pair GB is consumed, U is orphaned.
        assert_eq!(
            parse_stream_langs("\u{1F1EC}\u{1F1E7}\u{1F1FA}"),
            vec!["en"],
            "a 'find all flags' rewrite would report two languages here"
        );
        // Four in a row are two pairs: GB then US.
        assert_eq!(
            parse_stream_langs("\u{1F1EC}\u{1F1E7}\u{1F1FA}\u{1F1F8}"),
            vec!["en"]
        );
        // An unmapped country degrades to its own letters rather than vanishing.
        assert_eq!(parse_stream_langs("🇸🇪 subs"), vec!["se"]);
        assert_eq!(parse_stream_langs("no flags"), Vec::<String>::new());
        // A lone indicator is not a flag.
        assert_eq!(parse_stream_langs("\u{1F1EC} alone"), Vec::<String>::new());
    }

    #[test]
    fn map_addon_stream_joins_the_label_and_defaults_it() {
        let s = mapped(r#"{"url":"u","name":"N","title":"T","description":"D"}"#).unwrap();
        assert_eq!(s.label, "N\nT\nD");
        assert_eq!(s.source, "Add-on");

        // Empty strings are dropped by `filter(Boolean)`, not joined as blanks.
        assert_eq!(
            mapped(r#"{"url":"u","name":"N","title":""}"#)
                .unwrap()
                .label,
            "N"
        );
        // Nothing to say → the placeholder, but the DETECTORS saw the empty label.
        let bare = mapped(r#"{"url":"u"}"#).unwrap();
        assert_eq!(bare.label, "Source");
        assert_eq!(bare.quality, "");
        assert_eq!(bare.size, None);
    }

    #[test]
    fn map_addon_stream_drops_a_stream_with_no_url() {
        assert_eq!(mapped(r#"{"name":"N"}"#), None);
        assert_eq!(mapped(r#"{"url":"","name":"N"}"#), None);
        // Including one that has some other source — the shell cannot play it.
        assert_eq!(mapped(r#"{"infoHash":"abc","name":"N"}"#), None);
    }

    #[test]
    fn map_addon_stream_detects_hls_four_ways() {
        let kind = |json: &str| mapped(json).unwrap().kind;
        assert_eq!(kind(r#"{"url":"https://a.co/v.m3u8"}"#), "hls");
        assert_eq!(kind(r#"{"url":"https://a.co/v.M3U8?t=1"}"#), "hls");
        assert_eq!(kind(r#"{"url":"https://a.co/v.txt"}"#), "hls");
        assert_eq!(kind(r#"{"url":"https://a.co/hls/v.bin"}"#), "hls");
        assert_eq!(
            kind(r#"{"url":"https://a.co/v.mp4","behaviorHints":{"streamType":"hls"}}"#),
            "hls"
        );
        assert_eq!(kind(r#"{"url":"https://a.co/v.mp4"}"#), "url");
        // `.m3u8` must end a segment: a path that merely contains it is not HLS.
        assert_eq!(kind(r#"{"url":"https://a.co/v.m3u8x"}"#), "url");
        // The hint is compared exactly, as `===` does.
        assert_eq!(
            kind(r#"{"url":"https://a.co/v.mp4","behaviorHints":{"streamType":"HLS"}}"#),
            "url"
        );
    }

    #[test]
    fn map_addon_stream_resolves_languages_in_the_twins_order() {
        let langs = |json: &str| mapped(json).unwrap().langs;
        // 1. an explicit hint wins over the flags in the label
        assert_eq!(
            langs(r#"{"url":"u","name":"🇷🇺 dub","behaviorHints":{"lang":"ka"}}"#),
            vec!["ka"]
        );
        // 2. else the flags
        assert_eq!(langs(r#"{"url":"u","name":"🇷🇺 dub"}"#), vec!["ru"]);
        // 3. else English — the one guess in this function
        assert_eq!(langs(r#"{"url":"u","name":"no flag"}"#), vec!["en"]);
        // An empty hint is not a hint (`bh.lang ? …` is a truthiness test).
        assert_eq!(
            langs(r#"{"url":"u","name":"🇷🇺 dub","behaviorHints":{"lang":""}}"#),
            vec!["ru"]
        );
    }

    /// The bytes, whole. This is the record the harness will compare against
    /// `mapAddonStream`'s output, so field order and absence rules are asserted
    /// literally rather than field by field.
    #[test]
    fn mapped_stream_serialises_in_the_twins_field_order() {
        let s = mapped(
            r#"{"url":"https://a.co/v.mkv","name":"🇬🇧 Provider","title":"Movie 2160p 12.5 GB",
                "subtitles":[{"url":"https://a.co/s.srt","lang":"en"},{"lang":"ka"}]}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"source":"Add-on","label":"🇬🇧 Provider\nMovie 2160p 12.5 GB","quality":"4K","size":"12.5 GB","kind":"url","url":"https://a.co/v.mkv","langs":["en"],"subtitles":[{"url":"https://a.co/s.srt","lang":"en"}]}"#
        );

        // `size` is null-but-present; `subtitles` is absent entirely.
        assert_eq!(
            serde_json::to_string(&mapped(r#"{"url":"u"}"#).unwrap()).unwrap(),
            r#"{"source":"Add-on","label":"Source","quality":"","size":null,"kind":"url","url":"u","langs":["en"]}"#
        );

        // An array whose entries are all unusable is still an ARRAY.
        let empty = mapped(r#"{"url":"u","subtitles":[{"lang":"en"}]}"#).unwrap();
        assert_eq!(empty.subtitles, Some(Vec::new()));
        assert!(serde_json::to_string(&empty)
            .unwrap()
            .contains(r#""subtitles":[]"#));
    }

    /// The response-level rule, end to end: a malformed entry, an entry with no
    /// URL and a good one. Two of the three are dropped and the title still has a
    /// source — which is the whole reason both filters exist.
    #[test]
    fn mapping_a_response_drops_only_the_unusable_entries() {
        let r: StreamsResponse = serde_json::from_str(
            r#"{"streams":[{"url":"a","name":"A"},42,{"name":"no url"},{"url":"b","name":"B"}]}"#,
        )
        .unwrap();
        let out = map_addon_streams(&r, "Cinemeta");
        let urls: Vec<&str> = out.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, vec!["a", "b"]);
        assert!(out.iter().all(|s| s.source == "Cinemeta"));
    }

    #[test]
    fn round_trips_a_realistic_stream() {
        let raw = r#"{"url":"https://a.co/v.m3u8","name":"Addon","title":"Movie 2160p HDR",
                      "subtitles":[{"url":"https://a.co/s.srt","lang":"en"}],
                      "behaviorHints":{"notWebReady":true,"videoSize":9000000000}}"#;
        let s: Stream = serde_json::from_str(raw).unwrap();
        let back: Stream = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
