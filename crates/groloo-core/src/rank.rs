//! Capability-aware stream ranking — **the shell probes, the core decides**.
//!
//! ## The bug this exists to fix
//!
//! `VideoPlayer.tsx:190` treats hls.js's `noLevelsAvailable` (and
//! `manifestIncompatible`, and `levelEmpty`) as terminal and shows "Source
//! unavailable". Those errors mean *Media Source Extensions declined the codec* —
//! and MSE declines things the television's hardware decoder handles perfectly
//! well, because `MediaSource.isTypeSupported` answers for the browser's software
//! pipeline, not for the panel. The result is that a 4K HEVC source is declared
//! unplayable on exactly the hardware that plays it best, while a 720p AVC source
//! from the same add-on works. To the user this reads as "some sources are
//! broken", which is unactionable, and there is no ranking anywhere that could
//! have preferred the source that would have worked: today's entire ordering is
//! `qualityRank(b.quality) - qualityRank(a.quality)` over a five-value integer
//! derived by regex from free text (`DetailModal.tsx:271` and `:282`, the same
//! expression twice). Nothing in the app knows what a codec is.
//!
//! ## The split, and why it is the only one that works
//!
//! `MediaSource.isTypeSupported` and `HTMLMediaElement.canPlayType` are DOM APIs;
//! `MediaCodecList` is Android's. The core cannot call any of them and must not
//! try. So the **shell probes once per device** and hands the answer in as
//! [`Capabilities`]; the **core owns the rule**. That is the same shape
//! [`crate::rows::visible_rows`] uses for its row table, and it is what lets one
//! rule serve webOS, Android TV and the browser — the alternative is three
//! ranking implementations that agree until somebody edits one.
//!
//! ## The rule that must not be got wrong
//!
//! > **A stream is blocked only on a token that was positively identified and is
//! > not allowed. `None` never blocks.**
//!
//! This is the whole fix, and inverting it would be worse than the original bug.
//! Over-rejection is what breaks playback today; a core that rejected an
//! unlabelled torrent because it could not read a codec out of a release title
//! would be the identical failure with the sign flipped, and *silent*, because
//! nobody would see a reason. An unlabelled stream stays playable, is marked
//! `confidence: "unknown"`, and sorts below equivalently-specced known-good ones.
//!
//! The second half of the same principle: **blocked streams are returned, never
//! dropped**, each with the tokens that blocked it. That list is what lets the UI
//! say "no compatible source on this device" instead of rendering an empty box,
//! and it is the difference between a diagnosis and a shrug.
//!
//! Two consequences worth stating because they look like omissions:
//!
//! - **An absent or empty allow-list means "no constraint on this axis"**, never
//!   "allow nothing". A shell whose probe fails, or which has not been taught to
//!   probe audio yet, must degrade to permissive — otherwise a probe bug empties
//!   the source list on every title, which is the failure mode this module was
//!   written to remove. [`Capabilities::default`] is fully permissive for the same
//!   reason: the *core* supplies the fallback, so three shells cannot each guess a
//!   different one.
//! - **`maxHeight` caps, it does not block** (unless a profile explicitly asks it
//!   to, see [`Capabilities::block_above_max_height`]). A 2160p source on a 1080p
//!   panel is a source that downscales, not a source that fails; it simply must
//!   not outrank the 1080p one, which is what the cap achieves. A device that
//!   genuinely cannot *decode* above a height says so with the flag.
//!
//! ## Relationship to what it replaces
//!
//! `qualityRank` ([`quality_rank`]) and `orderLangs` ([`order_langs`]) are ported
//! here rather than deleted, because the language tabs still need the second one
//! and the corpus still needs the first. Under a fully permissive profile with
//! `preferLangs` set to the single selected language, this module **refines**
//! today's order: it never places a lower-quality stream above a higher-quality
//! one, and it breaks the ties that `qualityRank` left to add-on fan-out order by
//! file size. See `rank_refines_the_twins_order` for that property as a test, and
//! read it before assuming exact equality — it is deliberately not that.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

use crate::stream::{
    detect_resolution, extract_size, is_js_space, parse_stream_langs, StreamBehaviorHints,
};

/// Preferred display order for the language tabs — `addonClient.ts:31`.
const LANG_ORDER: [&str; 4] = ["en", "ka", "ru", "uk"];

/// `addonClient.ts:40`'s `QRANK`.
const QRANK: [(&str, u8); 4] = [("4K", 4), ("1080p", 3), ("720p", 2), ("480p", 1)];

/// Deduplicate and order language codes for display — `addonClient.ts:32-38`.
///
/// The four codes GROLOO ships for come first, in that order; everything else
/// follows, sorted.
///
/// **DECLARED DIVERGENCE RISK.** The twin's tie-break is `a.localeCompare(b)`,
/// which is ICU collation; this is `str::cmp`, which is byte order. They agree for
/// every input the app can currently produce — `LANG_ORDER` entries,
/// `behaviorHints.lang`, and `FLAG_LANG` values or lowercased ISO country codes
/// out of `parseStreamLangs`, all of which are `[a-z]{2}` — and they disagree the
/// moment one is not: `localeCompare` sorts `"ä"` before `"z"` and lowercase
/// before uppercase, byte order does neither. That assumption is executable rather
/// than assumed at `order_langs_pins_the_locale_compare_assumption`.
pub fn order_langs(langs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in langs {
        if !out.contains(l) {
            out.push(l.clone());
        }
    }
    let rank = |s: &str| -> usize {
        LANG_ORDER
            .iter()
            .position(|c| *c == s)
            .unwrap_or(LANG_ORDER.len().saturating_add(95))
    };
    out.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.as_str().cmp(b.as_str()))
    });
    out
}

/// The five-value quality score today's sort uses — `addonClient.ts:41`.
///
/// Kept because the corpus compares against it and because it is the *only*
/// ordering the shell has until [`rank_streams`] replaces both sort sites. It
/// scores the badge string [`crate::stream::detect_quality`] produces, so
/// `""` — anything unrecognised — is 0 and sorts last, which is the same
/// "unknown ranks low, never out" rule this module generalises.
pub fn quality_rank(quality: &str) -> u8 {
    QRANK
        .iter()
        .find(|(q, _)| *q == quality)
        .map_or(0, |(_, r)| *r)
}

// ---------------------------------------------------------------------------
// the capability profile
// ---------------------------------------------------------------------------

/// What this device can actually play, as probed **once** by the shell.
///
/// Every list is an allow-list of lowercase tokens and every empty one means "no
/// constraint on this axis". The tokens are the normalised forms
/// [`classify`] produces — `hvc1`, not `hevc`; `ec-3`, not `ddp` — so that a shell
/// which built its profile out of `isTypeSupported('video/mp4; codecs="hvc1"')`
/// probes can pass its own strings straight through without a translation table
/// on the JavaScript side.
///
/// Wire shape (every field optional; this is the fully permissive default):
///
/// ```json
/// { "maxHeight": null, "blockAboveMaxHeight": false, "video": [], "audio": [],
///   "containers": [], "hdr": [], "preferLangs": [], "allowUnknown": true }
/// ```
///
/// There is deliberately **no `maxBitrateKbps`**. Nothing in an add-on response
/// states a bitrate, and the only proxy available — file size — has no duration to
/// divide by, so the field could be honoured on no input at all. A capability the
/// core cannot evaluate is a capability a shell will set and then wonder why
/// nothing changed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    /// The panel's vertical resolution. Caps the ranking value of a taller
    /// stream so it cannot outrank a native-height one; see the module doc.
    pub max_height: Option<u32>,
    /// Set only by a device that genuinely cannot **decode** above
    /// [`Self::max_height`]. Turns the cap into a block, with a reason.
    pub block_above_max_height: bool,
    /// e.g. `["avc1", "hvc1", "vp9", "av01"]`, in preference order — the index is
    /// used as a ranking key, so the shell states its preference by ordering.
    pub video: Vec<String>,
    /// e.g. `["mp4a", "ec-3", "ac-3", "opus"]`, in preference order.
    pub audio: Vec<String>,
    /// e.g. `["mp4", "webm", "hls", "mkv"]`.
    pub containers: Vec<String>,
    /// e.g. `["sdr", "hdr10", "hlg", "dv"]`.
    pub hdr: Vec<String>,
    /// Audio languages the user prefers, most-wanted first. Ordering only — a
    /// language is never a reason to block, because a source in the wrong language
    /// is still a source and the alternative is an empty list.
    pub prefer_langs: Vec<String>,
    /// When `false`, an axis this profile constrains blocks every stream whose
    /// token on that axis could not be identified. **Default `true`**, and it
    /// should stay true on any device that can afford a failed playback attempt:
    /// see the module doc on why over-rejection is the worse failure.
    pub allow_unknown: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities {
            max_height: None,
            block_above_max_height: false,
            video: Vec::new(),
            audio: Vec::new(),
            containers: Vec::new(),
            hdr: Vec::new(),
            prefer_langs: Vec::new(),
            allow_unknown: true,
        }
    }
}

// ---------------------------------------------------------------------------
// classification
// ---------------------------------------------------------------------------

/// A stream candidate, in either of the two shapes the app has one in.
///
/// The shell holds `AddonStream` records (`label`, `langs`, `size`) after
/// [`crate::stream::map_addon_streams`]; a native shell may hold the raw wire
/// [`crate::stream::Stream`] (`name`/`title`/`description`, `behaviorHints`).
/// Both deserialise into this, so ranking does not force a shape on the caller and
/// there is no second "rank the raw ones" entry point to keep in step.
///
/// **Every field is as tolerant as [`crate::stream::Stream`]'s, and it has to be**,
/// because "both shapes deserialise into this" is a promise this type can only keep
/// if it accepts everything the other one does. It is also the type with the widest
/// blast radius in the crate: `api::rank_streams` reads a plain `Vec<RankCandidate>`,
/// so — unlike `StreamsResponse`, which drops the row — one unreadable field in one
/// candidate used to fail the WHOLE call and answer with an empty ranking. That is
/// the "one bad row costs that row" rule inverted: one bad row costing every row,
/// on the code path whose entire purpose is to stop the source list from looking
/// empty. Nothing here is dereferenced (`url` is read for a file extension, never
/// fetched), so nothing here is worth a ranking.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankCandidate {
    /// The mapped record's joined label. Preferred when present.
    #[serde(default, deserialize_with = "crate::types::de::opt_string_or_number")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::opt_string_or_number")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::opt_string_or_number")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::opt_string_or_number")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::optional")]
    pub url: Option<String>,
    /// The mapped record's resolved languages. Preferred when non-empty.
    #[serde(default, deserialize_with = "crate::types::de::optional")]
    pub langs: Option<Vec<String>>,
    /// The mapped record's `"2.3 GB"`. Re-derived from the label when absent.
    #[serde(default, deserialize_with = "crate::types::de::opt_string_or_number")]
    pub size: Option<String>,
    #[serde(default, deserialize_with = "crate::types::de::default_on_error")]
    pub behavior_hints: StreamBehaviorHints,
}

impl RankCandidate {
    /// The text every detector reads: the mapped label, or the wire fields joined
    /// the way `addonClient.ts:94` joins them.
    fn text(&self) -> String {
        match self.label.as_deref().filter(|l| !l.is_empty()) {
            Some(l) => l.to_string(),
            None => [&self.name, &self.title, &self.description]
                .into_iter()
                .filter_map(|v| v.as_deref().filter(|t| !t.is_empty()))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Everything the core could work out about one stream. Every field is
/// `Option` — that is the load-bearing part, not a convenience.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamClass {
    pub resolution: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    pub hdr: Option<String>,
    pub size_bytes: Option<u64>,
    pub langs: Vec<String>,
}

impl StreamClass {
    /// How much of this was actually read, rather than guessed at.
    ///
    /// `"high"` needs both a resolution and a video codec — enough to make a
    /// compatibility judgement that means something. `"unknown"` means nothing at
    /// all was identified, and it is a label on the *evidence*, never a verdict on
    /// the stream: those still play.
    fn confidence(&self) -> &'static str {
        if self.resolution.is_some() && self.video_codec.is_some() {
            return "high";
        }
        if self.resolution.is_some()
            || self.video_codec.is_some()
            || self.audio_codec.is_some()
            || self.container.is_some()
            || self.hdr.is_some()
        {
            return "medium";
        }
        "unknown"
    }
}

/// Video codec markers, specific before general; first hit wins.
const VIDEO_CODECS: [(&str, &str); 12] = [
    ("hevc", "hvc1"),
    ("h265", "hvc1"),
    ("h.265", "hvc1"),
    ("x265", "hvc1"),
    ("avc", "avc1"),
    ("h264", "avc1"),
    ("h.264", "avc1"),
    ("x264", "avc1"),
    ("av1", "av01"),
    ("vp9", "vp9"),
    ("xvid", "xvid"),
    ("divx", "xvid"),
];

/// Audio codec markers. Order matters where one is a substring of another that
/// the word rule cannot separate — `e-ac3` before `ac3`.
const AUDIO_CODECS: [(&str, &str); 17] = [
    ("eac3", "ec-3"),
    ("e-ac3", "ec-3"),
    ("ddp", "ec-3"),
    ("dd+", "ec-3"),
    ("ddplus", "ec-3"),
    ("atmos", "truehd"),
    ("truehd", "truehd"),
    ("dts-hd", "dts"),
    ("dtshd", "dts"),
    ("dts", "dts"),
    ("ac3", "ac-3"),
    ("ac-3", "ac-3"),
    ("aac", "mp4a"),
    ("opus", "opus"),
    ("flac", "flac"),
    ("mp3", "mp3"),
    ("vorbis", "vorbis"),
];

/// HDR markers, specific before general.
const HDR_MARKERS: [(&str, &str); 8] = [
    ("dolby vision", "dv"),
    ("dolbyvision", "dv"),
    ("dovi", "dv"),
    ("dv", "dv"),
    ("hlg", "hlg"),
    ("hdr10+", "hdr10"),
    ("hdr10", "hdr10"),
    ("hdr", "hdr10"),
];

/// Container markers in a label. The URL extension is consulted first.
const CONTAINER_MARKERS: [(&str, &str); 5] = [
    ("mkv", "mkv"),
    ("mp4", "mp4"),
    ("webm", "webm"),
    ("avi", "avi"),
    ("matroska", "mkv"),
];

/// URL extension → container. `m3u8`/`mpd` name a *protocol*, and that is the
/// distinction a capability profile cares about.
const URL_EXTENSIONS: [(&str, &str); 8] = [
    ("m3u8", "hls"),
    ("mpd", "dash"),
    ("mp4", "mp4"),
    ("mkv", "mkv"),
    ("webm", "webm"),
    ("avi", "avi"),
    ("mov", "mov"),
    ("ts", "ts"),
];

/// Read what can be read out of one stream. **Its own function on purpose**: this
/// is where every fuzzy match in the system lives, so it is the surface a fixture
/// corpus hammers, and keeping it apart from [`rank_streams`] means a token-table
/// edit cannot silently reorder results without a classification test noticing.
pub fn classify(c: &RankCandidate) -> StreamClass {
    let text = c.text();
    let lower = text.to_lowercase();
    let url_lower = c.url.as_deref().unwrap_or("").to_lowercase();
    let bh = &c.behavior_hints;

    let langs = match c.langs.as_ref().filter(|l| !l.is_empty()) {
        Some(l) => l.clone(),
        None => match bh.lang.as_deref().filter(|l| !l.is_empty()) {
            Some(l) => vec![l.to_string()],
            None => {
                let flags = parse_stream_langs(&text);
                if flags.is_empty() {
                    vec!["en".to_string()]
                } else {
                    flags
                }
            }
        },
    };

    // A stated size beats a scraped one: `behaviorHints.videoSize` is the add-on
    // telling us, `"2.3 GB"` is a regex reading a filename.
    let size_bytes = bh.video_size.or_else(|| {
        c.size
            .clone()
            .or_else(|| extract_size(&text))
            .and_then(|s| parse_size_bytes(&s))
    });

    StreamClass {
        resolution: detect_resolution(&text),
        video_codec: find_marker(&lower, &VIDEO_CODECS),
        audio_codec: find_marker(&lower, &AUDIO_CODECS),
        container: url_container(&url_lower).or_else(|| find_marker(&lower, &CONTAINER_MARKERS)),
        hdr: find_marker(&lower, &HDR_MARKERS),
        size_bytes,
        langs,
    }
}

/// First marker in the table that appears in `lower` as a standalone token.
fn find_marker(lower: &str, table: &[(&str, &str)]) -> Option<String> {
    table
        .iter()
        .find(|(marker, _)| has_marker(lower, marker))
        .map(|(_, normalised)| (*normalised).to_string())
}

/// Is `marker` present as its own token?
///
/// The boundary rule is asymmetric on purpose. The character **before** must not
/// be alphanumeric, so `hevc` does not match inside `xhevc` and `dv` does not
/// match inside `hdv`. The character **after** must not be a *letter*, so `dv`
/// does not match inside `dvdrip` while `ac3` still matches in `ac3 5.1` and `avc`
/// still matches in `avc1` — release titles run codec and channel-count together
/// far more often than they run two codecs together, and precision here is what
/// keeps a misread token from *blocking* a playable stream.
fn has_marker(lower: &str, marker: &str) -> bool {
    let bytes = lower.as_bytes();
    lower.match_indices(marker).any(|(i, _)| {
        let before_ok = match i.checked_sub(1).and_then(|j| bytes.get(j)) {
            Some(&c) => !c.is_ascii_alphanumeric(),
            None => true,
        };
        let after_ok = match i.checked_add(marker.len()).and_then(|j| bytes.get(j)) {
            Some(&c) => !c.is_ascii_alphabetic(),
            None => true,
        };
        before_ok && after_ok
    })
}

/// The container named by a URL's file extension, ignoring query and fragment.
fn url_container(url_lower: &str) -> Option<String> {
    let path = url_lower
        .split(['?', '#'])
        .next()
        .unwrap_or(url_lower)
        .to_string();
    let ext = path.rsplit('.').next()?;
    if ext == path {
        return None;
    }
    URL_EXTENSIONS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, c)| (*c).to_string())
}

/// `"2.3 GB"` → bytes, binary units. Only ever compared against another size, so
/// the choice of 1024 over 1000 changes no ordering — it is stated because a
/// consumer reading `sizeBytes` deserves to know which one it got.
fn parse_size_bytes(size: &str) -> Option<u64> {
    let mut parts = size.split(|c: char| is_js_space(c));
    let number: f64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.to_ascii_uppercase();
    let scale = match unit.as_str() {
        "GB" => 1024.0 * 1024.0 * 1024.0,
        "MB" => 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = number * scale;
    if bytes.is_finite() && bytes >= 0.0 {
        Some(bytes as u64)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// ranking
// ---------------------------------------------------------------------------

/// One stream's verdict. `index` points back into the input array and the stream
/// itself is never re-emitted — so this cannot drift from what
/// [`crate::stream::map_addon_streams`] produced, and the response stays small
/// enough to cross the boundary on every episode change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedStream {
    pub index: usize,
    /// Higher is better. Monotone with the ranking **except** for the two
    /// tie-breaks it cannot encode — file size and input order — so two entries
    /// may share a score and still be deliberately ordered. Sort by the array, not
    /// by this; it exists to be shown in a debug overlay and to make a regression
    /// legible in a diff.
    pub score: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub langs: Vec<String>,
    pub blocked: bool,
    /// `["video:hvc1", "container:mkv"]` — axis and normalised token, one entry
    /// per rule that refused it. Empty when playable.
    ///
    /// This list is the difference between "no compatible source on this device"
    /// and an empty box, and it is why blocked streams are returned rather than
    /// filtered: a UI cannot explain a stream it never received.
    pub blocked_by: Vec<String>,
    /// `"high"` | `"medium"` | `"unknown"` — how much was read, never a verdict.
    pub confidence: &'static str,
}

/// What the boundary answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ranking {
    /// Every input stream, best first, blocked ones last — never a subset.
    pub ranked: Vec<RankedStream>,
    pub summary: RankSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankSummary {
    pub playable: usize,
    pub blocked: usize,
    /// Index (into the **input** array) of the stream to play, or `None` when the
    /// device can play none of them — which is a state the UI must be able to
    /// distinguish from "the add-ons returned nothing".
    pub best_index: Option<usize>,
}

/// The ordering key, in the order it is compared. **Field order is the rule** —
/// derived `Ord` compares them top to bottom, so moving a field here changes what
/// the core prefers, and `Reverse` is what makes a field "bigger is better".
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    /// Playable before blocked, always.
    blocked: bool,
    /// Position in `preferLangs`; `usize::MAX` when this stream matches none.
    lang_rank: usize,
    /// Capped resolution, bigger first.
    resolution: Reverse<u32>,
    /// HDR the device can actually use, present first.
    hdr_match: Reverse<u8>,
    audio_rank: usize,
    video_rank: usize,
    /// Bigger is better, and only ever consulted **within one resolution
    /// bucket** — which falls out of the field order rather than needing a rule,
    /// because the comparison never reaches here unless the capped resolutions
    /// were equal. Across buckets a byte count is noise; within one it is the only
    /// bitrate proxy available.
    size: Reverse<u64>,
}

/// Rank streams for one device. See the module doc for the rule.
pub fn rank_streams(candidates: &[RankCandidate], caps: &Capabilities) -> Ranking {
    let mut scored: Vec<(SortKey, RankedStream)> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let class = classify(candidate);
            let blocked_by = block_reasons(&class, caps);
            let lang_rank = class
                .langs
                .iter()
                .filter_map(|l| caps.prefer_langs.iter().position(|p| p == l))
                .min()
                .unwrap_or(usize::MAX);
            let capped = capped_resolution(class.resolution, caps);
            let hdr_match = u8::from(
                class
                    .hdr
                    .as_deref()
                    .is_some_and(|h| !caps.hdr.is_empty() && caps.hdr.iter().any(|c| c == h)),
            );
            let audio_rank = allow_rank(&class.audio_codec, &caps.audio);
            let video_rank = allow_rank(&class.video_codec, &caps.video);

            let key = SortKey {
                blocked: !blocked_by.is_empty(),
                lang_rank,
                resolution: Reverse(capped),
                hdr_match: Reverse(hdr_match),
                audio_rank,
                video_rank,
                size: Reverse(class.size_bytes.unwrap_or(0)),
            };
            let confidence = class.confidence();
            let ranked = RankedStream {
                index,
                score: score_of(lang_rank, capped, hdr_match, audio_rank, video_rank),
                resolution: class.resolution,
                video_codec: class.video_codec,
                audio_codec: class.audio_codec,
                container: class.container,
                hdr: class.hdr,
                size_bytes: class.size_bytes,
                langs: class.langs,
                blocked: !blocked_by.is_empty(),
                blocked_by,
                confidence,
            };
            (key, ranked)
        })
        .collect();

    // Stable, so equal keys keep add-on fan-out order — the same guarantee
    // `Array.prototype.sort` has given since ES2019 and the reason the twin's
    // order is reproducible at all.
    scored.sort_by(|a, b| a.0.cmp(&b.0));

    let ranked: Vec<RankedStream> = scored.into_iter().map(|(_, r)| r).collect();
    let blocked = ranked.iter().filter(|r| r.blocked).count();
    let summary = RankSummary {
        playable: ranked.len().saturating_sub(blocked),
        blocked,
        best_index: ranked.iter().find(|r| !r.blocked).map(|r| r.index),
    };
    Ranking { ranked, summary }
}

/// Which rules refuse this stream. Empty means playable.
///
/// Each rule fires only when the profile constrains that axis **and** the token
/// was positively identified (or, with `allowUnknown: false`, positively
/// *un*identified). Read this alongside the module doc's statement of the rule:
/// everything here is that one sentence, four times.
fn block_reasons(class: &StreamClass, caps: &Capabilities) -> Vec<String> {
    let mut out = Vec::new();
    for (axis, allowed, token) in [
        ("video", &caps.video, &class.video_codec),
        ("audio", &caps.audio, &class.audio_codec),
        ("container", &caps.containers, &class.container),
        ("hdr", &caps.hdr, &class.hdr),
    ] {
        if allowed.is_empty() {
            continue;
        }
        match token.as_deref() {
            Some(t) if !allowed.iter().any(|a| a == t) => out.push(format!("{axis}:{t}")),
            None if !caps.allow_unknown => out.push(format!("{axis}:unknown")),
            _ => {}
        }
    }
    if caps.block_above_max_height {
        if let (Some(max), Some(res)) = (caps.max_height, class.resolution) {
            if res > max {
                out.push(format!("resolution:{res}"));
            }
        }
    }
    out
}

/// The height this stream is worth to this device: its own, but never more than
/// the panel has. An unknown resolution is worth 0 — it ranks last among playable
/// streams, exactly as `qualityRank("")` does today, and is never blocked for it.
fn capped_resolution(resolution: Option<u32>, caps: &Capabilities) -> u32 {
    match (resolution, caps.max_height) {
        (Some(r), Some(max)) => r.min(max),
        (Some(r), None) => r,
        (None, _) => 0,
    }
}

/// Position in an allow-list, i.e. how much the device prefers this token. An
/// unconstrained axis ranks everything equally rather than arbitrarily.
fn allow_rank(token: &Option<String>, allowed: &[String]) -> usize {
    match token.as_deref() {
        Some(t) => allowed.iter().position(|a| a == t).unwrap_or(usize::MAX),
        None => usize::MAX,
    }
}

/// Pack the comparable half of the key into one descending integer. The weights
/// are powers of two chosen so each field occupies its own range and cannot carry
/// into the next; see [`RankedStream::score`] for what it is and is not for.
fn score_of(
    lang_rank: usize,
    resolution: u32,
    hdr_match: u8,
    audio_rank: usize,
    video_rank: usize,
) -> i64 {
    let inverted = |rank: usize, span: i64| -> i64 {
        if rank == usize::MAX {
            0
        } else {
            span.saturating_sub(i64::try_from(rank).unwrap_or(span))
                .max(0)
        }
    };
    inverted(lang_rank, 1000)
        .saturating_mul(268_435_456)
        .saturating_add(i64::from(resolution).saturating_mul(32_768))
        .saturating_add(i64::from(hdr_match).saturating_mul(16_384))
        .saturating_add(inverted(audio_rank, 64).saturating_mul(128))
        .saturating_add(inverted(video_rank, 64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(json: &str) -> Vec<RankCandidate> {
        serde_json::from_str(json).unwrap()
    }
    fn caps(json: &str) -> Capabilities {
        serde_json::from_str(json).unwrap()
    }
    fn order(r: &Ranking) -> Vec<usize> {
        r.ranked.iter().map(|s| s.index).collect()
    }

    // -- the two ported helpers ---------------------------------------------

    #[test]
    fn quality_rank_scores_the_twins_five_values() {
        assert_eq!(quality_rank("4K"), 4);
        assert_eq!(quality_rank("1080p"), 3);
        assert_eq!(quality_rank("720p"), 2);
        assert_eq!(quality_rank("480p"), 1);
        assert_eq!(quality_rank(""), 0);
        assert_eq!(quality_rank("nonsense"), 0);
    }

    #[test]
    fn order_langs_puts_the_shipped_four_first() {
        let langs =
            |v: &[&str]| order_langs(&v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>());
        assert_eq!(langs(&["ru", "en", "ka"]), vec!["en", "ka", "ru"]);
        assert_eq!(langs(&["fr", "en", "de"]), vec!["en", "de", "fr"]);
        // Deduplicated, and the dedup happens before the sort.
        assert_eq!(langs(&["en", "en", "ka"]), vec!["en", "ka"]);
        assert_eq!(langs(&[]), Vec::<String>::new());
        // uk is fourth in the table, so it beats an alphabetically-earlier code.
        assert_eq!(langs(&["de", "uk"]), vec!["uk", "de"]);
    }

    /// The `localeCompare` assumption, made executable. Both assertions describe
    /// **this** implementation; both are inputs on which ICU collation would
    /// disagree, and neither can be produced by the app today.
    #[test]
    fn order_langs_pins_the_locale_compare_assumption() {
        let langs =
            |v: &[&str]| order_langs(&v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>());
        // Byte order: uppercase sorts before lowercase. `localeCompare` is the
        // other way round. (Codes outside LANG_ORDER, or the table would decide
        // this before the tie-break ever ran.)
        assert_eq!(langs(&["fr", "FR"]), vec!["FR", "fr"]);
        // Byte order: any non-ASCII sorts after every ASCII letter.
        // `localeCompare` would put "ä" next to "a".
        assert_eq!(langs(&["z", "ä"]), vec!["z", "ä"]);
    }

    // -- classification ------------------------------------------------------

    #[test]
    fn classify_reads_a_realistic_release_title() {
        let c = classify(
            &serde_json::from_str(
                r#"{"label":"Movie.2019.2160p.UHD.BluRay.x265.HDR10.DDP5.1-GROUP",
                    "url":"https://a.co/v.mkv","size":"12.5 GB"}"#,
            )
            .unwrap(),
        );
        assert_eq!(c.resolution, Some(2160));
        assert_eq!(c.video_codec.as_deref(), Some("hvc1"));
        assert_eq!(c.audio_codec.as_deref(), Some("ec-3"));
        assert_eq!(c.container.as_deref(), Some("mkv"));
        assert_eq!(c.hdr.as_deref(), Some("hdr10"));
        assert!(c.size_bytes.is_some_and(|b| b > 12_000_000_000));
    }

    /// The precision half of the marker rule. Every one of these is a token that a
    /// substring search would find and that must NOT be found, because a misread
    /// token is what blocks a playable stream.
    #[test]
    fn markers_do_not_match_inside_other_words() {
        let class = |label: &str| {
            classify(
                &serde_json::from_str(&format!(
                    r#"{{"label":{}}}"#,
                    serde_json::Value::String(label.to_string())
                ))
                .unwrap(),
            )
        };
        // `dv` is not `dvdrip`, and `dvd` is not Dolby Vision.
        assert_eq!(class("Movie DVDRip").hdr, None);
        assert_eq!(class("Movie DVD").hdr, None);
        assert_eq!(class("Movie Dolby Vision").hdr.as_deref(), Some("dv"));
        // `ac3` inside `eac3` is the more specific codec, not the general one.
        assert_eq!(class("Movie EAC3").audio_codec.as_deref(), Some("ec-3"));
        assert_eq!(class("Movie AC3").audio_codec.as_deref(), Some("ac-3"));
        // ...and channel counts run into the token constantly.
        assert_eq!(class("Movie AC3 5.1").audio_codec.as_deref(), Some("ac-3"));
        assert_eq!(class("Movie DDP5.1").audio_codec.as_deref(), Some("ec-3"));
        // `avc` must survive `avc1` but not `xavc`.
        assert_eq!(class("Movie AVC1").video_codec.as_deref(), Some("avc1"));
        assert_eq!(class("Movie XAVC").video_codec, None);
        // `dts` inside `dtshd` is still DTS; `dts` inside a word is not.
        assert_eq!(class("Movie DTS-HD MA").audio_codec.as_deref(), Some("dts"));
    }

    #[test]
    fn container_prefers_the_url_over_the_label() {
        let class = |json: &str| classify(&serde_json::from_str(json).unwrap()).container;
        assert_eq!(
            class(r#"{"label":"MKV rip","url":"https://a.co/v.mp4"}"#).as_deref(),
            Some("mp4"),
            "what the URL serves beats what the title claims"
        );
        assert_eq!(
            class(r#"{"label":"MKV rip","url":"https://a.co/stream"}"#).as_deref(),
            Some("mkv")
        );
        // A protocol is a container as far as a capability profile is concerned.
        assert_eq!(
            class(r#"{"url":"https://a.co/v.m3u8?t=1"}"#).as_deref(),
            Some("hls")
        );
        assert_eq!(class(r#"{"url":"https://a.co/stream"}"#), None);
    }

    #[test]
    fn a_stated_video_size_beats_a_scraped_one() {
        let c = classify(
            &serde_json::from_str(r#"{"label":"Movie 2.0 GB","behaviorHints":{"videoSize":9999}}"#)
                .unwrap(),
        );
        assert_eq!(c.size_bytes, Some(9999));
    }

    #[test]
    fn classify_accepts_the_raw_wire_shape_too() {
        let c = classify(
            &serde_json::from_str(
                r#"{"name":"🇬🇪 Provider","title":"Movie 1080p x264",
                    "url":"https://a.co/v.mp4","behaviorHints":{"videoSize":1024}}"#,
            )
            .unwrap(),
        );
        assert_eq!(c.resolution, Some(1080));
        assert_eq!(c.video_codec.as_deref(), Some("avc1"));
        assert_eq!(c.langs, vec!["ka"], "flags are read from the joined fields");
        assert_eq!(c.size_bytes, Some(1024));
    }

    /// The promise the type doc makes, kept: **anything a [`crate::stream::Stream`]
    /// accepts, a candidate accepts**. `api::rank_streams` reads a plain `Vec`, so
    /// a candidate that failed to deserialise did not cost one source — it emptied
    /// the ranking and answered `ok:false`, on the one call whose job is to keep the
    /// source list from looking empty.
    #[test]
    fn a_malformed_field_never_costs_the_whole_ranking() {
        let raw = r#"[{"name":1080,"url":"https://a.co/a.mp4"},
                      {"label":"Movie 720p","url":"https://a.co/b.mp4","fileIdx":"2",
                       "behaviorHints":{"videoSize":"9.2 GB","notWebReady":"yes"}},
                      {"label":"Movie 1080p","url":"https://a.co/c.mp4","langs":"en",
                       "behaviorHints":"hls"}]"#;
        // Every stream in the corresponding wire response parses as a `Stream`...
        let streams: crate::stream::StreamsResponse =
            serde_json::from_str(&format!(r#"{{"streams":{raw}}}"#)).unwrap();
        assert_eq!(streams.streams.len(), 3);
        // ...so every one of them must parse as a candidate too, or the two shapes
        // are not in fact interchangeable.
        let list = candidates(raw);
        assert_eq!(list.len(), 3);
        // The numeric name is stringified, and the detectors read it.
        assert_eq!(classify(&list[0]).resolution, Some(1080));
        // The unreadable hint is gone; the stream and its label are not.
        assert_eq!(list[1].behavior_hints.video_size, None);
        assert_eq!(classify(&list[1]).resolution, Some(720));
        // And the ranking still contains all three.
        let r = rank_streams(&list, &Capabilities::default());
        assert_eq!(r.ranked.len(), 3);
        assert_eq!(r.summary.playable, 3);
    }

    #[test]
    fn confidence_reports_the_evidence_not_a_verdict() {
        let conf = |json: &str| classify(&serde_json::from_str(json).unwrap()).confidence();
        assert_eq!(conf(r#"{"label":"Movie 1080p x264"}"#), "high");
        assert_eq!(conf(r#"{"label":"Movie 1080p"}"#), "medium");
        assert_eq!(conf(r#"{"label":"Some Release"}"#), "unknown");
    }

    // -- the rule ------------------------------------------------------------

    /// THE regression test for the bug this module exists to fix, stated as the
    /// two halves that must both hold.
    #[test]
    fn an_unlabelled_stream_stays_playable_while_a_known_bad_one_is_blocked() {
        let list = candidates(
            r#"[{"label":"Movie 2160p HEVC","url":"https://a.co/a.mkv"},
                {"label":"Some Release","url":"https://a.co/b"},
                {"label":"Movie 1080p x264","url":"https://a.co/c.mp4"}]"#,
        );
        // A device whose MSE only admits AVC in MP4.
        let profile = caps(r#"{"video":["avc1"],"containers":["mp4"]}"#);
        let r = rank_streams(&list, &profile);

        // 0 is blocked, WITH REASONS, and is still in the array.
        let hevc = r.ranked.iter().find(|s| s.index == 0).unwrap();
        assert!(hevc.blocked);
        assert_eq!(hevc.blocked_by, vec!["video:hvc1", "container:mkv"]);

        // 1 identified nothing at all and is therefore PLAYABLE. This is the
        // sign-flipped bug, and it must never come back.
        let unknown = r.ranked.iter().find(|s| s.index == 1).unwrap();
        assert!(!unknown.blocked, "an unlabelled stream must stay playable");
        assert_eq!(unknown.confidence, "unknown");

        // The known-good one wins, and nothing was dropped.
        assert_eq!(r.summary.best_index, Some(2));
        assert_eq!(r.summary.playable, 2);
        assert_eq!(r.summary.blocked, 1);
        assert_eq!(r.ranked.len(), 3);
        // Blocked entries sort last regardless of how good they look.
        assert_eq!(order(&r).last(), Some(&0));
    }

    /// The 4K HEVC case from the module doc: on a device that DOES admit HEVC it
    /// must win, which is the outcome `noLevelsAvailable` currently denies.
    #[test]
    fn four_k_hevc_wins_on_a_device_that_can_decode_it() {
        let list = candidates(
            r#"[{"label":"Movie 1080p x264 AAC","url":"https://a.co/a.mp4"},
                {"label":"Movie 2160p HEVC DDP5.1","url":"https://a.co/b.mp4"}]"#,
        );
        let profile = caps(
            r#"{"maxHeight":2160,"video":["hvc1","avc1"],"audio":["ec-3","mp4a"],
                "containers":["mp4"]}"#,
        );
        assert_eq!(rank_streams(&list, &profile).summary.best_index, Some(1));
    }

    /// An empty allow-list is "no constraint", never "allow nothing" — a probe
    /// that failed must not empty the list.
    #[test]
    fn an_empty_profile_blocks_nothing() {
        let list = candidates(
            r#"[{"label":"Movie 2160p HEVC DTS","url":"https://a.co/a.mkv"},
                {"label":"Movie 480p XviD","url":"https://a.co/b.avi"}]"#,
        );
        for profile in [Capabilities::default(), caps("{}"), caps(r#"{"video":[]}"#)] {
            let r = rank_streams(&list, &profile);
            assert_eq!(r.summary.blocked, 0);
            assert_eq!(r.summary.playable, 2);
        }
    }

    #[test]
    fn allow_unknown_false_blocks_the_unidentifiable_with_a_reason() {
        let list = candidates(r#"[{"label":"Some Release","url":"https://a.co/a"}]"#);
        let strict = caps(r#"{"video":["avc1"],"allowUnknown":false}"#);
        let r = rank_streams(&list, &strict);
        assert_eq!(r.ranked[0].blocked_by, vec!["video:unknown"]);
        // ...and the default is the permissive one.
        assert!(caps("{}").allow_unknown);
    }

    /// `maxHeight` caps the *value* of a stream without refusing it — a 2160p
    /// source on a 1080p panel downscales, it does not fail.
    #[test]
    fn max_height_caps_rather_than_blocks() {
        let list = candidates(
            r#"[{"label":"Movie 2160p","url":"https://a.co/a.mp4"},
                {"label":"Movie 1080p","url":"https://a.co/b.mp4"}]"#,
        );
        let panel = caps(r#"{"maxHeight":1080}"#);
        let r = rank_streams(&list, &panel);
        assert_eq!(r.summary.blocked, 0);
        // Both cap to 1080, so the tie falls to input order — the 2160p stream is
        // not promoted, and it is not punished either.
        assert_eq!(order(&r), vec![0, 1]);

        // A device that genuinely cannot decode above the cap says so.
        let limited = caps(r#"{"maxHeight":1080,"blockAboveMaxHeight":true}"#);
        let r = rank_streams(&list, &limited);
        assert_eq!(r.summary.best_index, Some(1));
        assert_eq!(
            r.ranked.iter().find(|s| s.index == 0).unwrap().blocked_by,
            vec!["resolution:2160"]
        );
    }

    #[test]
    fn preferred_languages_order_but_never_block() {
        let list = candidates(
            r#"[{"label":"Movie 2160p","langs":["ru"],"url":"https://a.co/a.mp4"},
                {"label":"Movie 480p","langs":["ka"],"url":"https://a.co/b.mp4"}]"#,
        );
        let profile = caps(r#"{"preferLangs":["ka","en"]}"#);
        let r = rank_streams(&list, &profile);
        // Language outranks resolution — a 480p track in your language beats a
        // 4K one you cannot understand.
        assert_eq!(order(&r), vec![1, 0]);
        assert_eq!(r.summary.blocked, 0, "a wrong language is still a source");
    }

    /// Within one resolution bucket, size is the only bitrate proxy available.
    #[test]
    fn size_breaks_ties_inside_a_resolution_bucket_only() {
        let list = candidates(
            r#"[{"label":"Movie 1080p 1.2 GB","url":"https://a.co/a.mp4"},
                {"label":"Movie 1080p 8.4 GB","url":"https://a.co/b.mp4"},
                {"label":"Movie 2160p 700 MB","url":"https://a.co/c.mp4"}]"#,
        );
        let r = rank_streams(&list, &Capabilities::default());
        // 2160p first despite being the smallest — size never crosses buckets.
        assert_eq!(order(&r), vec![2, 1, 0]);
    }

    #[test]
    fn ties_keep_add_on_fan_out_order() {
        let list = candidates(
            r#"[{"label":"Movie 1080p","url":"https://a.co/a.mp4"},
                {"label":"Movie 1080p","url":"https://a.co/b.mp4"},
                {"label":"Movie 1080p","url":"https://a.co/c.mp4"}]"#,
        );
        assert_eq!(
            order(&rank_streams(&list, &Capabilities::default())),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn an_empty_input_is_an_empty_answer_not_a_failure() {
        let r = rank_streams(&[], &Capabilities::default());
        assert!(r.ranked.is_empty());
        assert_eq!(r.summary.best_index, None);
        assert_eq!(r.summary.playable, 0);
    }

    /// When the device can play nothing, `bestIndex` is `None` — and the UI can
    /// tell that apart from "the add-ons returned nothing", which is the whole
    /// point of returning the blocked entries.
    #[test]
    fn no_compatible_source_is_a_reportable_state() {
        let list = candidates(r#"[{"label":"Movie 2160p HEVC","url":"https://a.co/a.mkv"}]"#);
        let r = rank_streams(&list, &caps(r#"{"video":["avc1"]}"#));
        assert_eq!(r.summary.best_index, None);
        assert_eq!(r.summary.playable, 0);
        assert!(!r.ranked.is_empty(), "the reason must survive");
    }

    // -- the relationship to the code this replaces --------------------------

    /// `DetailModal.tsx:271`/`:282` in Rust, for comparison.
    fn twin_order(list: &[RankCandidate], want: &str) -> Vec<usize> {
        let mut kept: Vec<(usize, u8)> = list
            .iter()
            .enumerate()
            .filter(|(_, c)| classify(c).langs.iter().any(|l| l == want))
            .map(|(i, c)| (i, quality_rank(crate::stream::detect_quality(&c.text()))))
            .collect();
        kept.sort_by_key(|(_, quality)| Reverse(*quality)); // stable, like V8's since ES2019
        kept.into_iter().map(|(i, _)| i).collect()
    }

    /// **The property that makes replacing the twin's sort safe**, and it is
    /// deliberately a refinement rather than an equality.
    ///
    /// Under a fully permissive profile, this module never places a lower-quality
    /// stream above a higher-quality one — so every ordering decision the twin
    /// made is preserved. It additionally breaks ties the twin left to add-on
    /// fan-out order, by file size, which is a *new* decision on inputs where the
    /// twin expressed no preference.
    ///
    /// If the differential corpus ever needs byte-exact equality instead, the
    /// change is one line: drop `neg_size` from [`SortKey`]. It is here rather
    /// than absent because a 700 MB and an 8 GB 1080p source are not
    /// interchangeable and today's UI offers the user no way to tell them apart.
    #[test]
    fn rank_refines_the_twins_order() {
        let list = candidates(
            r#"[{"label":"A 720p","langs":["en"],"url":"https://a.co/a.mp4"},
                {"label":"B 2160p","langs":["en"],"url":"https://a.co/b.mp4"},
                {"label":"C 1080p 2.0 GB","langs":["en"],"url":"https://a.co/c.mp4"},
                {"label":"D no quality","langs":["en"],"url":"https://a.co/d.mp4"},
                {"label":"E 1080p 8.0 GB","langs":["en"],"url":"https://a.co/e.mp4"},
                {"label":"F 480p","langs":["ka"],"url":"https://a.co/f.mp4"}]"#,
        );
        let permissive = caps(r#"{"preferLangs":["en"]}"#);
        let mine = order(&rank_streams(&list, &permissive));
        let theirs = twin_order(&list, "en");

        // Nothing is dropped: the twin's whole result is a prefix-set of mine.
        assert_eq!(theirs, vec![1, 2, 4, 0, 3]);
        assert!(theirs.iter().all(|i| mine.contains(i)));
        // The wrong-language stream is last rather than gone.
        assert_eq!(mine.last(), Some(&5));

        // REFINEMENT: for every pair the twin strictly ordered, so do I.
        let pos = |list: &[usize], i: usize| list.iter().position(|x| *x == i).unwrap();
        for (a_i, a) in theirs.iter().enumerate() {
            for b in theirs.iter().skip(a_i.saturating_add(1)) {
                let qa = quality_rank(crate::stream::detect_quality(&list[*a].text()));
                let qb = quality_rank(crate::stream::detect_quality(&list[*b].text()));
                if qa > qb {
                    assert!(
                        pos(&mine, *a) < pos(&mine, *b),
                        "rank contradicted the twin on {a} vs {b}"
                    );
                }
            }
        }
        // ...and the one genuinely new decision: equal quality, bigger file first.
        assert!(pos(&mine, 4) < pos(&mine, 2));
    }

    #[test]
    fn the_score_is_monotone_with_the_ranking_it_can_encode() {
        let list = candidates(
            r#"[{"label":"Movie 720p x264","langs":["en"],"url":"https://a.co/a.mp4"},
                {"label":"Movie 2160p HEVC","langs":["en"],"url":"https://a.co/b.mp4"},
                {"label":"Movie 1080p x264","langs":["ru"],"url":"https://a.co/c.mp4"}]"#,
        );
        let r = rank_streams(
            &list,
            &caps(r#"{"preferLangs":["en"],"video":["hvc1","avc1"]}"#),
        );
        let scores: Vec<i64> = r.ranked.iter().map(|s| s.score).collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "scores must not increase down the ranking: {scores:?}"
        );
    }

    #[test]
    fn the_answer_serialises_in_the_documented_shape() {
        let list = candidates(
            r#"[{"label":"Movie 1080p x264 AAC 2.0 GB","langs":["en"],"url":"https://a.co/a.mp4"}]"#,
        );
        let r = rank_streams(&list, &caps(r#"{"preferLangs":["en"],"video":["avc1"]}"#));
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v["ranked"][0]["index"], 0);
        assert_eq!(v["ranked"][0]["resolution"], 1080);
        assert_eq!(v["ranked"][0]["videoCodec"], "avc1");
        assert_eq!(v["ranked"][0]["audioCodec"], "mp4a");
        assert_eq!(v["ranked"][0]["container"], "mp4");
        assert_eq!(v["ranked"][0]["blocked"], false);
        assert_eq!(v["ranked"][0]["blockedBy"], serde_json::json!([]));
        assert_eq!(v["ranked"][0]["confidence"], "high");
        assert_eq!(v["summary"]["playable"], 1);
        assert_eq!(v["summary"]["bestIndex"], 0);
    }
}
