//! The response envelope — how the core tells a shell that something went wrong
//! *without* being able to hide it.
//!
//! Heart's doctrine was "infallible by degradation": on any bad input, return the
//! sensible fallback and say nothing. The degradation was right; the silence was
//! not. Four call sites in Stredio-Web wrap every core call in
//! `catch { return null }`, so a core that has stopped working and a core that
//! legitimately has nothing to say are the same observation, and there is no
//! telemetry that can tell them apart. That is problem 7, and it has been running
//! in production invisibly.
//!
//! So every boundary function answers with:
//!
//! ```json
//! { "ok": true, "core": "0.2.0+g1a2b3c4", "data": …,
//!   "warnings": [ {"code":"skipped.has_stream","subject":"pirate","detail":"…"} ],
//!   "error": null }
//! ```
//!
//! Two properties make this worth the bytes:
//!
//! 1. **`data` is always present and is always the graceful-degradation value** —
//!    the inline defaults, the unmodified library, `[]`. A shell that destructures
//!    `data` and ignores `ok` therefore behaves *bit-identically to today*. The
//!    envelope costs nothing to not adopt.
//! 2. **`ok` is a field, not an exception.** Throwing across wasm-bindgen would
//!    produce a JS `Error` that the four existing `catch` blocks would swallow
//!    exactly as they swallow everything else. A field you must reach through to
//!    get `data` cannot be ignored by accident.
//!
//! `core` is stamped by the constructor, which is the only way to build an
//! envelope — there is no code path that can forget the version, and a shell
//! comparing it against the vendored folder name it loaded from can finally see a
//! stale pin.

use crate::CORE_VERSION;
use serde::Serialize;

/// Why a call could not do what was asked. A closed set: a shell can branch on
/// these, and adding a variant is a deliberate, reviewable act rather than a new
/// free-text string appearing in someone's logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    /// The JSON did not parse at all.
    #[serde(rename = "parse.input")]
    ParseInput,
    /// A field had the wrong type; the record was skipped.
    #[serde(rename = "parse.field")]
    ParseField,
    /// A manifest/payload `schema` the core does not understand.
    #[serde(rename = "schema.unsupported")]
    SchemaUnsupported,
    /// A policy guard refused the input (neutral-conduit, icon allow-list).
    #[serde(rename = "guard.rejected")]
    GuardRejected,
    /// The thing asked for is not in the input (e.g. no official collection).
    #[serde(rename = "not_found")]
    NotFound,
    /// Could not produce a usable http(s) URL.
    #[serde(rename = "invalid.url")]
    InvalidUrl,
    /// Manifest validation failed; `detail` carries the specific reason.
    #[serde(rename = "invalid.manifest")]
    InvalidManifest,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ParseInput => "parse.input",
            ErrorCode::ParseField => "parse.field",
            ErrorCode::SchemaUnsupported => "schema.unsupported",
            ErrorCode::GuardRejected => "guard.rejected",
            ErrorCode::NotFound => "not_found",
            ErrorCode::InvalidUrl => "invalid.url",
            ErrorCode::InvalidManifest => "invalid.manifest",
        }
    }
}

/// Something the core dropped or clamped on the way through. Warnings are normal
/// operation, not failure: a CDN record rejected by a guard is the guard working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WarningCode {
    #[serde(rename = "skipped.no_id")]
    SkippedNoId,
    #[serde(rename = "skipped.not_official")]
    SkippedNotOfficial,
    #[serde(rename = "skipped.has_stream")]
    SkippedHasStream,
    #[serde(rename = "dropped.unsafe_icon")]
    DroppedUnsafeIcon,
    #[serde(rename = "dropped.bad_item")]
    DroppedBadItem,
    /// A row survived, but not as it was sent — a numeric id became a string, an
    /// explicit `null` became a default. Distinct from `dropped.bad_item` because
    /// the record is still *there*: nothing looks wrong at the call site, which is
    /// exactly why it has to be said. See [`crate::salvage`].
    #[serde(rename = "coerced.field")]
    CoercedField,
    #[serde(rename = "truncated.history")]
    TruncatedHistory,
    #[serde(rename = "truncated.progress")]
    TruncatedProgress,
    #[serde(rename = "truncated.mylist")]
    TruncatedMylist,
    #[serde(rename = "pruned.tombstone")]
    PrunedTombstone,
}

impl WarningCode {
    pub fn as_str(self) -> &'static str {
        match self {
            WarningCode::SkippedNoId => "skipped.no_id",
            WarningCode::SkippedNotOfficial => "skipped.not_official",
            WarningCode::SkippedHasStream => "skipped.has_stream",
            WarningCode::DroppedUnsafeIcon => "dropped.unsafe_icon",
            WarningCode::DroppedBadItem => "dropped.bad_item",
            WarningCode::CoercedField => "coerced.field",
            WarningCode::TruncatedHistory => "truncated.history",
            WarningCode::TruncatedProgress => "truncated.progress",
            WarningCode::TruncatedMylist => "truncated.mylist",
            WarningCode::PrunedTombstone => "pruned.tombstone",
        }
    }
}

/// One thing that happened which the caller may want to know about but which did
/// not stop the call. `subject` names *what* (an add-on id, a media key) so the
/// warning is actionable rather than a count.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub code: WarningCode,
    pub subject: String,
    pub detail: String,
}

impl Warning {
    pub fn new(code: WarningCode, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Warning {
            code,
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

/// The reason a call returned its fallback instead of its answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoreError {
    pub code: ErrorCode,
    pub detail: String,
}

impl CoreError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        CoreError {
            code,
            detail: detail.into(),
        }
    }
}

/// A boundary response. Fields are private and `core` is set by the constructor;
/// that is the whole mechanism by which no response can ship unversioned.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Envelope<T> {
    ok: bool,
    core: &'static str,
    data: T,
    warnings: Vec<Warning>,
    error: Option<CoreError>,
}

impl<T> Envelope<T> {
    /// The input was fully understood. `warnings` may still be non-empty.
    pub fn ok(data: T) -> Self {
        Envelope {
            ok: true,
            core: CORE_VERSION,
            data,
            warnings: Vec::new(),
            error: None,
        }
    }

    /// The call could not do what was asked. `data` must still be the value a
    /// shell can safely carry on with — that is the contract, not a courtesy.
    pub fn failed(data: T, error: CoreError) -> Self {
        Envelope {
            ok: false,
            core: CORE_VERSION,
            data,
            warnings: Vec::new(),
            error: Some(error),
        }
    }

    pub fn with_warning(mut self, w: Warning) -> Self {
        self.warnings.push(w);
        self
    }

    pub fn with_warnings(mut self, ws: impl IntoIterator<Item = Warning>) -> Self {
        self.warnings.extend(ws);
        self
    }

    pub fn is_ok(&self) -> bool {
        self.ok
    }
    pub fn data(&self) -> &T {
        &self.data
    }
    pub fn into_data(self) -> T {
        self.data
    }
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }
    pub fn error(&self) -> Option<&CoreError> {
        self.error.as_ref()
    }
}

impl<T: Serialize> Envelope<T> {
    /// Render to the exact string the shell receives.
    ///
    /// Infallible by construction: `serde_json` can only fail here on a
    /// non-string map key (nothing in this crate has one) or a non-finite float
    /// (which it renders as `null`). The fallback below therefore describes an
    /// unreachable state — but "unreachable" is not a thing to `unwrap()` on, and
    /// a hand-written envelope is still an envelope the shell can parse.
    pub fn into_json(self) -> String {
        match serde_json::to_string(&self) {
            Ok(s) => s,
            Err(e) => format!(
                r#"{{"ok":false,"core":"{}","data":null,"warnings":[],"error":{{"code":"parse.field","detail":{}}}}}"#,
                CORE_VERSION,
                serde_json::Value::String(format!("response could not be serialised: {e}"))
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn ok_envelope_has_every_key() {
        let v = parse(&Envelope::ok(vec!["a"]).into_json());
        assert_eq!(v["ok"], Value::Bool(true));
        assert_eq!(v["core"], Value::String(CORE_VERSION.to_string()));
        assert_eq!(v["data"], parse(r#"["a"]"#));
        assert_eq!(v["warnings"], parse("[]"));
        assert_eq!(v["error"], Value::Null);
        // `error: null` must be *present*, not merely absent-and-undefined.
        assert!(v.as_object().unwrap().contains_key("error"));
    }

    /// The load-bearing property: a failure still carries a usable `data`, so a
    /// shell that only reads `data` behaves exactly as it did before envelopes.
    #[test]
    fn failure_still_carries_the_fallback_value() {
        let v = parse(
            &Envelope::failed(
                vec!["inline"],
                CoreError::new(ErrorCode::SchemaUnsupported, "schema 2"),
            )
            .into_json(),
        );
        assert_eq!(v["ok"], Value::Bool(false));
        assert_eq!(v["data"], parse(r#"["inline"]"#));
        assert_eq!(
            v["error"]["code"],
            Value::String("schema.unsupported".into())
        );
        assert_eq!(v["error"]["detail"], Value::String("schema 2".into()));
    }

    #[test]
    fn warnings_serialise_with_their_dotted_codes() {
        let v = parse(
            &Envelope::ok(0)
                .with_warning(Warning::new(
                    WarningCode::SkippedHasStream,
                    "pirate",
                    "carries a transport",
                ))
                .with_warning(Warning::new(WarningCode::PrunedTombstone, "tt9", "expired"))
                .into_json(),
        );
        assert_eq!(v["ok"], Value::Bool(true)); // warnings are not failures
        assert_eq!(
            v["warnings"][0]["code"],
            Value::String("skipped.has_stream".into())
        );
        assert_eq!(v["warnings"][0]["subject"], Value::String("pirate".into()));
        assert_eq!(
            v["warnings"][1]["code"],
            Value::String("pruned.tombstone".into())
        );
    }

    /// Every code must serialise to the same token `as_str` reports, or a shell
    /// matching on the wire value and a Rust test matching on the enum are
    /// checking two different things.
    #[test]
    fn code_tokens_agree_with_their_wire_form() {
        let errs = [
            ErrorCode::ParseInput,
            ErrorCode::ParseField,
            ErrorCode::SchemaUnsupported,
            ErrorCode::GuardRejected,
            ErrorCode::NotFound,
            ErrorCode::InvalidUrl,
            ErrorCode::InvalidManifest,
        ];
        for c in errs {
            assert_eq!(
                serde_json::to_string(&c).unwrap(),
                format!("\"{}\"", c.as_str())
            );
        }
        let warns = [
            WarningCode::SkippedNoId,
            WarningCode::SkippedNotOfficial,
            WarningCode::SkippedHasStream,
            WarningCode::DroppedUnsafeIcon,
            WarningCode::DroppedBadItem,
            WarningCode::CoercedField,
            WarningCode::TruncatedHistory,
            WarningCode::TruncatedProgress,
            WarningCode::TruncatedMylist,
            WarningCode::PrunedTombstone,
        ];
        for c in warns {
            assert_eq!(
                serde_json::to_string(&c).unwrap(),
                format!("\"{}\"", c.as_str())
            );
        }
    }

    /// The version is stamped by construction, never by a caller remembering to.
    #[test]
    fn core_version_is_stamped_on_both_outcomes() {
        for s in [
            Envelope::ok(1).into_json(),
            Envelope::failed(1, CoreError::new(ErrorCode::ParseInput, "x")).into_json(),
        ] {
            assert_eq!(parse(&s)["core"], Value::String(CORE_VERSION.to_string()));
        }
    }
}
