//! Row-level leniency that says what it dropped.
//!
//! [`crate::types::de`] answers the question "what should the core do with one row
//! it cannot read?" with "drop the row and keep the document", and that answer is
//! right: a library missing one title is recoverable, a library missing everything
//! is not. What it cannot do is *say so*. A `deserialize_with` hook has no channel
//! back to the caller — it sees a row, it drops it, and serde hands the caller a
//! shorter document with no evidence that it is shorter. The boundary then answers
//! `ok:true` with an empty `warnings` array and the shell writes the shortened
//! document back as the user's truth.
//!
//! That is the exact failure [`crate::envelope`] was built to end — *the
//! degradation was right; the silence was not* — surviving in the one place where
//! the cost is a person's watch history rather than a repaint. So the same
//! leniency is performed here instead, one row at a time, over `serde_json::Value`:
//! a value the core walks itself is a value the core can account for.
//!
//! ## Two things are reported, not one
//!
//! - **Dropped** (`dropped.bad_item`) — the row did not deserialize and is gone.
//! - **Coerced** (`coerced.field`) — the row survived, but not as it was sent: a
//!   numeric id became a string, an explicit `null` position became `0`. This is
//!   the quieter half and the more dangerous one. A dropped row is at least
//!   *absent*; a coerced row is present, plausible and wrong, and it is what gets
//!   persisted.
//!
//! Coercion is detected structurally — the parsed row is re-serialised and
//! compared with the bytes it arrived as, key by key — rather than by keeping a
//! hand-written list of the coercions [`crate::types::de`] performs. A list is
//! something somebody has to remember to extend the next time a
//! `deserialize_with` is added, and the whole point of this module is that nothing
//! gets through unremarked. Two exclusions keep it honest rather than noisy:
//!
//! 1. **Keys the core does not model are not compared.** Dropping them is the
//!    documented contract ("unknown input fields are ignored, never round-tripped"),
//!    not a loss — and that same rule is what makes an explicit `null` on an
//!    `Option` field silent, because `null` *means* absent and serialises back to
//!    nothing.
//! 2. **Numbers compare by value.** JSON has no integer type and serde renders an
//!    `f64` field as `1.0` where the shell sent `1`; reporting that would put a
//!    warning on every progress record in every document and teach nobody anything.

use crate::envelope::{Warning, WarningCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Deserialize a JSON array one element at a time, keeping what parses and
/// reporting everything that did not survive verbatim.
///
/// `field` names the container so the warning's subject is `history[3]` rather
/// than a count: "we dropped 2 rows" is not something a shell can act on, and a
/// document holds several lists.
///
/// The `Err` case is deliberately narrow — it is only ever the *container*. A
/// `history` that is a number is not a lenient case; it is a document the core
/// cannot read, and quietly treating it as an empty list would be precisely the
/// silent wipe this crate exists to stop. `None` (the field was absent) and
/// `null` are both an empty list, matching `#[serde(default)]`.
pub fn items<T>(
    field: &str,
    raw: Option<Value>,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<T>, serde_json::Error>
where
    T: DeserializeOwned + Serialize,
{
    let rows: Vec<Value> = match raw {
        Some(v) => serde_json::from_value::<Option<Vec<Value>>>(v)?.unwrap_or_default(),
        None => Vec::new(),
    };

    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let subject = format!("{field}[{i}]");
        match serde_json::from_value::<T>(row.clone()) {
            Ok(parsed) => {
                report_coercions(&subject, &row, &parsed, warnings);
                out.push(parsed);
            }
            Err(e) => warnings.push(dropped(subject, &e)),
        }
    }
    Ok(out)
}

/// As [`items`], for a string-keyed map. This is the `{"someKey": null}` payload
/// the server documents a client may PUT.
///
/// The subject is `progress[tt1:S1E1]` — the key spelled out, because for a map
/// the key *is* the identity of the thing that was lost, and it is the same string
/// the shell would use to look the record up again.
pub fn entries<T>(
    field: &str,
    raw: Option<Value>,
    warnings: &mut Vec<Warning>,
) -> Result<BTreeMap<String, T>, serde_json::Error>
where
    T: DeserializeOwned + Serialize,
{
    let rows: BTreeMap<String, Value> = match raw {
        Some(v) => {
            serde_json::from_value::<Option<BTreeMap<String, Value>>>(v)?.unwrap_or_default()
        }
        None => BTreeMap::new(),
    };

    let mut out = BTreeMap::new();
    for (key, row) in rows {
        let subject = format!("{field}[{key}]");
        match serde_json::from_value::<T>(row.clone()) {
            Ok(parsed) => {
                report_coercions(&subject, &row, &parsed, warnings);
                out.insert(key, parsed);
            }
            Err(e) => warnings.push(dropped(subject, &e)),
        }
    }
    Ok(out)
}

/// The warning for a row that did not parse.
///
/// The serde message rides along verbatim, for the reason `parse.input` carries
/// it: "invalid type: boolean `true`, expected a string" is the difference between
/// a five-minute diagnosis and an afternoon, and it reports positions and expected
/// types rather than the user's data.
fn dropped(subject: String, e: &serde_json::Error) -> Warning {
    Warning::new(
        WarningCode::DroppedBadItem,
        subject,
        format!("row dropped; the rest of the document was kept ({e})"),
    )
}

/// Compare what was sent against what was kept, and warn about every modelled key
/// the two disagree on.
///
/// Field *names* and JSON *types* are reported; values are not. The shell already
/// holds this data, but a warning is a thing that ends up in a log, and "`title`
/// was not stored as sent" is every bit as actionable as quoting the title would
/// be without putting what someone watches into telemetry.
fn report_coercions<T: Serialize>(
    subject: &str,
    raw: &Value,
    parsed: &T,
    warnings: &mut Vec<Warning>,
) {
    let kept = match serde_json::to_value(parsed) {
        Ok(v) => v,
        // Unreachable for every type in this crate (no non-string map keys, no
        // non-finite floats that serde cannot render). "Unreachable" is not a
        // reason to panic, and a missing coercion warning is not a reason to fail
        // a call that otherwise succeeded.
        Err(_) => return,
    };
    let (raw, kept) = match (raw.as_object(), kept.as_object()) {
        (Some(a), Some(b)) => (a, b),
        // A scalar row — a tombstone's `u64` — has no fields to disagree about.
        _ => return,
    };

    for (key, before) in raw {
        let after = match kept.get(key) {
            Some(v) => v,
            // Either the core does not model this key, or it models it as an
            // `Option` that is now `None` because the value was `null`. Both are
            // the documented contract rather than a rewrite.
            None => continue,
        };
        if equivalent(before, after) {
            continue;
        }
        warnings.push(Warning::new(
            WarningCode::CoercedField,
            subject.to_string(),
            format!(
                "`{key}` was not stored as sent ({} -> {})",
                kind(before),
                kind(after)
            ),
        ));
    }
}

/// Structural equality with one deliberate looseness: two numbers are the same
/// number regardless of how JSON spelled them.
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y,
            // A u64/i64 outside f64's exact range: fall back to the literal
            // comparison rather than claim equality it cannot demonstrate.
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| equivalent(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|((ka, va), (kb, vb))| ka == kb && equivalent(va, vb))
        }
        _ => a == b,
    }
}

/// The JSON type name, for a warning detail that says what changed without saying
/// what the value was.
fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LibraryItem, Progress};
    use serde_json::json;

    fn subjects(ws: &[Warning]) -> Vec<&str> {
        ws.iter().map(|w| w.subject.as_str()).collect()
    }

    fn codes(ws: &[Warning]) -> Vec<&str> {
        ws.iter().map(|w| w.code.as_str()).collect()
    }

    /// The headline: the row is still dropped, and the drop is still on the wire.
    #[test]
    fn a_dropped_row_is_named() {
        let mut w = Vec::new();
        let out: Vec<LibraryItem> = items(
            "history",
            Some(json!([{"id":"good","at":2}, {"garbage":true}, {"id":"also","at":1}])),
            &mut w,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(codes(&w), vec!["dropped.bad_item"]);
        assert_eq!(subjects(&w), vec!["history[1]"]);
        assert!(
            w[0].detail.contains("id"),
            "the detail must say what failed: {}",
            w[0].detail
        );
    }

    /// A map key is its own identity — `progress[broken]`, not `progress[1]`.
    #[test]
    fn a_dropped_map_entry_is_named_by_its_key() {
        let mut w = Vec::new();
        let out: BTreeMap<String, Progress> = entries(
            "progress",
            Some(json!({"good":{"pos":1,"dur":2,"at":3}, "broken": null})),
            &mut w,
        )
        .unwrap();
        assert_eq!(out.keys().collect::<Vec<_>>(), vec!["good"]);
        assert_eq!(subjects(&w), vec!["progress[broken]"]);
    }

    /// The quieter half: the row is kept, so nothing looks wrong — which is why it
    /// has to be said out loud.
    #[test]
    fn a_coerced_field_is_named() {
        let mut w = Vec::new();
        let out: Vec<LibraryItem> = items(
            "history",
            Some(json!([{"id":603,"title":"The Matrix","year":1999,"at":5}])),
            &mut w,
        )
        .unwrap();
        assert_eq!(out[0].id(), "603");
        assert_eq!(codes(&w), vec!["coerced.field", "coerced.field"]);
        assert!(w.iter().all(|x| x.subject == "history[0]"));
        let details: Vec<&str> = w.iter().map(|x| x.detail.as_str()).collect();
        assert!(details.iter().any(|d| d.contains("`id`")), "{details:?}");
        assert!(details.iter().any(|d| d.contains("`year`")), "{details:?}");
        assert!(
            details.iter().all(|d| d.contains("number -> string")),
            "the detail must name the change, not the value: {details:?}"
        );
    }

    /// An explicit `null` where a number is modelled is a real loss — a resume
    /// position quietly becoming zero — and is the case that most needs a warning.
    #[test]
    fn a_nulled_number_is_a_coercion_not_an_absence() {
        let mut w = Vec::new();
        let out: BTreeMap<String, Progress> = entries(
            "progress",
            Some(json!({"k":{"pos":null,"dur":100,"at":5}})),
            &mut w,
        )
        .unwrap();
        assert_eq!(out["k"].pos, 0.0);
        assert_eq!(codes(&w), vec!["coerced.field"]);
        assert!(w[0].detail.contains("`pos`"), "{}", w[0].detail);
    }

    /// The two exclusions, asserted rather than assumed: serde rendering `1` as
    /// `1.0` is not news, and a key the core does not model was never promised
    /// back.
    #[test]
    fn ordinary_rows_are_silent() {
        let mut w = Vec::new();
        let _: Vec<LibraryItem> = items(
            "history",
            Some(json!([{"id":"a","title":"T","at":1,"poster":null,"whatever":{"x":1}}])),
            &mut w,
        )
        .unwrap();
        let _: BTreeMap<String, Progress> = entries(
            "progress",
            Some(json!({"k":{"pos":1,"dur":100,"at":5,"lang":"eng"}})),
            &mut w,
        )
        .unwrap();
        assert!(w.is_empty(), "{:?}", codes(&w));
    }

    /// A container that is not a container is a document the core cannot read, and
    /// must never degrade to "empty". That is the wipe, not the fix for it.
    #[test]
    fn a_broken_container_is_an_error_not_an_empty_list() {
        let mut w = Vec::new();
        assert!(items::<LibraryItem>("history", Some(json!(42)), &mut w).is_err());
        assert!(entries::<Progress>("progress", Some(json!([1, 2])), &mut w).is_err());
        // Absent and `null` are both simply empty.
        assert!(items::<LibraryItem>("history", None, &mut w)
            .unwrap()
            .is_empty());
        assert!(items::<LibraryItem>("history", Some(Value::Null), &mut w)
            .unwrap()
            .is_empty());
    }

    /// Tombstone maps are scalars, and a scalar has no fields to compare — the
    /// coercion walk must not invent one.
    #[test]
    fn scalar_rows_are_never_reported_as_coerced() {
        let mut w = Vec::new();
        let out: BTreeMap<String, u64> =
            entries("removed", Some(json!({"x": 50, "y": 4000})), &mut w).unwrap();
        assert_eq!(out.len(), 2);
        assert!(w.is_empty());
    }
}
