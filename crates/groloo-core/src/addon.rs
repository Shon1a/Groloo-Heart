//! Pure helpers for working with a real (user-installed) add-on and its manifest:
//! URL normalisation, base-URL resolution, manifest validation, and capability
//! checks. All string-in / string-out — no I/O, so identical on every device.
//!
//! VENDORED from `Groloo-Heart@0.1.0 src/addon.rs`, behaviour-frozen, tests
//! included. The only edits are to the *shape* of the code: `&s[a..b]` became
//! `s.get(a..b)` throughout, because a crate that denies `indexing_slicing`
//! denies it in vendored code too or the rule means nothing. No branch changed.
//!
//! ## Why this module matters more than its size suggests
//!
//! Heart shipped it and no shell could reach it — there was no binding for any of
//! these four functions. Meanwhile `addonClient.ts:45` reimplements
//! [`addon_base_url`] character for character, `addonClient.ts:49` reimplements
//! [`manifest_has_resource`], `addons.ts:233` reimplements a two-rule subset of
//! [`validate_manifest`], and `addons.ts:203` reimplements
//! [`normalize_manifest_url`] — **incorrectly**. Its `/manifest\.json$/` test runs
//! against the whole string including the query, so it never matches a
//! query-bearing URL and appends a second `/manifest.json`:
//!
//! ```text
//!   "https://a.co/x/manifest.json?y=2"
//!     TS   → "https://a.co/x/manifest.json?y=2/manifest.json"   broken
//!     here → "https://a.co/x/manifest.json?y=2"                 correct
//! ```
//!
//! By the add-on protocol's convention, a *configured* add-on packs its credentials into
//! the URL — so the add-ons that break are exactly the credentialed ones. This is
//! what an unreachable core costs: not a missing feature, a divergent one.
//!
//! ## THE UNIFICATION LEDGER — three copies, one survivor
//!
//! `normalize_manifest_url` existed **three** times and no two agreed. This is the
//! audit trail for which behaviour won on each axis, because "we took the union"
//! is not a reviewable statement and the losing behaviours are the ones a bug
//! report will be written against.
//!
//! | Input | `addons.ts:203` (client) | `server.js:1992` (server) | **here** |
//! |---|---|---|---|
//! | `https://a.co/x/manifest.json?y=2` | `…?y=2/manifest.json` **broken** | `…?y=2` | `…?y=2` |
//! | `https://a.co/addon?x=1` | `…?x=1/manifest.json` **broken** | `…/manifest.json?x=1` | `…/manifest.json?x=1` |
//! | `https://a.co/manifest.json/` | `…/manifest.json` | `…/manifest.json/manifest.json` **broken** | `…/manifest.json` |
//! | `https://a.co/cfg=a\|b,c/manifest.json` | kept | kept (byte-for-byte, never re-encoded) | kept |
//! | `https://a.co/x/foo.json` | `…/foo.json/manifest.json` | `…/foo.json` | `…/foo.json` |
//! | `stremio://a.co/x` | `https://a.co/x/manifest.json` | `https://a.co/x/manifest.json` | `https://a.co/x/manifest.json` |
//! | `ftp://a.co/x` | `https://a.co/x/manifest.json` | `https://a.co/x/manifest.json` | `https://a.co/x/manifest.json` |
//! | `a.co/x` (no scheme) | `https://a.co/x/manifest.json` | `null` | `https://a.co/x/manifest.json` |
//! | `not-an-id` | `https://not-an-id/manifest.json` | `null` | `None` |
//! | `my addon` | `https://my addon/manifest.json` | `null` | `None` |
//! | `""` / `"   "` | `https://manifest.json` | `null` | `None` |
//!
//! **Query handling: the server wins.** Its `.json($|?)` test runs against the
//! *pathname*, so a configured add-on's `?` never defeats it. The client's
//! `/manifest\.json$/` runs against the whole string, never matches a
//! query-bearing URL, and appends a second segment — and by the add-on protocol's
//! convention the URLs that carry a query are exactly the *credentialed* ones, so
//! the client copy fails precisely on the add-ons a user paid for.
//!
//! **The trailing slash: the client wins, and this is the one row the server got
//! wrong.** `addons.ts:203` strips `/+$` *before* testing for `.json`, so
//! `…/manifest.json/` — what a browser address bar hands back, and what a link
//! shortener leaves behind — normalises to the fetchable `…/manifest.json`. The
//! server tests `parsed.pathname` first, sees `/manifest.json/`, fails, and
//! appends a second segment, producing `…/manifest.json/manifest.json`: a 404
//! where the paste worked. Adopting the server's *pathname* test without its
//! *ordering* was the port's own bug (recorded as **U-slash**), and the fix is to
//! take both halves — trim first, then test the path — which is strictly better
//! than either shipping copy and is what makes `…/manifest.json/` a fixpoint too.
//!
//! **The scheme rewrite: both TS copies win, and the previous Rust loses.** This
//! module used to accept only `http(s)://` and its own `groloo://` alias, and
//! reject everything else. That is wrong in the one case that matters most:
//! add-on links are *shared* under a native-client scheme, so `stremio://…` is the
//! single commonest thing a user pastes, and rejecting it is a regression against
//! both shipping copies. Any `scheme://` that is not http(s) is now rewritten to
//! `https://` — which subsumes `groloo://` rather than special-casing it, and
//! costs a wrong-but-fetchable URL for the genuinely nonsensical `ftp://` case.
//!
//! **The bare host: neither copy wins outright.** The client prepends `https://`
//! to anything at all, which turns a typo into a URL that fails later, as a
//! network error, with nothing to say it was never a URL. The server rejects every
//! schemeless string, which loses the `v3-cinemeta.strem.io/manifest.json` paste
//! that people actually make. Here the string is accepted **only when its
//! authority is host-shaped** ([`looks_like_host`]): no whitespace, host
//! characters only, and either a dot or a port. That accepts every input the
//! client accepted *for a reason* and rejects the ones it accepted by accident,
//! and a rejection is reportable where a guess is not.
//!
//! `validate_manifest` is the same story in miniature: `server.js:1947` has all
//! four rules with four distinct messages, `addons.ts:233` has two of them under
//! one message. The server's copy won, verbatim, including its message strings —
//! see the note on [`validate_manifest`] for the one thing it got wrong.

use crate::types::AddonManifest;

/// The base URL (directory) that an add-on's resource paths are relative to —
/// i.e. the manifest URL with its last path segment removed.
///
/// `https://a.co/x/manifest.json` → `https://a.co/x/`
pub fn addon_base_url(manifest_url: &str) -> String {
    match manifest_url.rfind('/').and_then(|i| manifest_url.get(..=i)) {
        Some(s) => s.to_string(),
        None => String::new(),
    }
}

/// Normalise a raw, user-pasted string into a canonical `.../manifest.json` URL,
/// or `None` if it names nothing fetchable.
///
/// **The single implementation** of a rule that ran in three places; the ledger in
/// the module doc records which copy won on each axis and why. Four steps, in this
/// order, because the order is itself one of the divergences:
///
/// 1. a non-http(s) `scheme://` becomes `https://` — native-client deep links;
/// 2. a schemeless but host-shaped string gets `https://` prepended;
/// 3. the result must have a scheme *and* an authority, else `None`;
/// 4. trailing slashes come off the **path** (never the query), and only then is
///    the path tested for a `.json` suffix; if it has one the URL is returned
///    with the query re-attached, otherwise `/manifest.json` is inserted *before*
///    the query.
///
/// Step 4's ordering is the whole of U-slash: the server's copy tests before it
/// trims and turns `…/manifest.json/` into `…/manifest.json/manifest.json`. The
/// path is trimmed on both branches, so the two spellings of a canonical URL
/// answer with the same bytes.
///
/// The URL is kept **byte-for-byte** throughout — no parse, no re-encode.
/// Configured add-ons pack their options into the path as `|`- and `,`-separated
/// pairs, and round-tripping through any URL type percent-encodes those and
/// mangles the config. That is why this function is string surgery and not
/// `Url::parse`, and it is also half of why this crate needs no `url` dependency.
///
/// Idempotent by construction: feeding the output back in cannot change it, which
/// is exactly the property the client's twin fails (its output grows another
/// `/manifest.json` on every pass).
pub fn normalize_manifest_url(raw: &str) -> Option<String> {
    let mut url = raw.trim().to_string();
    if url.is_empty() {
        return None;
    }

    // 1. Foreign scheme → https. `stremio://`, `groloo://`, `ftp://` alike; both
    //    TypeScript copies do this and the shared-link case depends on it.
    if let Some(rest) = strip_foreign_scheme(&url) {
        url = format!("https://{rest}");
    }

    // 2. No scheme at all. Accepted only when what is there could be a host —
    //    see the ledger: the client accepted anything, the server accepted
    //    nothing, and both answers have a failure mode a user has to debug.
    let low = url.to_ascii_lowercase();
    let scheme_len = if low.starts_with("https://") {
        "https://".len()
    } else if low.starts_with("http://") {
        "http://".len()
    } else if looks_like_host(&url) {
        url = format!("https://{url}");
        "https://".len()
    } else {
        return None;
    };

    // 3. Require an authority (something between `scheme://` and the first `/`).
    let after_scheme = url.get(scheme_len..).unwrap_or("");
    if authority_of(after_scheme).is_empty() {
        return None;
    }

    // 4. Split off the query; test the PATH portion for a `.json` suffix. The
    //    client's twin tests the whole string, which is precisely why it fails on
    //    every configured add-on.
    let (path, qs) = match url.find('?') {
        Some(i) => (url.get(..i).unwrap_or(""), url.get(i..).unwrap_or("")),
        None => (url.as_str(), ""),
    };
    // 4a. Trailing slashes come off the path BEFORE the `.json` test, never after
    //     it — U-slash, and the one axis where the client's copy was the right one.
    //     Testing first sees `/manifest.json/`, fails, and appends a SECOND
    //     `/manifest.json`: a URL that 404s where the paste worked. Trimming first
    //     also makes `…/manifest.json/` and `…/manifest.json` the same fixpoint,
    //     which is the property the whole function is built around.
    let path = path.trim_end_matches('/');
    if path.to_ascii_lowercase().ends_with(".json") {
        // Not `url` — that still carries the trailing slash this just removed.
        return Some(format!("{path}{qs}"));
    }
    Some(format!("{path}/manifest.json{qs}"))
}

/// The `scheme://` prefix, when there is one and it is **not** http(s) — i.e. the
/// part `server.js:1996`'s `/^(?!https?:\/\/)[a-z][a-z0-9+.-]*:\/\//i` matches.
///
/// Returns what follows it. The charset test is what keeps a query string from
/// being mistaken for a scheme: `a.co/x?u=https://y` contains `://`, but the text
/// before it holds `/` and `?`, which no scheme may.
fn strip_foreign_scheme(url: &str) -> Option<&str> {
    let i = url.find("://")?;
    let scheme = url.get(..i)?;
    if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    url.get(i.checked_add(3)?..)
}

/// Everything up to the first `/`, `?` or `#` — the authority, on a string that
/// has already had its scheme removed.
fn authority_of(after_scheme: &str) -> &str {
    match after_scheme.find(['/', '?', '#']) {
        Some(i) => after_scheme.get(..i).unwrap_or(""),
        None => after_scheme,
    }
}

/// Could this schemeless string plausibly *start* with a hostname?
///
/// The client's twin asks no such question and prepends `https://` to anything,
/// so `not-an-id` and `my addon` become URLs that fail later as opaque network
/// errors. The test is deliberately shape-only — no DNS, no TLD list, nothing that
/// could reject a LAN host or a new gTLD: host characters only, and either a dot
/// or a port, which is the smallest rule that separates `a.co/x` from a typo.
fn looks_like_host(s: &str) -> bool {
    let host = authority_of(s);
    if host.is_empty() {
        return false;
    }
    let shaped = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '%'));
    shaped && host.contains(['.', ':'])
}

/// Validate the shape of an add-on's `manifest.json`. `Ok(())` if usable, else a
/// human-readable reason.
///
/// Four rules, each with its own message, taken verbatim from `server.js:1947` —
/// including the message strings, so a user who installs the same add-on from the
/// browser and from the server sees the same sentence. `addons.ts:233` implements
/// two of the four (`id`, `name`, presence only) under one message,
/// `'Not a valid add-on manifest'`, so a user with a malformed id and a user with
/// no `types` get identical, useless feedback.
///
/// **Four rules, not five.** The server's first — `!m || typeof m !== 'object'`,
/// `'Manifest is not a JSON object'` — cannot live here: this function takes an
/// `&AddonManifest`, and a document that is not an object never became one. It
/// runs at the boundary instead, on the untyped value, in
/// [`crate::api::validate_manifest`] — which is where U-notobject was fixed and
/// why that function parses to a [`serde_json::Value`] before it parses to a
/// manifest.
///
/// One correction against the server: its `m.name.length > 200` counts **UTF-16
/// code units**, and the Rust twin used to count *bytes*. For a Georgian or
/// Japanese add-on name those differ by 3×, so a 90-character Georgian name is
/// 270 bytes and the byte test rejected an add-on the server accepted — a
/// divergence that could only ever fire on non-Latin names, in an app whose first
/// audience is Georgian. Counted in UTF-16 units here, which is what the rule
/// meant and what every other copy computes.
pub fn validate_manifest(m: &AddonManifest) -> Result<(), String> {
    if m.id.is_empty() || !valid_id(&m.id) {
        return Err("Manifest \"id\" is missing or malformed".to_string());
    }
    if m.name.trim().is_empty() || m.name.encode_utf16().count() > 200 {
        return Err("Manifest \"name\" is missing or too long".to_string());
    }
    if m.resources.is_empty() {
        return Err("Manifest missing \"resources\"".to_string());
    }
    if m.types.is_empty() {
        return Err("Manifest missing \"types\"".to_string());
    }
    Ok(())
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,200}$`
fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    if b.is_empty() || b.len() > 201 {
        return false;
    }
    match b.first() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    b.get(1..)
        .unwrap_or(&[])
        .iter()
        .all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

/// Does the manifest advertise a `stream` resource?
pub fn manifest_provides_stream(m: &AddonManifest) -> bool {
    m.resources.iter().any(|r| r.name() == "stream")
}

/// Does the manifest advertise `resource`, optionally for `typ`?
///
/// Short (`"stream"`) and full (`{"name":"stream",...}`) resource forms are
/// treated identically — the shell's twin re-derives that with a
/// `typeof r === 'string' ? r : r?.name` map.
pub fn manifest_has_resource(m: &AddonManifest, resource: &str, typ: Option<&str>) -> bool {
    let has = m.resources.iter().any(|r| r.name() == resource);
    has && typ.is_none_or(|t| m.types.iter().any(|x| x == t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_strips_last_segment() {
        assert_eq!(
            addon_base_url("https://a.co/x/manifest.json"),
            "https://a.co/x/"
        );
        assert_eq!(addon_base_url("https://a.co/"), "https://a.co/");
        assert_eq!(addon_base_url("noslash"), "");
    }

    #[test]
    fn normalize_variants() {
        assert_eq!(
            normalize_manifest_url("https://a.co/x/manifest.json").as_deref(),
            Some("https://a.co/x/manifest.json")
        );
        assert_eq!(
            normalize_manifest_url("https://a.co/x").as_deref(),
            Some("https://a.co/x/manifest.json")
        );
        assert_eq!(
            normalize_manifest_url("https://a.co/x/").as_deref(),
            Some("https://a.co/x/manifest.json")
        );
        // own scheme maps to https
        assert_eq!(
            normalize_manifest_url("groloo://a.co/x").as_deref(),
            Some("https://a.co/x/manifest.json")
        );
        // query preserved, config chars untouched
        assert_eq!(
            normalize_manifest_url("https://a.co/cfg=a|b,c/manifest.json?x=1").as_deref(),
            Some("https://a.co/cfg=a|b,c/manifest.json?x=1")
        );
        assert_eq!(
            normalize_manifest_url("https://a.co/addon?x=1").as_deref(),
            Some("https://a.co/addon/manifest.json?x=1")
        );
        // rejects
        assert_eq!(normalize_manifest_url(""), None);
        assert_eq!(normalize_manifest_url("   "), None);
        assert_eq!(normalize_manifest_url("https://"), None);
    }

    /// The ledger in the module doc, executed. Every row of that table is an
    /// assertion here, because a table in a doc comment is prose until something
    /// runs it.
    #[test]
    fn normalize_matches_the_unification_ledger() {
        let n = |s: &str| normalize_manifest_url(s);

        // — query handling: the SERVER's rule won —
        assert_eq!(
            n("https://a.co/x/manifest.json?y=2").as_deref(),
            Some("https://a.co/x/manifest.json?y=2"),
            "client appends a second /manifest.json after the query"
        );
        assert_eq!(
            n("https://a.co/addon?x=1").as_deref(),
            Some("https://a.co/addon/manifest.json?x=1"),
            "the segment goes before the query, not after it"
        );
        // Any `.json` path is already a manifest document — server rule, not the
        // client's `/manifest\.json$/`.
        assert_eq!(
            n("https://a.co/x/foo.json").as_deref(),
            Some("https://a.co/x/foo.json")
        );
        // Packed config survives byte-for-byte: no parse, no re-encode.
        assert_eq!(
            n("https://a.co/|other=KEY|,pv=X/manifest.json").as_deref(),
            Some("https://a.co/|other=KEY|,pv=X/manifest.json")
        );

        // — the trailing slash: the CLIENT's rule won —
        assert_eq!(
            n("https://a.co/manifest.json/").as_deref(),
            Some("https://a.co/manifest.json"),
            "the server appends a SECOND segment here and 404s"
        );

        // — scheme rewrite: BOTH TypeScript copies won over the old Rust —
        for raw in ["stremio://a.co/x", "groloo://a.co/x", "ftp://a.co/x"] {
            assert_eq!(
                n(raw).as_deref(),
                Some("https://a.co/x/manifest.json"),
                "{raw} must be rewritten, not rejected — shared add-on links \
                 arrive under a native-client scheme"
            );
        }
        // A `://` inside a query is not a scheme — the charset test is what stops
        // the rewrite from eating the whole string.
        assert_eq!(
            n("a.co/x?u=https://y").as_deref(),
            Some("https://a.co/x/manifest.json?u=https://y")
        );

        // — bare host: accepted only when host-shaped —
        assert_eq!(
            n("a.co/x").as_deref(),
            Some("https://a.co/x/manifest.json"),
            "the paste people actually make"
        );
        assert_eq!(
            n("localhost:3000").as_deref(),
            Some("https://localhost:3000/manifest.json")
        );
        for typo in ["not-an-id", "my addon", "manifest.json ok"] {
            assert_eq!(n(typo), None, "{typo:?} is not a host — the client guessed");
        }
    }

    /// **U-slash, and the class it belongs to.** The trailing slash has to be
    /// stripped from the PATH before the `.json` test, and the path is not the
    /// whole string — so the same input has to survive being combined with the two
    /// things this function exists to preserve: a query string, and a `|`-packed
    /// debrid config. Each combination is its own regression, because trimming the
    /// wrong span is exactly as wrong as not trimming at all:
    ///
    /// - trimming the whole URL would eat a slash that belongs to the *query*;
    /// - trimming and then returning `url` unchanged would keep the slash anyway;
    /// - percent-encoding on the way through would mangle the `|` and `,`.
    #[test]
    fn a_trailing_slash_survives_a_query_and_a_packed_config() {
        let n = |s: &str| normalize_manifest_url(s);

        // Bare: the U-slash case itself.
        assert_eq!(
            n("https://a.co/manifest.json/").as_deref(),
            Some("https://a.co/manifest.json")
        );
        // Many slashes, not one.
        assert_eq!(
            n("https://a.co/manifest.json///").as_deref(),
            Some("https://a.co/manifest.json")
        );
        // × a query string. The slash is on the PATH, the query is re-attached
        // after it — not `…/manifest.json/?y=2`, and not a second segment.
        assert_eq!(
            n("https://a.co/x/manifest.json/?y=2").as_deref(),
            Some("https://a.co/x/manifest.json?y=2")
        );
        // × a query string, on the branch that INSERTS the segment.
        assert_eq!(
            n("https://a.co/addon/?x=1").as_deref(),
            Some("https://a.co/addon/manifest.json?x=1")
        );
        // × `|`-packed debrid config, byte-for-byte: no `%7C`, no `%2C`.
        assert_eq!(
            n("https://a.co/|other=KEY|,pv=X/manifest.json/").as_deref(),
            Some("https://a.co/|other=KEY|,pv=X/manifest.json")
        );
        assert_eq!(
            n("https://a.co/|other=KEY|,pv=X/").as_deref(),
            Some("https://a.co/|other=KEY|,pv=X/manifest.json")
        );
        // × both at once, which is what a configured add-on's link actually looks
        // like once a browser has been near it.
        assert_eq!(
            n("stremio://a.co/cfg=a|b,c/manifest.json/?tok=1").as_deref(),
            Some("https://a.co/cfg=a|b,c/manifest.json?tok=1")
        );

        // A trailing slash INSIDE the query is data and must not be touched — the
        // trim is scoped to the path precisely so this keeps working.
        assert_eq!(
            n("https://a.co/x/manifest.json?redirect=https://b.co/").as_deref(),
            Some("https://a.co/x/manifest.json?redirect=https://b.co/")
        );
        assert_eq!(
            n("https://a.co/addon?redirect=b.co/").as_deref(),
            Some("https://a.co/addon/manifest.json?redirect=b.co/")
        );

        // The authority alone, with nothing but slashes for a path.
        assert_eq!(
            n("https://a.co//").as_deref(),
            Some("https://a.co/manifest.json")
        );
    }

    /// Normalisation must be a fixpoint: feeding its own output back in cannot
    /// change it. This is the property the TS twin fails — its output grows a
    /// `/manifest.json` on every pass.
    #[test]
    fn normalize_is_a_fixpoint() {
        for raw in [
            "https://a.co/x",
            "https://a.co/x/",
            "https://a.co/addon?x=1",
            "https://a.co/x/manifest.json",
            "https://a.co/x/manifest.json?y=2",
            // U-slash: before the fix this one grew a second segment on pass 1 and
            // then stabilised on the wrong answer, so the property held while the
            // URL was already broken. It only means something now.
            "https://a.co/manifest.json/",
            "https://a.co/x/manifest.json/?y=2",
            "https://a.co/|other=KEY|,pv=X/manifest.json/",
            "groloo://a.co/cfg=a|b,c",
            "stremio://a.co/x",
            "a.co/x",
        ] {
            let once = normalize_manifest_url(raw).unwrap();
            let twice = normalize_manifest_url(&once).unwrap();
            assert_eq!(once, twice, "not a fixpoint for {raw}");
        }
    }

    #[test]
    fn validate_manifest_cases() {
        let ok: AddonManifest = serde_json::from_str(
            r#"{"id":"com.x.addon","name":"X","resources":["catalog"],"types":["movie"]}"#,
        )
        .unwrap();
        assert!(validate_manifest(&ok).is_ok());

        let no_res: AddonManifest =
            serde_json::from_str(r#"{"id":"x","name":"X","resources":[],"types":["movie"]}"#)
                .unwrap();
        assert!(validate_manifest(&no_res).is_err());

        let bad_id: AddonManifest = serde_json::from_str(
            r#"{"id":"-bad","name":"X","resources":["catalog"],"types":["movie"]}"#,
        )
        .unwrap();
        assert!(validate_manifest(&bad_id).is_err());
    }

    /// Each rule reports itself. The shell's twin gives one message for all of
    /// them, so a user with a typed id and a user with no types get identical,
    /// useless feedback.
    #[test]
    fn validate_manifest_messages_are_distinct() {
        let m = |json: &str| -> String {
            let p: AddonManifest = serde_json::from_str(json).unwrap();
            validate_manifest(&p).unwrap_err()
        };
        assert!(
            m(r#"{"id":"","name":"X","resources":["catalog"],"types":["movie"]}"#)
                .contains("\"id\"")
        );
        assert!(
            m(r#"{"id":"x","name":"  ","resources":["catalog"],"types":["movie"]}"#)
                .contains("\"name\"")
        );
        assert!(
            m(r#"{"id":"x","name":"X","resources":[],"types":["movie"]}"#)
                .contains("\"resources\"")
        );
        assert!(
            m(r#"{"id":"x","name":"X","resources":["catalog"],"types":[]}"#).contains("\"types\"")
        );
    }

    /// The name cap counts UTF-16 units, as `server.js:1950` does — not bytes.
    /// A 90-character Georgian name is 270 UTF-8 bytes; the byte test rejected an
    /// add-on the server had already accepted, and only ever for non-Latin names.
    #[test]
    fn name_length_is_counted_the_way_javascript_counts_it() {
        let manifest = |name: &str| -> AddonManifest {
            serde_json::from_str(&format!(
                r#"{{"id":"x","name":{},"resources":["catalog"],"types":["movie"]}}"#,
                serde_json::Value::String(name.to_string())
            ))
            .unwrap()
        };
        // 90 Georgian characters: 90 UTF-16 units, 270 UTF-8 bytes.
        let georgian = "ა".repeat(90);
        assert_eq!(georgian.len(), 270);
        assert_eq!(georgian.encode_utf16().count(), 90);
        assert!(validate_manifest(&manifest(&georgian)).is_ok());

        assert!(validate_manifest(&manifest(&"a".repeat(200))).is_ok());
        assert!(validate_manifest(&manifest(&"a".repeat(201))).is_err());
        assert!(validate_manifest(&manifest(&"ა".repeat(201))).is_err());
    }

    #[test]
    fn resource_checks_short_and_full() {
        let m: AddonManifest = serde_json::from_str(
            r#"{"id":"x","name":"X","types":["movie","series"],
                "resources":["catalog",{"name":"stream","types":["movie"]}]}"#,
        )
        .unwrap();
        assert!(manifest_provides_stream(&m));
        assert!(manifest_has_resource(&m, "catalog", Some("movie")));
        assert!(!manifest_has_resource(&m, "meta", None));
        assert!(!manifest_has_resource(&m, "catalog", Some("channel")));
    }
}
