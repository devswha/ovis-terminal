//! Accepted `.ok();` discards: each names its file, the discarded call and a reason, and
//! an entry that no longer matches a discard fails the check.

use serde::Deserialize;

use super::checklist::Dispositions;
use super::report::Finding;

/// One accepted `.ok();` discard: the file, the called `receiver.method` or function, why.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DroppedResultExemption {
    pub(super) path: String,
    pub(super) call: String,
    pub(super) reason: String,
}

/// `findings` without the accepted discards, plus a finding for each exemption that
/// matched nothing or gives no reason.
pub(super) fn apply_exemptions(table: &Dispositions, findings: Vec<Finding>) -> Vec<Finding> {
    let exempts = |finding: &Finding, exemption: &DroppedResultExemption| {
        finding.rule == "dropped-result"
            && finding
                .location
                .strip_prefix(&exemption.path)
                .is_some_and(|rest| rest.starts_with(':'))
            && finding.detail.starts_with(&format!("`{}(", exemption.call))
    };
    let mut kept = Vec::new();
    let mut used = vec![false; table.dropped_result_exemptions.len()];
    for finding in findings {
        let matched = table
            .dropped_result_exemptions
            .iter()
            .position(|exemption| exempts(&finding, exemption));
        match matched.and_then(|index| used.get_mut(index)) {
            Some(slot) => *slot = true,
            None => kept.push(finding),
        }
    }
    for (exemption, matched) in table.dropped_result_exemptions.iter().zip(used) {
        if !matched || exemption.reason.trim().is_empty() {
            let detail = format!(
                "the exemption for `{}` matches no discard or gives no reason",
                exemption.call
            );
            kept.push(Finding::new(
                "stale-exemption",
                exemption.path.clone(),
                detail,
            ));
        }
    }
    kept
}
