//! The checker's verdicts on its checked-in boundary fixtures.

use std::path::Path;

use workspace_hygiene::check::check_source;
use workspace_hygiene::fixtures::{self, Fixture};
use workspace_hygiene::report::Finding;

fn findings(fixture: &Fixture) -> Vec<Finding> {
    check_source(Path::new(fixture.path), fixture.source)
        .into_iter()
        .map(|violation| violation.finding)
        .collect()
}

#[test]
fn exactly_250_pure_loc_and_three_parameters_pass() {
    assert_eq!(
        findings(fixtures::find("boundary-250-loc-three-parameters").unwrap()),
        Vec::new()
    );
}

#[test]
fn one_line_and_one_parameter_over_the_limit_both_fail() {
    assert_eq!(
        findings(fixtures::find("excessive-loc-and-parameters").unwrap()),
        vec![
            Finding::PureLoc { count: 251 },
            Finding::Parameters {
                function: "too_many".to_owned(),
                count: 4
            },
        ]
    );
}

#[test]
fn violations_report_file_line_rule_and_count() {
    let fixture = fixtures::find("excessive-loc-and-parameters").unwrap();
    let lines: Vec<String> = check_source(Path::new(fixture.path), fixture.source)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        lines,
        vec![
            format!("{}:1: rule=max-pure-loc count=251 limit=250", fixture.path),
            format!(
                "{}:8: rule=max-parameters fn=too_many count=4 limit=3",
                fixture.path
            ),
        ]
    );
}

#[test]
fn malformed_source_is_a_syntax_violation() {
    let found = findings(fixtures::find("malformed-syntax").unwrap());
    assert!(
        matches!(found.as_slice(), [Finding::Syntax { .. }]),
        "expected one syntax finding, got {found:?}"
    );
}

#[test]
fn receivers_count_as_parameters() {
    let source = "struct S;\nimpl S {\n    fn m(&self, a: u8, b: u8, c: u8) {}\n}\n";
    let found: Vec<Finding> = check_source(Path::new("receiver.rs"), source)
        .into_iter()
        .map(|violation| violation.finding)
        .collect();
    assert_eq!(
        found,
        vec![Finding::Parameters {
            function: "m".to_owned(),
            count: 4
        }]
    );
}
