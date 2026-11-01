//! Reading one exact-filtered libtest run: the named test ran once and passed.

/// Why a captured libtest run does not prove its test passed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NotProven {
    /// The process exited unsuccessfully or was killed.
    Exit(Option<i32>),
    /// No `test <name> ... ok` line: the test failed or never ran.
    NoOkLine,
    /// The summary is missing or counts anything but exactly one pass.
    Summary(String),
}

/// Checks one `cargo test <name> -- --exact` run proves the test passed.
///
/// It needs exit 0, one `test <name> ... ok` line and the summary
/// `1 passed; 0 failed; 0 ignored`, so an empty filter, an ignored test and a
/// failure are all rejected.
///
/// # Errors
///
/// The first of those conditions that does not hold.
pub fn proven(stdout: &str, name: &str, exit_code: Option<i32>) -> Result<(), NotProven> {
    if exit_code != Some(0) {
        return Err(NotProven::Exit(exit_code));
    }
    let ok_line = format!("test {name} ... ok");
    if stdout
        .lines()
        .filter(|line| line.trim_end() == ok_line)
        .count()
        != 1
    {
        return Err(NotProven::NoOkLine);
    }
    let summaries: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("test result: "))
        .collect();
    match summaries.as_slice() {
        [summary] if summary.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;") => {
            Ok(())
        }
        other => Err(NotProven::Summary(other.join(" | "))),
    }
}

#[cfg(test)]
mod tests {
    use super::{NotProven, proven};

    const NAME: &str = "listen::pipe::tests::hello";

    fn run(lines: &[&str]) -> String {
        lines.join("\n")
    }

    #[test]
    fn one_exact_pass_is_proven() {
        let out = run(&[
            "running 1 test",
            "test listen::pipe::tests::hello ... ok",
            "",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 900 filtered out; finished in 0.20s",
        ]);
        assert_eq!(proven(&out, NAME, Some(0)), Ok(()));
    }

    #[test]
    fn a_filter_that_matched_nothing_is_not_a_pass() {
        let out = run(&[
            "running 0 tests",
            "",
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 901 filtered out; finished in 0.00s",
        ]);
        assert_eq!(proven(&out, NAME, Some(0)), Err(NotProven::NoOkLine));
    }

    #[test]
    fn an_ignored_test_is_not_a_pass() {
        let out = run(&[
            "running 1 test",
            "test listen::pipe::tests::hello ... ignored, needs the CI account",
            "",
            "test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 900 filtered out",
        ]);
        assert_eq!(proven(&out, NAME, Some(0)), Err(NotProven::NoOkLine));
    }

    #[test]
    fn a_second_summary_or_a_failed_exit_is_rejected() {
        let ok = "test listen::pipe::tests::hello ... ok";
        let pass = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured";
        let twice = run(&[ok, pass, pass]);
        assert!(matches!(
            proven(&twice, NAME, Some(0)),
            Err(NotProven::Summary(_))
        ));
        assert_eq!(
            proven(&run(&[ok, pass]), NAME, Some(101)),
            Err(NotProven::Exit(Some(101)))
        );
        assert_eq!(
            proven(&run(&[ok, pass]), NAME, None),
            Err(NotProven::Exit(None))
        );
    }

    #[test]
    fn a_longer_test_sharing_the_prefix_does_not_count() {
        let out = run(&[
            "test listen::pipe::tests::hello_twice ... ok",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured",
        ]);
        assert_eq!(proven(&out, NAME, Some(0)), Err(NotProven::NoOkLine));
    }
}
