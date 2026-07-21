//! `MediaRef` — the one way GROLOO names a title or an episode, and the one
//! place the two string formats built from it are spelled.
//!
//! ## Why this module exists at all
//!
//! Today the platform derives these strings inline, in the view layer:
//!
//! ```text
//!   DetailModal.tsx:188  `${target.id}:S${pickedEp.season}E${pickedEp.ep}`   resume key
//!   DetailModal.tsx:243  `${target.id}:S${ep.season}E${ep.ep}`               resume key (again)
//!   DetailModal.tsx:168  `${imdb}:${pickedEp.season}:${pickedEp.ep}`         add-on video id
//!   DetailModal.tsx:268  `${imdb}:${season}:${ep}`                           add-on video id (again)
//! ```
//!
//! Four template literals, two formats, one component. `07-build-plan.md` §2.7
//! records the decision that closes this: **the core owns media-key parsing and
//! the Android Engage publisher calls it** rather than re-typing the split in
//! Kotlin — because `TvEpisodeEntity` needs `setSeasonNumber`/`setEpisodeNumber`
//! as bare numeric strings, and a season/episode splitter is exactly the
//! four-line function that gets re-implemented instead of called. That is not a
//! hypothetical: `06-rust-core.md` §2.5 documents [`crate::addon::addon_base_url`]
//! sitting unreachable in Rust while `addonClient.ts:45` reimplemented it
//! line-for-line. The API here is therefore shaped so that "call the core" is
//! obviously less work than "split the string myself": one parse in, whole
//! decomposed record out, every rendering as a named method.
//!
//! ## TWO key formats exist. They are not the same format. Do not unify them.
//!
//! | Format | Example | Produced by | Consumed by |
//! |---|---|---|---|
//! | media key | `tt0903747:S1E4` | [`MediaRef::media_key`] | resume/progress map, [`crate::types::LibraryItem::media_key`], [`crate::library`]'s prefix sweep |
//! | video id | `tt0903747:1:4` | [`MediaRef::stream_video_id`] | the Stremio `stream` resource path — the *wire* |
//!
//! They share the `id` prefix and the `:` separator, which is why the temptation
//! to collapse them is real and why this table is in the module doc rather than a
//! commit message. [`crate::library`]'s sweep keys off `format!("{id}:")`, so it
//! is indifferent to which of the two it is looking at — but the add-on wire is
//! not, and neither is the persisted progress map. Changing either string is a
//! data migration, not a refactor.
//!
//! ## `movie` | `series`, and why `tv` is the derived form
//!
//! Two vocabularies land in the same field today. `Stredio-server`'s `mapMovie`
//! emits `'movie' | 'tv'`; `addonClient.ts:134`'s `mapCatalogMeta` emits
//! `'movie' | 'series'`; `lib/types.ts:12` types the field as the union of both,
//! and five call sites patch over it with `=== 'tv' || === 'series'`.
//!
//! The canonical form here is **Stremio's** — see [`MediaKind`]. The add-on
//! protocol is the thing being ported and its wire vocabulary is `series`: the
//! resource path requires it and `manifest.types` is matched against it. `tv` has
//! exactly one producer and one consumer (`/api/meta?type=`), so it is the
//! dialect, and dialects are derived — [`MediaKind::as_api_type`]. Both renderings
//! are exposed so that no call site ever re-derives either one.
//!
//! [`MediaKind::from_wire`] is deliberately total and permissive because
//! **persisted data is never rewritten**: `history.ts` and `library.ts` have
//! written whatever value they had at the time into `localStorage` for as long as
//! they have existed, and both vocabularies are in there now. Tolerating both on
//! read, forever, in one function, is what lets the five patch sites collapse.

use serde::{Deserialize, Serialize};

/// What separates a title id from its episode coordinates in **both** key formats
/// (`tt123:S1E4`, `tt123:1:4`).
///
/// It lives here rather than in [`crate::library`] — where it was first needed and
/// from where it is still re-exported — because this module owns the format and
/// `library` is one of its consumers. A constant defined next to its only historic
/// caller is a constant that acquires a second definition the moment it acquires a
/// second caller.
pub const MEDIA_KEY_SEPARATOR: &str = ":";

/// Which id namespace [`MediaRef::id`] belongs to.
///
/// Closed on purpose, at two arms, because those are the only two namespaces
/// GROLOO produces: TMDB numeric ids from every `/api/*` route, IMDb `tt…` ids
/// from add-on catalogs and from `/api/meta`'s `imdb` field. Stremio itself also
/// ships `kitsu:` and `anidb:` namespaces; if a shell ever needs one, this enum
/// grows an arm and [`MediaRef::parse`] grows a branch — which is a visible,
/// reviewable change. The alternative (an open `Other(String)`) would let an
/// unrecognised id through as a *guess*, and a guessed namespace produces a
/// request that 404s at the add-on with nothing in the log to say why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaSource {
    /// `tt0903747` — IMDb. What every add-on's `stream` resource expects.
    Imdb,
    /// `12345` — TMDB. What `Stredio-server` returns and what `/api/meta` keys on.
    Tmdb,
}

impl MediaSource {
    /// The lowercase token used on the wire and in JSON (`"imdb"` / `"tmdb"`).
    pub fn as_wire(self) -> &'static str {
        match self {
            MediaSource::Imdb => "imdb",
            MediaSource::Tmdb => "tmdb",
        }
    }
}

/// Movie or series — the canonical vocabulary (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    #[default]
    Movie,
    Series,
}

impl MediaKind {
    /// Read a type token from *anywhere* — an add-on catalog, `/api/*`, or a
    /// `localStorage` record written by a build that no longer exists.
    ///
    /// `tv`, `series` and `show` are series; **everything else is a movie**,
    /// including the empty string and tokens nobody has invented yet. Total and
    /// infallible by design: this is the tolerance that lets stored data keep its
    /// three-armed union forever while the core reasons about two cases. A
    /// `Result` here would push exactly one `unwrap_or(Movie)` into every caller,
    /// which is the same rule written five times instead of once.
    pub fn from_wire(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "tv" | "series" | "show" => MediaKind::Series,
            _ => MediaKind::Movie,
        }
    }

    /// [`MediaKind::from_wire`] for a field that may legitimately be *absent*.
    ///
    /// A blank hint is "I don't know", not "movie" — and the difference matters to
    /// [`MediaRef::parse`], which infers `Series` from the presence of episode
    /// coordinates and must not have that inference overwritten by a default.
    pub fn from_wire_hint(s: &str) -> Option<Self> {
        if s.trim().is_empty() {
            None
        } else {
            Some(MediaKind::from_wire(s))
        }
    }

    /// The canonical token — `"movie"` / `"series"`. This is what goes in a
    /// resource path and what is matched against `manifest.types`.
    pub fn as_wire(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Series => "series",
        }
    }

    /// The GROLOO API dialect — `"movie"` / `"tv"`, for `/api/meta?type=`.
    ///
    /// Derived, never stored. `queries.ts:35` currently computes this with
    /// `type === 'tv' || type === 'series' ? 'tv' : 'movie'`; that expression is
    /// this method.
    pub fn as_api_type(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Series => "tv",
        }
    }
}

/// A fully-resolved reference to one playable thing: a film, or one episode of one
/// season of one series.
///
/// Wire shape (episode coordinates omitted entirely when absent, never `null` —
/// the shell persists these records and an explicit `null` would be a new byte in
/// every film ever stored):
///
/// ```json
/// {"source":"imdb","id":"tt0903747","kind":"series","season":1,"episode":4}
/// {"source":"imdb","id":"tt0111161","kind":"movie"}
/// ```
///
/// `season` and `episode` move together — [`MediaRef::parse`] never produces one
/// without the other, and both key formats fall back to the bare id unless both
/// are present. A half-populated pair would silently render as a film.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub source: MediaSource,
    pub id: String,
    /// Defaults to [`MediaKind::Movie`] when a hand-written record omits it, which
    /// matches [`MediaKind::from_wire`]'s answer for an unknown token — one rule,
    /// not two.
    #[serde(default)]
    pub kind: MediaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
}

/// Resources that address a **title**, not a video. `meta/series/tt0903747.json`
/// describes the show; there is no such thing as `meta` for one episode, and
/// asking for `meta/series/tt0903747:1:4.json` 404s at the add-on.
///
/// Everything else — `stream`, `subtitles` — addresses the video, so it carries
/// the episode coordinates. Encoding the rule here rather than at the call site is
/// the difference between a wrong path being impossible and a wrong path being a
/// silent empty source list.
const TITLE_SCOPED_RESOURCES: [&str; 2] = ["meta", "catalog"];

impl MediaRef {
    /// Parse any id string GROLOO can hand us into a decomposed reference, or
    /// `None` if it names nothing this core can address.
    ///
    /// Accepted, with and without a `tmdb:`/`imdb:` prefix (case-insensitive):
    ///
    /// ```text
    ///   tt0111161          film, IMDb
    ///   tt0903747:1:4      episode, IMDb   — the add-on wire form
    ///   tt0903747:S1E4     episode, IMDb   — the resume/library form
    ///   tmdb:12345         film, TMDB
    ///   12345              film, TMDB      — bare numeric ids are TMDB's
    ///   12345:S1E4         episode, TMDB   — what DetailModal writes for a server-sourced series
    /// ```
    ///
    /// **Both key formats parse.** That is the whole point: the caller that has a
    /// resume key and the caller that has a wire video id run the same function
    /// and get the same record, so neither has to know which one it is holding.
    ///
    /// `kind_hint` is consulted **only** when there are no episode coordinates to
    /// infer from — coordinates always win, because a string carrying `S1E4`
    /// cannot be a film no matter what a stale `type` field says. Pass `None`
    /// (or see [`MediaKind::from_wire_hint`]) when the caller genuinely does not
    /// know; the answer then defaults to [`MediaKind::Movie`].
    ///
    /// Returns `None` for: an empty/blank string, an id whose namespace is not
    /// [`MediaSource`], malformed or non-decimal episode coordinates, and more
    /// than three colon-separated parts. `None` is a *reportable* answer the shell
    /// can surface; a guess is not.
    pub fn parse(raw: &str, kind_hint: Option<MediaKind>) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        // An explicit namespace prefix wins over inference. Both names are
        // accepted for symmetry: a caller that has a `MediaSource` in hand should
        // not have to know that only one of the two is spellable.
        let (explicit, rest) = match split_source_prefix(trimmed) {
            Some((src, rest)) => (Some(src), rest),
            None => (None, trimmed),
        };

        let mut parts = rest.split(MEDIA_KEY_SEPARATOR);
        let id = parts.next().unwrap_or("");
        if id.is_empty() {
            return None;
        }

        let (season, episode) = match (parts.next(), parts.next(), parts.next()) {
            // `tt0111161` / `tmdb:12345`
            (None, _, _) => (None, None),
            // `tt0903747:S1E4`
            (Some(token), None, None) => {
                let (s, e) = parse_episode_token(token)?;
                (Some(s), Some(e))
            }
            // `tt0903747:1:4`
            (Some(s), Some(e), None) => (Some(decimal_u32(s)?), Some(decimal_u32(e)?)),
            // Four or more parts is not a shape this format has. Guessing which
            // three of them were meant is how a mangled key becomes a silently
            // wrong resume position.
            _ => return None,
        };

        let source = match explicit {
            Some(s) => s,
            None => infer_source(id)?,
        };

        // Coordinates are proof; the hint is hearsay.
        let kind = if season.is_some() {
            MediaKind::Series
        } else {
            kind_hint.unwrap_or_default()
        };

        Some(MediaRef {
            source,
            id: id.to_string(),
            kind,
            season,
            episode,
        })
    }

    /// True when this reference names a specific episode rather than a whole title.
    pub fn is_episode(&self) -> bool {
        self.season.is_some() && self.episode.is_some()
    }

    /// The **media key** — how resume positions, tombstones and library rows are
    /// keyed: `tt0111161` for a film, `tt0903747:S1E4` for an episode.
    ///
    /// Byte-identical to `DetailModal.tsx:188`/`:243`'s
    /// `` `${target.id}:S${ep.season}E${ep.ep}` ``, and to what
    /// [`crate::types::LibraryItem::media_key`] reads back out.
    pub fn media_key(&self) -> String {
        match (self.season, self.episode) {
            (Some(s), Some(e)) => format!("{}{MEDIA_KEY_SEPARATOR}S{s}E{e}", self.id),
            _ => self.id.clone(),
        }
    }

    /// The **add-on video id** — Stremio's wire form for the `stream` resource:
    /// `tt0111161` for a film, `tt0903747:1:4` for an episode.
    ///
    /// Byte-identical to `DetailModal.tsx:168`/`:268`'s
    /// `` `${imdb}:${season}:${ep}` ``.
    ///
    /// Note the shell passes `meta.imdb` here, not the item's own id — an add-on
    /// resolves IMDb ids, so calling this on a [`MediaSource::Tmdb`] reference
    /// produces a syntactically valid id that no add-on will recognise. The core
    /// cannot resolve TMDB → IMDb (that is a network lookup, and network is the
    /// shell's), so it does not pretend to: the caller supplies the right id.
    pub fn stream_video_id(&self) -> String {
        match (self.season, self.episode) {
            (Some(s), Some(e)) => {
                format!(
                    "{}{MEDIA_KEY_SEPARATOR}{s}{MEDIA_KEY_SEPARATOR}{e}",
                    self.id
                )
            }
            _ => self.id.clone(),
        }
    }

    /// The add-on resource path this reference resolves to, relative to
    /// [`crate::addon::addon_base_url`]:
    ///
    /// ```text
    ///   stream/series/tt0903747%3A1%3A4.json
    ///   stream/movie/tt0111161.json
    ///   meta/series/tt0903747.json            ← title-scoped: coordinates dropped
    /// ```
    ///
    /// The id is percent-encoded because `addonClient.ts:118` wraps it in
    /// `encodeURIComponent`, which turns `:` into `%3A`. Add-ons accept both
    /// forms, so this is not a correctness requirement — it is a *byte-identity*
    /// requirement, and the twin rule says the TypeScript cannot be deleted until
    /// the bytes match. See [`encode_uri_component`].
    pub fn resource_path(&self, resource: &str) -> String {
        let addressed = if TITLE_SCOPED_RESOURCES.contains(&resource) {
            self.id.clone()
        } else {
            self.stream_video_id()
        };
        addon_resource_path(resource, self.kind.as_wire(), &addressed)
    }

    /// The GROLOO API dialect for this reference — `"tv"` or `"movie"`, for
    /// `/api/meta/<id>?type=`. See [`MediaKind::as_api_type`].
    pub fn api_type(&self) -> &'static str {
        self.kind.as_api_type()
    }
}

/// Any add-on resource path, from its three parts — **the one formatter**.
///
/// ```text
///   ("stream",  "series", "tt0903747:1:4") → "stream/series/tt0903747%3A1%3A4.json"
///   ("catalog", "movie",  "top")           → "catalog/movie/top.json"
/// ```
///
/// Two expressions in `addonClient.ts` build this string —
/// `'stream/' + type + '/' + encodeURIComponent(videoId) + '.json'` (`:118`) and
/// `` `catalog/${c.type}/${encodeURIComponent(c.id)}.json` `` (`:161`) — and they
/// are the same expression with different words in it. They are one function here,
/// and [`MediaRef::resource_path`] is the typed convenience that works out the
/// `kind` and the `id` from a reference before calling it, rather than a second
/// implementation of the format.
///
/// The catalog case is why this takes a bare `&str` id and not a [`MediaRef`]: a
/// catalog id (`"top"`, `"trending"`) names a *list*, not a title, and is not
/// parseable as a media reference at all.
pub fn addon_resource_path(resource: &str, media_type: &str, id: &str) -> String {
    format!("{resource}/{media_type}/{}.json", encode_uri_component(id))
}

/// JavaScript's `encodeURIComponent`, exactly.
///
/// Unreserved set `A-Za-z0-9 - _ . ! ~ * ' ( )`; everything else becomes `%XX`
/// per UTF-8 byte, uppercase hex. Hand-rolled rather than pulled from
/// `percent-encoding` because this crate's entire dependency surface is
/// `serde` + `serde_json` and one twelve-line function is not worth spending
/// that budget on — the `percent-encoding` crate would also arrive with `url`
/// behind it in practice, which is how stremio-core reaches 183 lockfile entries.
///
/// One difference from the JS, and it is in our favour: `encodeURIComponent`
/// throws `URIError` on a lone surrogate. A Rust `&str` cannot hold one, so this
/// function is total where the original is not.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            out.push(char::from(b));
        } else {
            // `format!` rather than a hex table: a table needs indexing, and this
            // crate denies `indexing_slicing`. The cost is an allocation per
            // escaped byte on strings that are never longer than an id.
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Strip a leading `tmdb:` / `imdb:` namespace prefix, case-insensitively.
fn split_source_prefix(s: &str) -> Option<(MediaSource, &str)> {
    for (prefix, source) in [("tmdb:", MediaSource::Tmdb), ("imdb:", MediaSource::Imdb)] {
        if let Some((head, tail)) = s.split_at_checked(prefix.len()) {
            if head.eq_ignore_ascii_case(prefix) {
                return Some((source, tail));
            }
        }
    }
    None
}

/// Infer the namespace from the id's own shape. `tt` + digits is IMDb's format;
/// an all-decimal id is TMDB's. Anything else is unnameable — see [`MediaSource`].
fn infer_source(id: &str) -> Option<MediaSource> {
    if let Some(digits) = id.strip_prefix("tt") {
        if is_decimal(digits) {
            return Some(MediaSource::Imdb);
        }
        return None;
    }
    if is_decimal(id) {
        return Some(MediaSource::Tmdb);
    }
    None
}

/// `S<season>E<episode>`, either case. Returns `None` on anything else.
fn parse_episode_token(token: &str) -> Option<(u32, u32)> {
    let rest = token.strip_prefix(['S', 's'])?;
    let split = rest.find(['E', 'e'])?;
    let season = rest.get(..split)?;
    let episode = rest.get(split.checked_add(1)?..)?;
    Some((decimal_u32(season)?, decimal_u32(episode)?))
}

/// Parse a run of ASCII decimal digits. Stricter than `str::parse::<u32>()`, which
/// accepts a leading `+` — so `S+1E4` would otherwise round-trip to `S1E4` and
/// silently change the key a resume position is stored under.
fn decimal_u32(s: &str) -> Option<u32> {
    if !is_decimal(s) {
        return None;
    }
    s.parse::<u32>().ok()
}

/// Non-empty, and every byte an ASCII decimal digit.
fn is_decimal(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imdb_movie(id: &str) -> MediaRef {
        MediaRef {
            source: MediaSource::Imdb,
            id: id.to_string(),
            kind: MediaKind::Movie,
            season: None,
            episode: None,
        }
    }

    #[test]
    fn parses_every_documented_input_shape() {
        assert_eq!(
            MediaRef::parse("tt0111161", None),
            Some(imdb_movie("tt0111161"))
        );

        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(ep.source, MediaSource::Imdb);
        assert_eq!(ep.id, "tt0903747");
        assert_eq!(ep.kind, MediaKind::Series);
        assert_eq!((ep.season, ep.episode), (Some(1), Some(4)));

        // The two key formats must decompose identically — that is the property
        // that lets one function serve the resume store and the add-on wire.
        assert_eq!(MediaRef::parse("tt0903747:S1E4", None), Some(ep));

        let tmdb = MediaRef::parse("tmdb:12345", None).unwrap();
        assert_eq!(tmdb.source, MediaSource::Tmdb);
        assert_eq!(tmdb.id, "12345");

        // A bare numeric id is TMDB's — this is what /api/home hands the shell.
        assert_eq!(
            MediaRef::parse("12345", None).unwrap().source,
            MediaSource::Tmdb
        );

        // ...and DetailModal writes `12345:S1E4` for a server-sourced series.
        let tmdb_ep = MediaRef::parse("12345:S1E4", None).unwrap();
        assert_eq!(tmdb_ep.source, MediaSource::Tmdb);
        assert_eq!(tmdb_ep.id, "12345");
        assert_eq!(tmdb_ep.kind, MediaKind::Series);
    }

    #[test]
    fn namespace_prefixes_are_case_insensitive_and_symmetric() {
        assert_eq!(
            MediaRef::parse("TMDB:12345", None).unwrap().source,
            MediaSource::Tmdb
        );
        assert_eq!(
            MediaRef::parse("imdb:tt1", None).unwrap().source,
            MediaSource::Imdb
        );
        // An explicit prefix overrides what the id's shape would have implied.
        assert_eq!(
            MediaRef::parse("tmdb:tt1", None).unwrap().source,
            MediaSource::Tmdb
        );
        assert_eq!(MediaRef::parse("tmdb:tt1", None).unwrap().id, "tt1");
    }

    #[test]
    fn episode_coordinates_outrank_the_kind_hint() {
        // A stale `type: 'movie'` on a record that names an episode must not turn
        // the episode into a film — the coordinates are evidence, the hint is not.
        let r = MediaRef::parse("tt1:2:3", Some(MediaKind::Movie)).unwrap();
        assert_eq!(r.kind, MediaKind::Series);
        // With no coordinates, the hint is all there is.
        assert_eq!(
            MediaRef::parse("tt1", Some(MediaKind::Series))
                .unwrap()
                .kind,
            MediaKind::Series
        );
        assert_eq!(MediaRef::parse("tt1", None).unwrap().kind, MediaKind::Movie);
    }

    #[test]
    fn rejects_what_it_cannot_name() {
        for raw in [
            "",
            "   ",
            "kitsu:12345", // a namespace this enum does not model
            "ttabc",       // `tt` but not IMDb's shape
            "tt",          // `tt` with no digits
            "not-an-id",
            "tt1:S1",    // half an episode token
            "tt1:SxEy",  // non-decimal coordinates
            "tt1:1:2:3", // more parts than the format has
            "tt1:",      // trailing separator, empty token
            ":1:2",      // no id
            "tt1:+1:4",  // `str::parse::<u32>` would accept the `+`
            "tt1:S+1E4",
        ] {
            assert_eq!(MediaRef::parse(raw, None), None, "should reject {raw:?}");
        }
    }

    #[test]
    fn parse_trims_and_accepts_lowercase_episode_tokens() {
        assert_eq!(
            MediaRef::parse("  tt0903747:s1e4  ", None),
            MediaRef::parse("tt0903747:S1E4", None)
        );
    }

    #[test]
    fn renders_both_key_formats() {
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(ep.media_key(), "tt0903747:S1E4");
        assert_eq!(ep.stream_video_id(), "tt0903747:1:4");
        assert!(ep.is_episode());

        let film = MediaRef::parse("tt0111161", None).unwrap();
        assert_eq!(film.media_key(), "tt0111161");
        assert_eq!(film.stream_video_id(), "tt0111161");
        assert!(!film.is_episode());
    }

    /// Both renderings must feed back into `parse` unchanged. This is what makes
    /// it safe for one part of the app to hand a key to another part that only
    /// knows the other format.
    #[test]
    fn both_key_formats_round_trip_through_parse() {
        for raw in [
            "tt0111161",
            "tt0903747:1:4",
            "tt0903747:S1E4",
            "12345",
            "12345:S1E4",
        ] {
            let r = MediaRef::parse(raw, None).unwrap();
            assert_eq!(MediaRef::parse(&r.media_key(), None).as_ref(), Some(&r));
            assert_eq!(
                MediaRef::parse(&r.stream_video_id(), None).as_ref(),
                Some(&r)
            );
        }
    }

    /// Pins the exact bytes `addonClient.ts:118` produces:
    /// `'stream/' + type + '/' + encodeURIComponent(videoId) + '.json'`.
    #[test]
    fn resource_path_matches_the_typescript_twin() {
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(
            ep.resource_path("stream"),
            "stream/series/tt0903747%3A1%3A4.json"
        );
        assert_eq!(
            ep.resource_path("subtitles"),
            "subtitles/series/tt0903747%3A1%3A4.json"
        );

        let film = MediaRef::parse("tt0111161", None).unwrap();
        assert_eq!(film.resource_path("stream"), "stream/movie/tt0111161.json");
    }

    /// A catalog id is not a media id, which is why the formatter takes three
    /// strings rather than a reference — `addonClient.ts:161`.
    #[test]
    fn the_formatter_also_builds_a_catalog_path() {
        assert_eq!(
            addon_resource_path("catalog", "movie", "top"),
            "catalog/movie/top.json"
        );
        // Ids with separators are encoded the same way everywhere.
        assert_eq!(
            addon_resource_path("catalog", "series", "trending/all"),
            "catalog/series/trending%2Fall.json"
        );
        // ...and it is genuinely the same code the typed path runs.
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(
            ep.resource_path("stream"),
            addon_resource_path("stream", "series", "tt0903747:1:4")
        );
    }

    /// `meta` describes the show, not the episode. Carrying the coordinates into
    /// this path is a 404 at the add-on, and a silent one.
    #[test]
    fn meta_is_title_scoped() {
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(ep.resource_path("meta"), "meta/series/tt0903747.json");
        assert_eq!(ep.resource_path("catalog"), "catalog/series/tt0903747.json");
    }

    #[test]
    fn encode_uri_component_matches_javascript() {
        // The unreserved set is passed through verbatim.
        assert_eq!(encode_uri_component("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        // Uppercase hex, per byte, exactly as encodeURIComponent emits it.
        assert_eq!(encode_uri_component("tt1:1:4"), "tt1%3A1%3A4");
        assert_eq!(encode_uri_component("a b/c?d&e=f"), "a%20b%2Fc%3Fd%26e%3Df");
        assert_eq!(encode_uri_component("+"), "%2B");
        // Multi-byte input escapes per UTF-8 byte, not per char.
        assert_eq!(encode_uri_component("ა"), "%E1%83%90");
        assert_eq!(encode_uri_component(""), "");
    }

    #[test]
    fn kind_from_wire_tolerates_both_vocabularies() {
        for s in ["tv", "series", "show", "TV", " Series "] {
            assert_eq!(MediaKind::from_wire(s), MediaKind::Series, "{s:?}");
        }
        for s in ["movie", "film", "", "channel", "nonsense"] {
            assert_eq!(MediaKind::from_wire(s), MediaKind::Movie, "{s:?}");
        }
    }

    #[test]
    fn kind_hint_distinguishes_absent_from_movie() {
        assert_eq!(MediaKind::from_wire_hint(""), None);
        assert_eq!(MediaKind::from_wire_hint("   "), None);
        assert_eq!(MediaKind::from_wire_hint("tv"), Some(MediaKind::Series));
        assert_eq!(MediaKind::from_wire_hint("movie"), Some(MediaKind::Movie));
    }

    #[test]
    fn api_type_is_the_derived_dialect() {
        assert_eq!(MediaKind::Series.as_wire(), "series");
        assert_eq!(MediaKind::Series.as_api_type(), "tv");
        assert_eq!(MediaKind::Movie.as_wire(), "movie");
        assert_eq!(MediaKind::Movie.as_api_type(), "movie");

        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(ep.api_type(), "tv");
    }

    #[test]
    fn wire_shape_omits_absent_coordinates() {
        let film = MediaRef::parse("tt0111161", None).unwrap();
        assert_eq!(
            serde_json::to_string(&film).unwrap(),
            r#"{"source":"imdb","id":"tt0111161","kind":"movie"}"#
        );
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        assert_eq!(
            serde_json::to_string(&ep).unwrap(),
            r#"{"source":"imdb","id":"tt0903747","kind":"series","season":1,"episode":4}"#
        );
    }

    #[test]
    fn wire_shape_round_trips_and_defaults_kind() {
        let ep = MediaRef::parse("tt0903747:1:4", None).unwrap();
        let json = serde_json::to_string(&ep).unwrap();
        assert_eq!(serde_json::from_str::<MediaRef>(&json).unwrap(), ep);

        let minimal: MediaRef = serde_json::from_str(r#"{"source":"tmdb","id":"12345"}"#).unwrap();
        assert_eq!(minimal.kind, MediaKind::Movie);
        assert_eq!(minimal.season, None);
    }

    #[test]
    fn source_wire_tokens() {
        assert_eq!(MediaSource::Imdb.as_wire(), "imdb");
        assert_eq!(MediaSource::Tmdb.as_wire(), "tmdb");
    }

    /// The separator is one constant with one definition; `library` re-exports it
    /// rather than owning a second copy.
    #[test]
    fn separator_is_shared_with_library() {
        assert_eq!(MEDIA_KEY_SEPARATOR, ":");
        assert_eq!(MEDIA_KEY_SEPARATOR, crate::library::MEDIA_KEY_SEPARATOR);
    }

    /// `library::remove` sweeps progress by the `{id}:` prefix. Both key formats
    /// must therefore stay sweepable, and neither may be a prefix of a *different*
    /// title's key.
    #[test]
    fn keys_stay_compatible_with_the_library_prefix_sweep() {
        let ep = MediaRef::parse("tt1:1:4", None).unwrap();
        let prefix = format!("tt1{MEDIA_KEY_SEPARATOR}");
        assert!(ep.media_key().starts_with(&prefix));
        assert!(ep.stream_video_id().starts_with(&prefix));
        assert!(!MediaRef::parse("tt10:1:4", None)
            .unwrap()
            .media_key()
            .starts_with(&prefix));
    }
}
