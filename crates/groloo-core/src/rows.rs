//! The seven surviving lines of Heart's `catalog.rs` — which home rows render.
//!
//! Heart owned a 14-entry `HOME_ROWS` const, a `Catalog` model, a `CatMsg` enum
//! and a per-row `LoadStatus`. All of it is gone except the one rule that is
//! actually a *rule* rather than data: a row renders when its gating add-on is
//! installed AND (it is a studio row OR its per-row toggle is on).
//!
//! The table itself is an ARGUMENT, not a constant. Heart's copy had already
//! drifted from the shell's — 14 entries with `studios` inline against the
//! shell's 13 plus a special-cased `StudioRow` — and the i18n keys baked into the
//! Rust const resolve against a dictionary that lives in a third repository. Row
//! order is data; data belongs on the wire, not in a compiled table that two
//! teams have to remember to edit together.
//!
//! `CatMsg::SetGating` — the message the shell had to forward from
//! `runtime::Model` because Heart's two reducers could not see each other — is
//! now simply the second argument. Problem 3 dissolves rather than gets solved.

use serde::Deserialize;
use std::collections::BTreeMap;

/// One row in the shell's table. `kind` selects which gating add-on governs it.
///
/// Unknown `kind` values are tolerated at parse time and treated as ungated-off
/// (see [`Gating::allows`]) rather than rejected: a newer shell shipping a new row
/// kind must degrade to "that row does not render on this core", never to "the
/// whole home page failed to gate".
#[derive(Debug, Clone, Deserialize)]
pub struct RowDecl {
    #[serde(default)]
    pub cat: String,
    #[serde(default)]
    pub kind: String,
}

/// Which of the three gating add-ons are installed.
///
/// Field names are the shell's own (`config.catalog` / `.providers` / `.studios`
/// in `homeConfig`), so the shell can pass its gating object straight through
/// without building a translation layer that could itself drift.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Gating {
    #[serde(default)]
    pub catalog: bool,
    #[serde(default)]
    pub providers: bool,
    #[serde(default)]
    pub studios: bool,
}

impl Gating {
    /// Is the add-on that governs rows of this `kind` installed?
    ///
    /// The singular/plural mismatch (`provider` the row kind, `providers` the
    /// add-on) is Heart's, and both spellings are accepted so the shell's table
    /// does not have to be rewritten to adopt this function.
    fn installed(self, kind: &str) -> bool {
        match kind {
            "catalog" => self.catalog,
            "provider" | "providers" => self.providers,
            "studio" | "studios" => self.studios,
            _ => false,
        }
    }

    /// The whole rule, verbatim from `catalog.rs:150-156`: the gating add-on is
    /// installed AND the row is either a studio row (which has no per-row toggle)
    /// or its toggle is on. An ABSENT toggle means ON — a row the user has never
    /// touched is a row they have not turned off.
    fn allows(self, row: &RowDecl, config: &BTreeMap<String, bool>) -> bool {
        if !self.installed(&row.kind) {
            return false;
        }
        matches!(row.kind.as_str(), "studio" | "studios") || config.get(&row.cat) != Some(&false)
    }
}

/// The visible cats, in the input table's order.
///
/// Order is preserved from the argument and never re-derived: the shell renders
/// in the order it declared, and a core that sorted here would silently reorder
/// someone's home page.
pub fn visible_rows(
    rows: &[RowDecl],
    gating: Gating,
    config: &BTreeMap<String, bool>,
) -> Vec<String> {
    rows.iter()
        .filter(|r| gating.allows(r, config))
        .map(|r| r.cat.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell's real table (`src/lib/home.ts`), abbreviated to one row per kind
    /// plus the studio special case. Using the real cats keeps the test honest
    /// about what the strings actually look like.
    fn table() -> Vec<RowDecl> {
        serde_json::from_str(
            r#"[{"cat":"trending_movie","kind":"catalog"},
                {"cat":"top_movie","kind":"catalog"},
                {"cat":"prov_netflix","kind":"provider"},
                {"cat":"studios","kind":"studio"}]"#,
        )
        .unwrap()
    }

    fn cfg(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn nothing_installed_shows_nothing() {
        let out = visible_rows(&table(), Gating::default(), &BTreeMap::new());
        assert!(out.is_empty());
    }

    #[test]
    fn gating_selects_by_kind() {
        let g = Gating {
            catalog: true,
            providers: false,
            studios: false,
        };
        assert_eq!(
            visible_rows(&table(), g, &BTreeMap::new()),
            vec!["trending_movie", "top_movie"]
        );
    }

    /// The default that matters: a cat with no entry in the config renders.
    #[test]
    fn absent_toggle_means_on() {
        let g = Gating {
            catalog: true,
            providers: true,
            studios: true,
        };
        assert_eq!(
            visible_rows(&table(), g, &BTreeMap::new()),
            vec!["trending_movie", "top_movie", "prov_netflix", "studios"]
        );
    }

    #[test]
    fn explicit_false_hides_one_row() {
        let g = Gating {
            catalog: true,
            providers: true,
            studios: true,
        };
        let out = visible_rows(&table(), g, &cfg(&[("top_movie", false)]));
        assert_eq!(out, vec!["trending_movie", "prov_netflix", "studios"]);
    }

    /// The studio row has no per-row toggle — turning it "off" in the config must
    /// not hide it, because nothing in the UI can turn it back on.
    #[test]
    fn studio_row_ignores_its_toggle() {
        let g = Gating {
            catalog: false,
            providers: false,
            studios: true,
        };
        let out = visible_rows(&table(), g, &cfg(&[("studios", false)]));
        assert_eq!(out, vec!["studios"]);
    }

    /// Input order is the output order, even when the table is shuffled — proof
    /// the core does not carry an opinion about row order any more.
    #[test]
    fn input_order_is_preserved() {
        let rows: Vec<RowDecl> = serde_json::from_str(
            r#"[{"cat":"top_movie","kind":"catalog"},{"cat":"trending_movie","kind":"catalog"}]"#,
        )
        .unwrap();
        let g = Gating {
            catalog: true,
            ..Default::default()
        };
        assert_eq!(
            visible_rows(&rows, g, &BTreeMap::new()),
            vec!["top_movie", "trending_movie"]
        );
    }

    /// Forward compatibility: a row kind this core has never heard of hides that
    /// row and nothing else.
    #[test]
    fn unknown_kind_hides_only_itself() {
        let rows: Vec<RowDecl> = serde_json::from_str(
            r#"[{"cat":"future","kind":"podcast"},{"cat":"trending_movie","kind":"catalog"}]"#,
        )
        .unwrap();
        let g = Gating {
            catalog: true,
            providers: true,
            studios: true,
        };
        assert_eq!(
            visible_rows(&rows, g, &BTreeMap::new()),
            vec!["trending_movie"]
        );
    }
}
