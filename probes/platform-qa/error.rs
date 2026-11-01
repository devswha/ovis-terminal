//! The platform QA boundary error.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Why a smoke run, a CI verification or a lane manifest failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum QaError {
    /// The command line is incomplete or malformed.
    #[error("{detail}")]
    Usage {
        /// What is wrong with the arguments.
        detail: String,
    },
    /// A file or process operation failed.
    #[error("could not {operation}")]
    Io {
        /// The operation that failed.
        operation: &'static str,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A JSON document did not parse into its typed shape.
    #[error("{what} is not valid")]
    Json {
        /// The document that failed.
        what: String,
        /// The underlying error.
        #[source]
        source: serde_json::Error,
    },
    /// A manifest parsed but breaks one of its rules.
    #[error("{file}: {detail}")]
    ManifestInvalid {
        /// The manifest file.
        file: PathBuf,
        /// The broken rule.
        detail: String,
    },
    /// A spawned child reported a process id that must never be signalled.
    #[error("the child reported pid {pid}")]
    UnsafePid {
        /// The reported process id.
        pid: u32,
    },
    /// The GitHub CLI exited unsuccessfully.
    #[error("{argv:?} exited with {exit_code:?}: {stderr}")]
    Gh {
        /// The full command line.
        argv: Vec<String>,
        /// Its exit code, absent when it was killed.
        exit_code: Option<i32>,
        /// Its standard error.
        stderr: String,
    },
    /// No completed workflow run exists for the commit.
    #[error("no completed {workflow} run at {sha}")]
    RunNotFound {
        /// The workflow file name.
        workflow: String,
        /// The commit.
        sha: String,
    },
    /// The job did not conclude with success.
    #[error("job {job} concluded {conclusion}")]
    JobNotGreen {
        /// The job name.
        job: String,
        /// Its conclusion, or `missing` when the run has no such job.
        conclusion: String,
    },
    /// The report names another commit than the one verified.
    #[error("the report is for {reported}, not {expected}")]
    ShaMismatch {
        /// The commit being verified.
        expected: String,
        /// The commit the report names.
        reported: String,
    },
    /// A required check is absent from the report.
    #[error("check {name} is not in the report")]
    CheckMissing {
        /// The check name.
        name: String,
    },
    /// A required check ran and did not pass.
    #[error("check {name} did not pass")]
    CheckFailed {
        /// The check name.
        name: String,
    },
    /// The report's overall verdict contradicts its checks.
    #[error("{detail}")]
    ReportInconsistent {
        /// The contradiction.
        detail: String,
    },
    /// At least one smoke check failed in this run.
    #[error("{failed} smoke check(s) failed")]
    SmokeFailed {
        /// How many checks failed.
        failed: usize,
    },
}

impl QaError {
    /// Stable machine-readable kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::Io { .. } => "io",
            Self::Json { .. } => "invalid_json",
            Self::ManifestInvalid { .. } => "manifest_invalid",
            Self::UnsafePid { .. } => "unsafe_pid",
            Self::Gh { .. } => "gh_failed",
            Self::RunNotFound { .. } => "run_not_found",
            Self::JobNotGreen { .. } => "job_not_green",
            Self::ShaMismatch { .. } => "sha_mismatch",
            Self::CheckMissing { .. } => "check_missing",
            Self::CheckFailed { .. } => "check_failed",
            Self::ReportInconsistent { .. } => "report_inconsistent",
            Self::SmokeFailed { .. } => "smoke_failed",
        }
    }

    /// What to do about it.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "pass every required flag; see the workflow for an example",
            Self::Io { .. } => "check the path exists and is writable",
            Self::Json { .. } => "regenerate the document; never edit a report by hand",
            Self::ManifestInvalid { .. } => "fix the manifest so it matches its documented shape",
            Self::UnsafePid { .. } => "never signal pid 0 or 1; investigate the spawn",
            Self::Gh { .. } => "authenticate gh and confirm the repository and workflow exist",
            Self::RunNotFound { .. } => "push the commit and wait for the workflow to finish",
            Self::JobNotGreen { .. } => "open the run log and fix the failing job",
            Self::ShaMismatch { .. } => "download the artifact of the run for this exact commit",
            Self::CheckMissing { .. } => "add the check to the smoke manifest",
            Self::CheckFailed { .. } => "read the check's captured log in the artifact",
            Self::ReportInconsistent { .. } => "the report is corrupt; rerun the smoke job",
            Self::SmokeFailed { .. } => "read each failed check's log in the output directory",
        }
    }
}
