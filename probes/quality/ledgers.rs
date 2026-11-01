//! The binding ledgers: IDEAL and EXCL rows of `ideal-state.md`, GAP rows of `gap-list.md`.

use std::path::Path;

use super::error::QualityError;
use super::markdown::{ids, ledger_rows};

/// One IDEAL row.
#[derive(Debug, Clone)]
pub(super) struct Ideal {
    pub(super) id: String,
    pub(super) user: String,
    pub(super) statement: String,
    pub(super) reason: String,
    pub(super) needs: Vec<String>,
}

/// One GAP row and the IDEAL rows its citation names.
#[derive(Debug, Clone)]
pub(super) struct Gap {
    pub(super) id: String,
    pub(super) ideals: Vec<String>,
}

/// One EXCL row.
#[derive(Debug, Clone)]
pub(super) struct Exclusion {
    pub(super) id: String,
    pub(super) title: String,
}

/// Both ledgers, in ledger order.
#[derive(Debug, Clone, Default)]
pub(super) struct Ledgers {
    pub(super) ideals: Vec<Ideal>,
    pub(super) gaps: Vec<Gap>,
    pub(super) exclusions: Vec<Exclusion>,
}

impl Ledgers {
    /// Parses the two ledger texts; `None` names the table that is missing.
    pub(super) fn parse(ideal_state: &str, gap_list: &str) -> Result<Self, &'static str> {
        let ideals: Vec<Ideal> = ledger_rows(ideal_state, "IDEAL")
            .into_iter()
            .filter_map(|(id, row)| match row.as_slice() {
                [_, user, statement, reason, evidence] => Some(Ideal {
                    id,
                    user: (*user).to_owned(),
                    statement: (*statement).to_owned(),
                    reason: (*reason).to_owned(),
                    needs: ids(evidence, "NEED"),
                }),
                _ => None,
            })
            .collect();
        let exclusions: Vec<Exclusion> = ledger_rows(ideal_state, "EXCL")
            .into_iter()
            .filter_map(|(id, row)| {
                let title = row.get(1)?;
                Some(Exclusion {
                    id,
                    title: (*title).to_owned(),
                })
            })
            .collect();
        let gaps: Vec<Gap> = ledger_rows(gap_list, "GAP")
            .into_iter()
            .map(|(id, row)| Gap {
                ideals: row
                    .last()
                    .map_or_else(Vec::new, |citation| ids(citation, "IDEAL")),
                id,
            })
            .collect();
        match (ideals.is_empty(), gaps.is_empty(), exclusions.is_empty()) {
            (true, _, _) => Err("an IDEAL table"),
            (_, true, _) => Err("a GAP table"),
            (_, _, true) => Err("an EXCL table"),
            (false, false, false) => Ok(Self {
                ideals,
                gaps,
                exclusions,
            }),
        }
    }

    /// Reads and parses both ledger files.
    pub(super) fn read(ideal_state: &Path, gap_list: &Path) -> Result<Self, QualityError> {
        let ideal_text =
            std::fs::read_to_string(ideal_state).map_err(QualityError::io(ideal_state))?;
        let gap_text = std::fs::read_to_string(gap_list).map_err(QualityError::io(gap_list))?;
        Self::parse(&ideal_text, &gap_text).map_err(|missing| QualityError::Ledger {
            path: if missing == "a GAP table" {
                gap_list.to_path_buf()
            } else {
                ideal_state.to_path_buf()
            },
            missing: missing.to_owned(),
        })
    }

    pub(super) fn ideal_ids(&self) -> Vec<String> {
        self.ideals.iter().map(|ideal| ideal.id.clone()).collect()
    }

    pub(super) fn gap_ids(&self) -> Vec<String> {
        self.gaps.iter().map(|gap| gap.id.clone()).collect()
    }

    pub(super) fn exclusion_ids(&self) -> Vec<String> {
        self.exclusions
            .iter()
            .map(|exclusion| exclusion.id.clone())
            .collect()
    }
}

/// `<PREFIX>-001` through `<PREFIX>-<count>`, the ids a `--require-*` count asks for.
pub(super) fn required(prefix: &str, count: u16) -> Vec<String> {
    (1..=count)
        .map(|number| format!("{prefix}-{number:03}"))
        .collect()
}
