//! Published exit codes and the error kinds each one covers.

use std::process::ExitCode;

/// The process exit status; codes are stable once published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExitStatus {
    /// The command succeeded.
    Success,
    /// A failure whose kind no other code covers.
    Failure,
    /// The invocation is malformed; nothing was sent.
    Usage,
    /// The named resource does not exist.
    NotFound,
    /// No server answers, or it cannot serve the request now.
    Unavailable,
    /// The command needs an explicit target; candidates are listed.
    NeedsTarget,
    /// The program waits on a prompt.
    Blocked,
    /// The operation was cancelled.
    Cancelled,
    /// The request conflicts with current state.
    Conflict,
    /// The principal is not authenticated or not allowed.
    Forbidden,
    /// Effects may have begun; inspect the operation.
    OutcomeUnknown,
}

impl ExitStatus {
    /// Every status, in code order.
    pub const ALL: [Self; 11] = [
        Self::Success,
        Self::Failure,
        Self::Usage,
        Self::NotFound,
        Self::Unavailable,
        Self::NeedsTarget,
        Self::Blocked,
        Self::Cancelled,
        Self::Conflict,
        Self::Forbidden,
        Self::OutcomeUnknown,
    ];

    /// The status an error kind selects; kinds no status names are plain failures.
    #[must_use]
    pub fn for_kind(kind: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|status| status.kinds().contains(&kind))
            .unwrap_or(Self::Failure)
    }

    /// The numeric exit code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Usage => 2,
            Self::NotFound => 3,
            Self::Unavailable => 4,
            Self::NeedsTarget => 7,
            Self::Blocked => 9,
            Self::Cancelled => 10,
            Self::Conflict => 11,
            Self::Forbidden => 12,
            Self::OutcomeUnknown => 13,
        }
    }

    /// The stable name of the status.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Usage => "usage",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::NeedsTarget => "needs_target",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Conflict => "conflict",
            Self::Forbidden => "forbidden",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    /// The error kinds, from this binary or a server, that select the status.
    #[must_use]
    pub const fn kinds(self) -> &'static [&'static str] {
        match self {
            Self::Success | Self::Failure => &[],
            Self::Usage => &[
                "usage",
                "invalid_params",
                "unknown_method",
                "invalid_json_switch",
                "invalid_session",
                "invalid_endpoint_override",
                "no_runtime_directory",
                "invalid_user",
                "socket_path_too_long",
                "unsupported_path",
                "invalid_pipe_name",
                "invalid_account",
                "privileged_account",
                "password_required",
                "not_a_tty",
            ],
            Self::NotFound => &[
                "not_found",
                "template_not_found",
                "parent_not_found",
                "stream_not_found",
                "not_registered",
            ],
            Self::Unavailable => &[
                "unavailable",
                "server_not_running",
                "stale_endpoint",
                "method_not_found",
                "machine_unreachable",
                "unsupported_transport",
                "backpressure",
                "recovering",
                "not_ready",
                "service_manager_unavailable",
                "service_unsupported",
                "connection_lost",
            ],
            Self::NeedsTarget => &["needs_target", "ambiguous_target"],
            Self::Blocked => &["blocked"],
            Self::Cancelled => &["cancelled"],
            Self::Conflict => &[
                "conflict",
                "op_reused",
                "lease_conflict",
                "version_conflict",
                "revision_mismatch",
                "already_running",
                "already_stopped",
                "duplicate_name",
                "stale_target",
                "foreign_registration",
                "identity_mismatch",
                "service_exists",
                "lease_held",
            ],
            Self::Forbidden => &[
                "forbidden",
                "unauthenticated",
                "invalid_credential",
                "credential_expired",
                "credential_revoked",
                "peer_rejected",
                "not_authorized",
                "no_lease",
                "raw_not_authorized",
                "takeover_not_authorized",
                "attach_refused",
            ],
            Self::OutcomeUnknown => &[
                "outcome_unknown",
                "deadline_exceeded",
                "manager_timeout",
                "holder_not_ready",
            ],
        }
    }
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status.code())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::ExitStatus;

    #[test]
    fn codes_are_distinct_and_every_kind_selects_exactly_its_status() {
        let codes: BTreeSet<u8> = ExitStatus::ALL.iter().map(|status| status.code()).collect();
        assert_eq!(codes.len(), ExitStatus::ALL.len());
        let kinds: Vec<&str> = ExitStatus::ALL
            .iter()
            .flat_map(|status| status.kinds().iter().copied())
            .collect();
        let distinct: BTreeSet<&str> = kinds.iter().copied().collect();
        assert_eq!(distinct.len(), kinds.len(), "a kind maps to two statuses");
        for status in ExitStatus::ALL {
            for kind in status.kinds() {
                assert_eq!(ExitStatus::for_kind(kind), status);
            }
        }
    }

    #[test]
    fn an_unknown_kind_from_a_newer_server_is_a_plain_failure() {
        assert_eq!(
            ExitStatus::for_kind("a_kind_from_a_newer_server"),
            ExitStatus::Failure
        );
    }
}
