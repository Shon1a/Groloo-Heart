//! An add-on catalog's `meta` entries → the poster-card shape the UI renders.
//!
//! This is `addonClient.ts:130-141`'s `mapCatalogMeta` plus the
//! `.filter(m => m.poster)` that always follows it (`:162`). Fifteen lines of
//! TypeScript, and almost all of the work here is reproducing what those fifteen
//! lines *mean in JavaScript* rather than what they appear to say.
//!
//! ## Why this module is full of `serde_json::Value`
//!
//! Because the twin does no coercion, and the twin rule says the bytes have to
//! match before it can be deleted. `mapCatalogMeta` copies `m.id` straight
//! through: if an add-on sends `"id": 12345`, a number comes out the other side,
//! and `lib/types.ts:9` types the field `string | number` precisely because that
//! happens. Deserialising into `String` here would be a *better* record and a
//! different one, and the difference would show up as a failed comparison in the
//! corpus rather than as a decision anybody made.
//!
//! So `id`, `title`, `genre` and `rating` are carried as [`Value`]. The fields
//! this core actually reasons about — `type` and `year` — are typed, because those
//! are the ones with a rule.
//!
//! ## The JavaScript this module is a translation of
//!
//! Four expressions, each of which is a trap:
//!
//! | TypeScript | What it actually does |
//! |---|---|
//! | `m.genres \|\| m.genre \|\| []` | *truthiness*, so `""`, `0` and `[]`… no: `[]` is truthy in JS, so an empty array short-circuits here and `genre` ends up `""` via `genres[0]` being `undefined` |
//! | `String(a \|\| b \|\| '').slice(0, 4)` | `slice` counts **UTF-16 code units**, and `releaseInfo` is routinely a range (`"2008-2013"` → `"2008"`) |
//! | `+parseFloat(x).toFixed(1)` | `parseFloat` takes the longest numeric *prefix* (`"8.5/10"` → `8.5`); a non-numeric string gives `NaN`, which `JSON.stringify` writes as **`null`**, not `0` |
//! | `m.poster \|\| undefined` | an absent key, not a `null` one — `JSON.stringify` drops `undefined` properties entirely |
//!
//! The `rating` case is the one worth staring at. `+parseFloat("abc").toFixed(1)`
//! is `+"NaN"`, which is `NaN`, which serialises as `null` — so a catalog with a
//! junk rating produces `"rating": null` in a field the shell's own type declares
//! as `number`. Reproduced here deliberately ([`js_number_value`]); the fix
//! belongs in the corpus as a declared divergence, not smuggled in under a port.
//!
//! ## The one deliberate divergence: `type`
//!
//! `addonClient.ts:134` is `m.type === 'series' ? 'series' : 'movie'` — a strict
//! equality against one token, so an add-on that labels a show `"tv"` gets
//! `"movie"`, is rendered with a film's chrome, and has its streams requested from
//! `stream/movie/…`, which returns nothing. Here the token goes through
//! [`MediaKind::from_wire`], which reads `tv`, `series` and `show` alike. This is
//! the `movie|tv` vs `movie|series` split the increment exists to close, and it is
//! a **byte divergence** on exactly one input class (`type: "tv"`), to be declared
//! in the corpus rather than hidden.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::media::MediaKind;
use crate::stream::is_js_space;

/// One `meta` entry as an add-on's `catalog` resource returns it.
///
/// Only the eight fields the mapping reads are modelled; everything else an
/// add-on sends is ignored rather than round-tripped, per the boundary's rule
/// that a field the core does not model is a field the core drops.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogMeta {
    #[serde(default)]
    pub id: Value,
    #[serde(default)]
    pub name: Value,
    #[serde(default)]
    pub poster: Value,
    /// Stremio's own field. Frequently a range — `"2008-2013"` — which is why the
    /// twin slices it to four characters rather than parsing it.
    #[serde(default)]
    pub release_info: Value,
    #[serde(default)]
    pub year: Value,
    #[serde(default)]
    pub imdb_rating: Value,
    /// Either a list or a bare string, and add-ons use both spellings.
    #[serde(default)]
    pub genres: Value,
    #[serde(default)]
    pub genre: Value,
    #[serde(rename = "type", default)]
    pub item_type: Value,
}

/// A poster card — `lib/types.ts`'s `MediaItem`, restricted to the seven fields
/// `mapCatalogMeta` sets.
///
/// Field order is the twin's object literal order (`id`, `type`, `title`, `year`,
/// `rating`, `genre`, `poster`) because the harness compares bytes. `poster` is
/// the only omittable one: the twin writes `undefined`, which `JSON.stringify`
/// deletes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogItem {
    pub id: Value,
    /// `"movie"` or `"series"` — the canonical vocabulary. See the module doc for
    /// why this is not `m.type === 'series' ? …`.
    #[serde(rename = "type")]
    pub item_type: &'static str,
    pub title: Value,
    pub year: String,
    /// A JSON number, or `null` when the add-on's rating was not a number at all.
    pub rating: Value,
    pub genre: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster: Option<Value>,
}

impl CatalogItem {
    /// Whether this card can be rendered — `addonClient.ts:162`'s
    /// `.filter((m) => m.poster)`. A card with no artwork is a hole in a row.
    pub fn has_poster(&self) -> bool {
        self.poster.is_some()
    }
}

/// Map one catalog entry to a poster card — `addonClient.ts:130`.
pub fn map_catalog_meta(m: &CatalogMeta) -> CatalogItem {
    // `m.genres || m.genre || []`. Note that an empty ARRAY is truthy in
    // JavaScript, so `genres: []` wins the `||` chain and then yields `undefined`
    // at `[0]` — which is why the final `|| ''` is not redundant.
    let genres = first_truthy(&[&m.genres, &m.genre]).unwrap_or(&Value::Null);
    let genre = match genres {
        Value::Array(list) => list.first().unwrap_or(&Value::Null),
        other => other,
    };

    CatalogItem {
        id: m.id.clone(),
        item_type: MediaKind::from_wire(m.item_type.as_str().unwrap_or_default()).as_wire(),
        title: if is_truthy(&m.name) {
            m.name.clone()
        } else {
            Value::String("Untitled".to_string())
        },
        year: slice_utf16_4(&js_string(
            first_truthy(&[&m.release_info, &m.year]).unwrap_or(&Value::Null),
        )),
        rating: if is_truthy(&m.imdb_rating) {
            js_number_value(js_to_fixed_1(js_parse_float(&js_string(&m.imdb_rating))))
        } else {
            Value::from(0)
        },
        genre: if is_truthy(genre) {
            genre.clone()
        } else {
            Value::String(String::new())
        },
        poster: is_truthy(&m.poster).then(|| m.poster.clone()),
    }
}

/// Map a whole `catalog/{type}/{id}.json` response and drop the cards that cannot
/// be rendered — `addonClient.ts:162`'s `.map(mapCatalogMeta).filter(m => m.poster)`
/// as one pass.
///
/// One unreadable entry costs that entry, never the row: the same rule
/// [`crate::stream::map_addon_streams`] applies one layer down.
pub fn map_catalog_metas(metas: &[CatalogMeta]) -> Vec<CatalogItem> {
    metas
        .iter()
        .map(map_catalog_meta)
        .filter(CatalogItem::has_poster)
        .collect()
}

/// The body of a `catalog` response. `metas` absent, `null` or holding an
/// unreadable element all degrade rather than fail.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CatalogResponse {
    #[serde(default, deserialize_with = "crate::types::de::lenient_vec")]
    pub metas: Vec<CatalogMeta>,
}

// ---------------------------------------------------------------------------
// JavaScript semantics, spelled out
// ---------------------------------------------------------------------------

/// JavaScript truthiness for a JSON value: everything except `null`, `false`,
/// `0`, `NaN` and `""`. Note that `[]` and `{}` are **true**.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `a || b` over a list, returning the first truthy operand.
fn first_truthy<'a>(candidates: &[&'a Value]) -> Option<&'a Value> {
    candidates.iter().copied().find(|v| is_truthy(v))
}

/// `String(x)` for the shapes a catalog actually produces.
///
/// Strings pass through; numbers use Rust's shortest-round-trip formatting, which
/// agrees with JavaScript's for every value in range (`2008.0` renders `2008` in
/// both). Arrays and objects — for which JS would produce `"1,2"` and
/// `"[object Object]"` — render as `""`, a deliberate simplification: this feeds
/// a four-character year slice, and no catalog in the corpus puts a container
/// there. If one ever does, this is the function to grow.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// `s.slice(0, 4)` — **UTF-16 code units**, which is what JavaScript counts and
/// what a `chars()` port would silently get wrong on any non-BMP input.
fn slice_utf16_4(s: &str) -> String {
    let units: Vec<u16> = s.encode_utf16().take(4).collect();
    String::from_utf16_lossy(&units)
}

/// `parseFloat(s)` — the **longest numeric prefix**, `NaN` when there is none.
///
/// `parseFloat` is not `Number()`: it stops at the first character it cannot use
/// instead of failing, so `"8.5/10"` is `8.5` and `"8.5 GB"` is `8.5`. An add-on
/// writing `imdbRating: "8.5/10"` therefore gets a rating, which is worth
/// preserving rather than tightening.
fn js_parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_space);
    let bytes = t.as_bytes();
    let mut i = 0usize;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    if t.get(i..).is_some_and(|r| r.starts_with("Infinity")) {
        return if t.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }

    let digits_from = |mut j: usize| -> usize {
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j = j.saturating_add(1);
        }
        j
    };

    let int_end = digits_from(i);
    let mut end = int_end;
    if bytes.get(end) == Some(&b'.') {
        end = digits_from(end.saturating_add(1));
    }
    // At least one digit somewhere in the mantissa, or this is not a number.
    if end == i || (int_end == i && end == i.saturating_add(1)) {
        return f64::NAN;
    }
    // The exponent is taken only if it is complete — `"1e"` is `1`, not an error.
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut k = end.saturating_add(1);
        if matches!(bytes.get(k), Some(b'+' | b'-')) {
            k = k.saturating_add(1);
        }
        let exp_end = digits_from(k);
        if exp_end > k {
            end = exp_end;
        }
    }
    t.get(..end)
        .and_then(|n| n.parse::<f64>().ok())
        .unwrap_or(f64::NAN)
}

/// `+x.toFixed(1)` — round to one decimal place and come back as a number.
///
/// **Not `format!("{x:.1}")`.** ECMA-262 defines `toFixed` as: take the sign off
/// first, then "let `n` be an integer for which `n / 10^f - x` is as close to zero
/// as possible; if there are two such `n`, pick the **larger**". Applied to the
/// magnitude, that is round-half-**away-from-zero** — `(-8.25).toFixed(1)` is
/// `"-8.3"`, which is worth checking against a real engine before assuming
/// otherwise. Rust's formatter rounds half to **even**. That difference is not
/// theoretical: a tie needs `x = j/4` for odd `j`, which is a dyadic rational and
/// therefore exactly representable, so `8.25` is a real f64 and the two rules
/// disagree about it (`"8.3"` vs `"8.2"`). An add-on writing `imdbRating: "8.25"`
/// is not exotic.
///
/// Non-ties are decided on the value's **exact** decimal expansion, which is what
/// the spec says and what makes `8.15` round *up* — the nearest f64 to 8.15 is
/// very slightly above it. Thirty digits is more than enough to separate a genuine
/// tie from a near-one at any magnitude a rating reaches (the gap between adjacent
/// f64s near 8 is ~1.8e-15).
///
/// Above `1e21` `toFixed` gives up and returns `String(x)`, which round-trips
/// unchanged — reproduced for completeness, not because a rating will reach it.
fn js_to_fixed_1(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 1e21 {
        return x;
    }
    let negative = x.is_sign_negative();
    let exact = format!("{:.30}", x.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let kept = frac.chars().next().unwrap_or('0');
    let dropped = frac.get(1..).unwrap_or("");

    // The sign is already off, so "the larger n" is "the larger magnitude".
    let round_up = compare_to_half(dropped) != std::cmp::Ordering::Less;

    let mut digits = format!("{int_part}{kept}");
    if round_up {
        digits = increment_decimal(&digits);
    }
    let (whole, tenth) = digits.split_at(digits.len().saturating_sub(1));
    let rebuilt = format!("{}{whole}.{tenth}", if negative { "-" } else { "" });
    rebuilt.parse::<f64>().unwrap_or(f64::NAN)
}

/// Compare a run of decimal digits against `"5000…"` — i.e. decide whether the
/// dropped tail is more than, exactly, or less than one half.
fn compare_to_half(dropped: &str) -> std::cmp::Ordering {
    let mut chars = dropped.chars();
    match chars.next() {
        None => std::cmp::Ordering::Less,
        Some(c) if c < '5' => std::cmp::Ordering::Less,
        Some(c) if c > '5' => std::cmp::Ordering::Greater,
        // Leading 5: a tie only if literally nothing follows it.
        Some(_) => {
            if chars.any(|c| c != '0') {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        }
    }
}

/// Add one to a decimal digit string, growing it if it carries out (`"99"` →
/// `"100"`). Done on the string rather than by adding `0.1` to a float, because
/// `8.2 + 0.1` is `8.299999999999999` and would come back as a different number.
fn increment_decimal(digits: &str) -> String {
    let mut out: Vec<u8> = digits.bytes().collect();
    for slot in out.iter_mut().rev() {
        if *slot == b'9' {
            *slot = b'0';
        } else {
            *slot = slot.saturating_add(1);
            return String::from_utf8_lossy(&out).into_owned();
        }
    }
    format!("1{}", String::from_utf8_lossy(&out))
}

/// A JavaScript number as `JSON.stringify` would write it.
///
/// Two rules that a naive `f64` serialisation gets wrong, both of which the corpus
/// would catch as a byte difference:
///
/// - `NaN` and `±Infinity` become **`null`**. This is the non-numeric-rating case,
///   and it is why `rating` is a [`Value`] and not an `f64`.
/// - an integral value has no fractional part: `8` and not `8.0`. `serde_json`
///   writes `8.0` for `8.0_f64`, and every rating that is a round number would
///   differ from the twin.
fn js_number_value(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        return Value::from(x as i64);
    }
    serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(json: &str) -> CatalogMeta {
        serde_json::from_str(json).unwrap()
    }
    fn mapped(json: &str) -> String {
        serde_json::to_string(&map_catalog_meta(&meta(json))).unwrap()
    }

    /// The whole record, as bytes, in the twin's field order.
    #[test]
    fn maps_a_full_entry_in_the_twins_field_order() {
        assert_eq!(
            mapped(
                r#"{"id":"tt0903747","name":"Breaking Bad","type":"series",
                    "releaseInfo":"2008-2013","imdbRating":"9.5",
                    "genres":["Crime","Drama"],"poster":"https://a.co/p.jpg"}"#
            ),
            r#"{"id":"tt0903747","type":"series","title":"Breaking Bad","year":"2008","rating":9.5,"genre":"Crime","poster":"https://a.co/p.jpg"}"#
        );
    }

    /// Nothing but an id. Every default the twin applies, at once — and `poster`
    /// absent rather than `null`, which is `undefined`'s whole point.
    #[test]
    fn an_empty_entry_gets_every_default() {
        assert_eq!(
            mapped(r#"{"id":"x"}"#),
            r#"{"id":"x","type":"movie","title":"Untitled","year":"","rating":0,"genre":""}"#
        );
    }

    #[test]
    fn genres_are_read_from_three_spellings() {
        let genre = |json: &str| map_catalog_meta(&meta(json)).genre;
        assert_eq!(genre(r#"{"id":"x","genres":["A","B"]}"#), Value::from("A"));
        assert_eq!(genre(r#"{"id":"x","genres":"A"}"#), Value::from("A"));
        // `genre` is the fallback spelling, list or string alike.
        assert_eq!(genre(r#"{"id":"x","genre":["A"]}"#), Value::from("A"));
        assert_eq!(genre(r#"{"id":"x","genre":"A"}"#), Value::from("A"));
        // Neither, or an empty list — `[]` is truthy, so it wins the `||` chain
        // and then `[0]` is undefined. Both roads end at `""`.
        assert_eq!(genre(r#"{"id":"x"}"#), Value::from(""));
        assert_eq!(genre(r#"{"id":"x","genres":[]}"#), Value::from(""));
        // An empty STRING is falsy, so the fallback is consulted.
        assert_eq!(
            genre(r#"{"id":"x","genres":"","genre":"B"}"#),
            Value::from("B")
        );
    }

    #[test]
    fn year_prefers_release_info_and_slices_to_four() {
        let year = |json: &str| map_catalog_meta(&meta(json)).year;
        assert_eq!(year(r#"{"id":"x","releaseInfo":"2008-2013"}"#), "2008");
        assert_eq!(year(r#"{"id":"x","year":"1994"}"#), "1994");
        // `releaseInfo` wins when both are present.
        assert_eq!(
            year(r#"{"id":"x","releaseInfo":"2008","year":"1994"}"#),
            "2008"
        );
        // ...but only when it is truthy.
        assert_eq!(year(r#"{"id":"x","releaseInfo":"","year":"1994"}"#), "1994");
        // A number is stringified without a fractional part.
        assert_eq!(year(r#"{"id":"x","year":2019}"#), "2019");
        assert_eq!(year(r#"{"id":"x","year":2019.0}"#), "2019");
        assert_eq!(year(r#"{"id":"x"}"#), "");
        // Shorter than four is not padded.
        assert_eq!(year(r#"{"id":"x","year":"99"}"#), "99");
    }

    /// The `NaN` hazard, pinned. A junk rating is `null`, not `0` and not an
    /// error — because `JSON.stringify(NaN)` is `null` and the twin has no guard.
    #[test]
    fn rating_reproduces_the_javascript_number_pipeline() {
        let rating = |json: &str| map_catalog_meta(&meta(json)).rating;
        // 8.15 is not exactly 8.15 in binary — it is a hair ABOVE — so this is a
        // genuine round-up and not a tie.
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"8.15"}"#),
            Value::from(8.2)
        );
        // 8.25 IS exactly representable, so it is a real tie, and the spec says
        // the larger n wins. Rust's own `{:.1}` would answer 8.2 here.
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"8.25"}"#),
            Value::from(8.3)
        );
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"8.75"}"#),
            Value::from(8.8)
        );
        // ...and the sign comes off first, so this is 8.3 with a minus, not 8.2.
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"-8.25"}"#),
            Value::from(-8.3)
        );
        // 9.95 is a hair BELOW 9.95 in binary, so it rounds down — verified
        // against V8, not assumed.
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"9.95"}"#),
            Value::from(9.9)
        );
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"0.15"}"#),
            Value::from(0.1)
        );
        // A carry that grows the integer part.
        assert_eq!(rating(r#"{"id":"x","imdbRating":"9.99"}"#), Value::from(10));
        // Integral results lose the fractional part: `8`, never `8.0`.
        assert_eq!(rating(r#"{"id":"x","imdbRating":"8"}"#), Value::from(8));
        assert_eq!(
            serde_json::to_string(&rating(r#"{"id":"x","imdbRating":"8.0"}"#)).unwrap(),
            "8"
        );
        // A number, not a string, is equally acceptable.
        assert_eq!(rating(r#"{"id":"x","imdbRating":7.44}"#), Value::from(7.4));
        // parseFloat takes the numeric PREFIX.
        assert_eq!(
            rating(r#"{"id":"x","imdbRating":"8.5/10"}"#),
            Value::from(8.5)
        );
        // THE HAZARD: non-numeric → NaN → null.
        assert_eq!(rating(r#"{"id":"x","imdbRating":"abc"}"#), Value::Null);
        assert_eq!(rating(r#"{"id":"x","imdbRating":"N/A"}"#), Value::Null);
        // Falsy ratings never reach parseFloat at all — they are 0.
        assert_eq!(rating(r#"{"id":"x"}"#), Value::from(0));
        assert_eq!(rating(r#"{"id":"x","imdbRating":""}"#), Value::from(0));
        assert_eq!(rating(r#"{"id":"x","imdbRating":null}"#), Value::from(0));
        // ...but the STRING "0" is truthy in JavaScript, so it does.
        assert_eq!(rating(r#"{"id":"x","imdbRating":"0"}"#), Value::from(0));
    }

    /// The declared divergence. `type: "tv"` is a series here and a **movie** in
    /// the twin, which is the bug this increment exists to close.
    #[test]
    fn type_reads_both_vocabularies_diverging_from_the_twin_on_purpose() {
        let kind =
            |t: &str| map_catalog_meta(&meta(&format!(r#"{{"id":"x","type":"{t}"}}"#))).item_type;
        assert_eq!(kind("series"), "series");
        assert_eq!(kind("tv"), "series", "the twin answers \"movie\" here");
        assert_eq!(kind("show"), "series");
        assert_eq!(kind("movie"), "movie");
        assert_eq!(kind("channel"), "movie");
        // A non-string type is not a type.
        assert_eq!(
            map_catalog_meta(&meta(r#"{"id":"x","type":7}"#)).item_type,
            "movie"
        );
    }

    /// The twin copies `id` through without coercion, and `lib/types.ts` types it
    /// `string | number` because add-ons take it at its word.
    #[test]
    fn id_and_title_pass_through_uncoerced() {
        assert_eq!(
            mapped(r#"{"id":12345,"name":"X","poster":"p"}"#),
            r#"{"id":12345,"type":"movie","title":"X","year":"","rating":0,"genre":"","poster":"p"}"#
        );
    }

    #[test]
    fn a_response_drops_the_cards_that_cannot_be_rendered() {
        let r: CatalogResponse = serde_json::from_str(
            r#"{"metas":[{"id":"a","poster":"p1"},{"id":"b"},"nonsense",
                         {"id":"c","poster":""},{"id":"d","poster":"p2"}]}"#,
        )
        .unwrap();
        let out = map_catalog_metas(&r.metas);
        let ids: Vec<&Value> = out.iter().map(|i| &i.id).collect();
        assert_eq!(ids, vec![&Value::from("a"), &Value::from("d")]);
    }

    #[test]
    fn an_absent_or_null_metas_array_is_an_empty_one() {
        for json in [r#"{}"#, r#"{"metas":null}"#] {
            let r: CatalogResponse = serde_json::from_str(json).unwrap();
            assert!(r.metas.is_empty());
        }
    }

    #[test]
    fn parse_float_takes_the_longest_prefix() {
        assert_eq!(js_parse_float("8.5"), 8.5);
        assert_eq!(js_parse_float("  8.5  "), 8.5);
        assert_eq!(js_parse_float("8.5abc"), 8.5);
        assert_eq!(js_parse_float("-8.5"), -8.5);
        assert_eq!(js_parse_float("1e3"), 1000.0);
        assert_eq!(
            js_parse_float("1e"),
            1.0,
            "an incomplete exponent is dropped"
        );
        assert_eq!(js_parse_float(".5"), 0.5);
        assert_eq!(js_parse_float("5."), 5.0);
        assert!(js_parse_float("abc").is_nan());
        assert!(js_parse_float("").is_nan());
        assert!(js_parse_float(".").is_nan());
        assert!(js_parse_float("-").is_nan());
        assert!(js_parse_float("Infinity").is_infinite());
    }
}
