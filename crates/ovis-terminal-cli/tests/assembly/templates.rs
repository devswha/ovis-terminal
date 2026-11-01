//! `template.list` and `template.get` read `<state>/templates` on the real binary; an
//! unknown template is refused.

use serde_json::json;

use super::support::{Served, TestResult};

#[test]
fn templates_in_the_state_directory_are_listed_and_read_by_name() -> TestResult {
    let served = Served::start()?;
    let dir = served.root().join("state").join("templates");
    let template = json!({"name": "build", "version": "1", "description": "build agent",
        "argv": ["/usr/bin/env", "build-agent"]});
    std::fs::write(dir.join("build-1.json"), template.to_string())?;
    let listed = served.ok(&["call", "template.list", "--params", "{}"])?;
    let names: Vec<_> = listed
        .get("templates")
        .and_then(serde_json::Value::as_array)
        .map(|all| all.iter().filter_map(|t| t.get("name").cloned()).collect())
        .unwrap_or_default();
    assert_eq!(names, vec![json!("build")], "{listed}");
    let got = served.ok(&[
        "call",
        "template.get",
        "--params",
        &json!({"name": "build", "version": null}).to_string(),
    ])?;
    assert_eq!(got.get("version"), Some(&json!("1")), "{got}");
    let missing = served.refused(
        &[
            "call",
            "template.get",
            "--params",
            &json!({"name": "absent", "version": null}).to_string(),
        ],
        3,
    )?;
    assert_eq!(
        missing.get("kind"),
        Some(&json!("template_not_found")),
        "{missing}"
    );
    served.stop()?;
    Ok(())
}
