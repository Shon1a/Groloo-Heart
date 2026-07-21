//! Core domain types — the shared data model every GROLOO client speaks.
//!
//! These mirror the JSON shapes used across the platform (the official add-on
//! collection, an add-on's own `manifest.json`, per-account install state, and the
//! `/api/library-state` document) so the same bytes flow from CDN → core → any
//! shell with zero transformation.
//!
//! VENDORED from `Groloo-Heart@0.1.0 src/types.rs`. Everything Heart modelled is
//! still here and still means the same thing; what changed are the five
//! corrections the Phase 1 contract calls for, each of which exists because the
//! *absence* of it is currently costing the shell something concrete:
//!
//! - [`Progress::lang`] — Heart had no place to put the audio/subtitle language,
//!   so `history.ts` had to re-attach it after every core call (`keepLangs`), and
//!   forgot to on two of the four paths. The language belongs next to the position
//!   it was recorded with, because it is written and read on the same beat.
//! - `Progress` is no longer `Copy`. `String` is not `Copy`; this is a compile
//!   error rather than a runtime surprise, which is the point.
//! - `MetaItem::id`/`year`/`rating` tolerate a JSON number. The TypeScript types
//!   have always said `string | number`; only the server's `String(m.id)` keeps a
//!   numeric id from reaching us today, and a third-party add-on's catalog is not
//!   bound by that. One such row used to fail the whole-document deserialize and,
//!   through `unwrap_or_default()`, wipe the user's entire library.
//! - `Progress::is_finished` is gone. It used 0.9; the shell has always used 0.94.
//!   Two thresholds for one idea is one too many — see `library::RESUME_DONE_FRACTION`.
//! - [`AddonManifest::catalogs`] is new. Heart did not model it at all, which is
//!   why catalog enumeration had to be re-derived in TypeScript.
//!
//! Nothing else moves. The differential harness can only pin behaviour that holds
//! still.

use serde::{Deserialize, Serialize};

/// The only `schema` major the collection loader understands. Anything else is a
/// newer server talking to an older client, and the correct answer is to keep the
/// inline defaults rather than guess (version negotiation, not version tolerance).
pub const COLLECTION_SCHEMA: u32 = 1;

/// Which shelf a card belongs to. `official` cards ship curated; `community`
/// cards are added by the user (by URL) at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    #[default]
    Official,
    Community,
}

/// Capability/marker flags carried alongside a descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official: Option<bool>,
    /// `true` marks a behaviour-critical entry a merge must never drop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
}

/// A single official/curated add-on card.
///
/// Dual-purpose by design: it carries a canonical core (`id`, `version`,
/// `types`, `resources`, `flags`, `kind`) that a future device shell can consume
/// natively, **plus** the flat UI hints today's clients read directly (`ver`,
/// `icon_cls`, `glyph`, `tags`, `default_installed`, ...). Absent fields stay
/// `None`/empty so a partial CDN record never wipes an inline default.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AddonDescriptor {
    /// Required in every sense that matters — a record without one cannot be
    /// merged, updated or keyed — but `#[serde(default)]` rather than mandatory,
    /// because *where* it is enforced decides what one bad record costs.
    ///
    /// Mandatory here means the missing id fails the whole `addons` array, so one
    /// malformed CDN record takes the entire official collection down with it and
    /// [`crate::collection::SkipReason::NoId`] — the guard written for exactly this
    /// input — is unreachable by construction. Defaulting to `""` moves the
    /// enforcement into the merge, where it already lives, where it costs that one
    /// record, and where it produces a `skipped.no_id` warning naming it.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub section: Section,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub name: String,

    /// Canonical semantic version, no leading `v` (e.g. `1.3.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// UI-facing version string, with leading `v` (e.g. `v1.3.0`). Derived from
    /// `version` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ver: Option<String>,

    #[serde(rename = "iconCls", default, skip_serializing_if = "Option::is_none")]
    pub icon_cls: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<String>,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub tags: Vec<String>,

    /// Default install state — a *hint* only, honoured for genuinely new ids.
    /// Real per-account state lives in [`crate::state`], not here.
    #[serde(
        rename = "defaultInstalled",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_installed: Option<bool>,
    /// Legacy/runtime install flag (tolerated on input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,

    #[serde(rename = "noConfig", default, skip_serializing_if = "Option::is_none")]
    pub no_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,

    /// `"discovery"` marks a metadata-only card (no stream sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub types: Vec<String>,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub resources: Vec<String>,

    /// Present only if a card is (or becomes) a real network add-on. Its presence
    /// means "carries a transport", which the neutral-conduit merge rejects for
    /// curated official cards.
    #[serde(
        rename = "transportUrl",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub transport_url: Option<String>,
    #[serde(rename = "configRef", default, skip_serializing_if = "Option::is_none")]
    pub config_ref: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<Flags>,
}

/// One entry in the manifest's `collections[]` — points at a payload file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionRef {
    pub id: String,
    #[serde(default)]
    pub section: Section,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(rename = "protectedIds", default)]
    pub protected_ids: Vec<String>,
}

/// The small, eager-loaded manifest (`index.json`). Lists the payload files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionManifest {
    /// Schema major. Consumers ignore any value other than [`COLLECTION_SCHEMA`]
    /// and fall back to their inline defaults (version negotiation).
    pub schema: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdn: Option<String>,
    #[serde(default)]
    pub collections: Vec<CollectionRef>,
}

impl CollectionManifest {
    /// The `official` collection reference, if the manifest is understood.
    pub fn official(&self) -> Option<&CollectionRef> {
        if self.schema != COLLECTION_SCHEMA {
            return None;
        }
        self.collections
            .iter()
            .find(|c| c.section == Section::Official && !c.file.is_empty())
    }
}

/// A collection payload file (`addons.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddonCollection {
    pub schema: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default)]
    pub section: Section,
    #[serde(default)]
    pub addons: Vec<AddonDescriptor>,
}

/// A `resources` entry in a real add-on `manifest.json` — either a short string
/// (`"stream"`) or the full form (`{ "name": "stream", "types": [...] }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Short(String),
    Full {
        name: String,
        #[serde(default)]
        types: Vec<String>,
        #[serde(rename = "idPrefixes", default)]
        id_prefixes: Vec<String>,
    },
}

impl Resource {
    pub fn name(&self) -> &str {
        match self {
            Resource::Short(s) => s,
            Resource::Full { name, .. } => name,
        }
    }
    pub fn types(&self) -> &[String] {
        match self {
            Resource::Short(_) => &[],
            Resource::Full { types, .. } => types,
        }
    }
}

/// One declared catalog in an add-on's manifest (`catalogs[]`).
///
/// NEW — Heart modelled `resources` and `types` but not this, so every shell that
/// wanted to *enumerate* an add-on's catalogs had to walk the raw JSON itself.
/// Only the three fields a picker needs are modelled; `extra`/`extraSupported`
/// are request-shaping concerns and belong to whoever builds the request.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AddonCatalogDecl {
    #[serde(rename = "type", default, deserialize_with = "de::null_to_default")]
    pub catalog_type: String,
    #[serde(default, deserialize_with = "de::string_or_number_default")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A real third-party add-on's own `manifest.json` (the thing a user installs by
/// URL). Only the fields the core needs are modelled.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AddonManifest {
    #[serde(default, deserialize_with = "de::string_or_number_default")]
    pub id: String,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "de::lenient_vec")]
    pub resources: Vec<Resource>,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub types: Vec<String>,
    #[serde(default, deserialize_with = "de::lenient_vec")]
    pub catalogs: Vec<AddonCatalogDecl>,
}

/// A poster/catalog item — the shape a home row or search result renders.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MetaItem {
    /// Required. Tolerates a JSON number on input (a third-party catalog is under
    /// no obligation to stringify), but is always a string once inside the core —
    /// every map key, tombstone and media key in the system is a string.
    #[serde(deserialize_with = "de::string_or_number")]
    pub id: String,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub year: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    /// Stays `String` deliberately. The shell has always round-tripped it as one
    /// (`heartLibrary.ts` stringifies on the way in and parses on the way out), so
    /// "fixing" it to `f64` would change bytes the differential harness is pinning.
    /// It only gained *input* tolerance, which matches what the shell already did.
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ep: Option<String>,
}

/// A library / history entry — a [`MetaItem`] plus per-user watch bookkeeping.
/// Matches the flat shape persisted per account (id/title/poster/... + at/key/season/episode).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LibraryItem {
    #[serde(flatten)]
    pub meta: MetaItem,
    /// Stable render key some rows use (usually equal to `meta.id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Last-touched timestamp (ms) — drives recency ordering and merge.
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
}

impl LibraryItem {
    /// The identity a tombstone, a history slot and a merge are all keyed by.
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    /// The key its *resume position* is stored under — `key` when the shell set
    /// one (episodes carry `id:S#E#`), else the bare id.
    ///
    /// This is a core concept and not a shell one on purpose: `library::remove`
    /// cannot sweep an episode's progress unless the core knows how episode keys
    /// are spelled.
    pub fn media_key(&self) -> &str {
        match self.key.as_deref() {
            Some(k) if !k.is_empty() => k,
            _ => self.id(),
        }
    }
}

/// An entry in the user's My List (watchlist).
///
/// NEW to the core, but not new to the platform: this is `WatchItem` from
/// `stores/library.ts`, the fourth copy of the same merge-by-recency CRDT the
/// history store, the server and Heart each implement separately. Modelling it
/// here is what lets one merge serve all of them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MyListItem {
    #[serde(deserialize_with = "de::string_or_number")]
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(
        default,
        deserialize_with = "de::opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub year: Option<String>,
    /// Passed through untouched. The core never reasons about a rating, and the
    /// My List store writes it as a number — so anything other than verbatim
    /// round-tripping would be the core corrupting data it does not use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    /// Added-at (ms) — the merge's recency signal, exactly as in `history`.
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub at: u64,
}

/// Resume position for one media key (seconds), with the time it was recorded (ms).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub pos: f64,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub dur: f64,
    #[serde(default, deserialize_with = "de::null_to_default")]
    pub at: u64,
    /// The audio/source language the user was actually watching.
    ///
    /// It lives here, next to the position it belongs to, because it is written
    /// and read on the same beat as `pos`. Heart's omission of this one field is
    /// the entire reason `history.ts::keepLangs()` exists — and because that
    /// helper is only called from two of the four write paths, recording a watch
    /// or removing a title silently discards the saved audio track for every
    /// entry in the map. The field deletes the helper and the bug together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl Progress {
    /// Fraction watched in `[0, 1]` (0 when the duration is unknown).
    pub fn fraction(&self) -> f64 {
        if self.dur > 0.0 {
            (self.pos / self.dur).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Deserialisation helpers.
///
/// Every one of these exists to answer the same question the same way: **what
/// should the core do with one row it cannot read?** Heart's answer was to fail
/// the whole document, which — through `hydrate()`'s `unwrap_or_default()` —
/// turned a single unparseable history entry into an empty library that the shell
/// then wrote back as truth. The answer here is to drop the row and keep the
/// document, because a library missing one title is recoverable and a library
/// missing everything is not.
pub(crate) mod de {
    use serde::de::{DeserializeOwned, Deserializer, Visitor};
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::fmt;

    /// `null` (or absent) means "the default", not "a parse error". JSON written
    /// by a browser is full of explicit nulls where a field was simply never set.
    pub(crate) fn null_to_default<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Default,
    {
        Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
    }

    /// Accept a JSON string *or* number where the core wants a string.
    ///
    /// `lib/types.ts` has always typed ids and years as `string | number`; only
    /// the server's `String(m.id)` keeps the numeric case from reaching us today,
    /// and a third-party add-on's catalog is not bound by that.
    pub(crate) fn string_or_number<'de, D>(d: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        d.deserialize_any(StringOrNumber)
    }

    /// As [`string_or_number`], but a missing/null field yields `""`.
    pub(crate) fn string_or_number_default<'de, D>(d: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(opt_string_or_number(d)?.unwrap_or_default())
    }

    /// As [`string_or_number`], for an optional field. `null` → `None`.
    pub(crate) fn opt_string_or_number<'de, D>(d: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Option::<Coerced>::deserialize(d)?.map(|c| c.0))
    }

    /// Newtype so `Option<_>` can drive [`string_or_number`] for us.
    #[derive(Deserialize)]
    struct Coerced(#[serde(deserialize_with = "string_or_number")] String);

    struct StringOrNumber;

    impl Visitor<'_> for StringOrNumber {
        type Value = String;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string or a number")
        }
        fn visit_str<E>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_string<E>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_u64<E>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_i64<E>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
        /// `2019.0` must render as `2019`, not `2019.0` — JSON has no integer
        /// type, so a whole number arriving as a float is the common case, and
        /// `"2019.0"` would not match anything the shell stored.
        fn visit_f64<E>(self, v: f64) -> Result<String, E> {
            if v.is_finite() && v.fract() == 0.0 {
                Ok((v.trunc() as i64).to_string())
            } else {
                Ok(v.to_string())
            }
        }
    }

    /// A list where an unreadable element is dropped, not fatal. A `null` list is
    /// an empty list.
    pub(crate) fn lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let raw = Option::<Vec<serde_json::Value>>::deserialize(d)?;
        Ok(raw
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect())
    }

    /// [`lenient_vec`] for a field where **"the caller sent no array" and "the
    /// caller sent an empty array" are different answers**.
    ///
    /// Exactly JavaScript's `Array.isArray(x) ? … : undefined`: only an array
    /// produces `Some`, and `null`, a string, an object or an absent field all
    /// produce `None` *without* failing the record. That last part is why this is
    /// a deserializer and not `Option<Vec<T>>`: serde would reject
    /// `"subtitles": "none"` and cost the whole stream, where the twin shrugs and
    /// renders it with no subtitle tracks.
    pub(crate) fn array_or_none<'de, D, T>(d: D) -> Result<Option<Vec<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Array(items) => Ok(Some(
                items
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect(),
            )),
            _ => Ok(None),
        }
    }

    /// An **optional field** where a value the core cannot read costs *the field*,
    /// not the record.
    ///
    /// The row-level rule ([`lenient_vec`], [`lenient_map`]) one layer further in.
    /// A `Option<T>` written the ordinary way makes serde fail the whole record
    /// when the value is present and of the wrong type, and for a field the core
    /// merely *carries* — `fileIdx`, `infoHash`, `behaviorHints.videoSize` — that
    /// trades a hint nobody would have missed for the entire row. On the stream
    /// path that row is a playable source, and a source that vanishes is invisible
    /// to the user: the title simply looks as though the add-on had nothing.
    ///
    /// Use it for anything the core stores but does not dereference. A field the
    /// core *acts* on (a URL it hands to a `<video>`) must stay strict, because
    /// there the wrong type is not a lost hint, it is a broken action.
    pub(crate) fn optional<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        // `null` fails `from_value::<T>` for every `T` this is used on and lands on
        // `None`, which is the same answer `Option<T>` would have given.
        Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).ok())
    }

    /// [`optional`] for a field that has a default rather than an absence: a value
    /// the core cannot read means "the default", never a failed record.
    ///
    /// Strictly wider than [`null_to_default`] — `null` is one of the values it
    /// cannot read — so it replaces it wherever the wire is a third party's rather
    /// than our own.
    pub(crate) fn default_on_error<'de, D, T>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned + Default,
    {
        Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).unwrap_or_default())
    }

    /// A string-keyed map where an unreadable value is dropped, not fatal. This is
    /// the `{"someKey": null}` case the server explicitly documents a client can
    /// PUT; under Heart it took the whole library with it.
    pub(crate) fn lenient_map<'de, D, T>(d: D) -> Result<BTreeMap<String, T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let raw = Option::<BTreeMap<String, serde_json::Value>>::deserialize(d)?;
        Ok(raw
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| serde_json::from_value(v).ok().map(|t| (k, t)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_round_trips_lang() {
        let p: Progress =
            serde_json::from_str(r#"{"pos":30,"dur":100,"at":5,"lang":"eng"}"#).unwrap();
        assert_eq!(p.lang.as_deref(), Some("eng"));
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"pos":30.0,"dur":100.0,"at":5,"lang":"eng"}"#
        );
    }

    /// The field must vanish when absent rather than serialise as `null` — the
    /// shell stores this map verbatim in localStorage and an explicit `null`
    /// would be a new byte in every record that never had a language.
    #[test]
    fn progress_omits_absent_lang() {
        let p: Progress = serde_json::from_str(r#"{"pos":1,"dur":2,"at":3}"#).unwrap();
        assert_eq!(p.lang, None);
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"pos":1.0,"dur":2.0,"at":3}"#
        );
    }

    #[test]
    fn progress_tolerates_partial_and_null_fields() {
        let p: Progress = serde_json::from_str(r#"{"pos":9,"at":null}"#).unwrap();
        assert_eq!(p.pos, 9.0);
        assert_eq!(p.dur, 0.0);
        assert_eq!(p.at, 0);
    }

    #[test]
    fn meta_item_accepts_numeric_id_and_year() {
        let m: MetaItem =
            serde_json::from_str(r#"{"id":12345,"title":"X","year":2019,"rating":8.4}"#).unwrap();
        assert_eq!(m.id, "12345");
        assert_eq!(m.year.as_deref(), Some("2019"));
        assert_eq!(m.rating.as_deref(), Some("8.4"));
    }

    #[test]
    fn meta_item_still_accepts_strings() {
        let m: MetaItem =
            serde_json::from_str(r#"{"id":"tt1","title":"X","year":"2019","rating":"8.4"}"#)
                .unwrap();
        assert_eq!(m.id, "tt1");
        assert_eq!(m.year.as_deref(), Some("2019"));
        assert_eq!(m.rating.as_deref(), Some("8.4"));
    }

    #[test]
    fn library_item_media_key_prefers_key() {
        let it: LibraryItem =
            serde_json::from_str(r#"{"id":"tt1","key":"tt1:S1E2","at":5}"#).unwrap();
        assert_eq!(it.id(), "tt1");
        assert_eq!(it.media_key(), "tt1:S1E2");

        let bare: LibraryItem = serde_json::from_str(r#"{"id":"tt1","at":5}"#).unwrap();
        assert_eq!(bare.media_key(), "tt1");
    }

    #[test]
    fn manifest_models_catalogs() {
        let m: AddonManifest = serde_json::from_str(
            r#"{"id":"x","name":"X","resources":["catalog"],"types":["movie"],
                "catalogs":[{"type":"movie","id":"top","name":"Top"},
                            {"type":"series","id":"trend"}]}"#,
        )
        .unwrap();
        assert_eq!(m.catalogs.len(), 2);
        assert_eq!(m.catalogs[0].catalog_type, "movie");
        assert_eq!(m.catalogs[0].name.as_deref(), Some("Top"));
        assert_eq!(m.catalogs[1].name, None);
    }

    /// One malformed element must cost exactly that element. This is the shape of
    /// the whole "never wipe the library" contract, tested at its source.
    #[test]
    fn lenient_containers_drop_only_the_bad_row() {
        #[derive(serde::Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "de::lenient_vec")]
            items: Vec<MetaItem>,
            #[serde(default, deserialize_with = "de::lenient_map")]
            map: std::collections::BTreeMap<String, Progress>,
        }
        let d: Doc = serde_json::from_str(
            r#"{"items":[{"id":"a"},{"nope":true},{"id":"b"}],
                "map":{"a":{"pos":1,"dur":2,"at":3},"b":null,"c":"nonsense"}}"#,
        )
        .unwrap();
        let ids: Vec<&str> = d.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(d.map.keys().collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn null_list_is_an_empty_list() {
        let m: AddonManifest =
            serde_json::from_str(r#"{"id":"x","name":"X","resources":null,"types":null}"#).unwrap();
        assert!(m.resources.is_empty());
        assert!(m.types.is_empty());
    }
}
