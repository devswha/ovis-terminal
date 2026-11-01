use super::parse;

const BLOCK: &str = "## Todos\n\n- [x] 80. Fix a flake\n  Closes: none (regression fix) | Delivers: IDEAL-050 ordering\n  Parallelization: Wave 11 | Blocked by: 25, 77 | Blocks: 61 | Write scope: crates/a/tests/c.rs, crates/a/src/conn/ (only if proven), docs/evidence/task-80/\n  QA scenarios (exact tool + invocation): happy: shell loop; failure: shell loop on the pre-fix tree\n- [ ] 83. Pass graphics\n  Closes: GAP-151 (attach drops graphics) | Delivers: IDEAL-118\n  Parallelization: Wave 15 | Blocked by: 23 | Blocks: 84 | Write scope: `crates/b/`, `conformance/scenarios/graphics/passthrough.json`\n  QA scenarios: happy: `conformance --scenario graphics/passthrough --case happy`; failure: `conformance --scenario all/all --case failure`. Must NOT reproduce EXCL-016.\n\n> **Scope decision:** todos 38 and 39 (parsers) were removed from the engine.\n";

#[test]
fn fields_are_read_wherever_they_sit_on_a_line() {
    let plan = parse(BLOCK);
    let flake = plan.todo(80).unwrap();
    assert!(flake.landed);
    assert_eq!(flake.closes, Vec::<String>::new());
    assert_eq!(flake.delivers, ["IDEAL-050"]);
    assert_eq!(
        (flake.wave, flake.blocked_by.clone(), flake.blocks.clone()),
        (Some(11), vec![25, 77], vec![61])
    );
    assert_eq!(
        flake.write_scope,
        [
            "crates/a/tests/c.rs",
            "crates/a/src/conn/",
            "docs/evidence/task-80/"
        ]
    );
    let graphics = plan.todo(83).unwrap();
    assert_eq!(
        (graphics.closes.as_slice(), graphics.delivers.as_slice()),
        (&["GAP-151".to_owned()][..], &["IDEAL-118".to_owned()][..])
    );
    assert_eq!(
        graphics.write_scope,
        [
            "crates/b/",
            "conformance/scenarios/graphics/passthrough.json"
        ]
    );
    assert_eq!(graphics.scenarios, ["graphics/passthrough"]);
    assert_eq!(graphics.excludes, ["EXCL-016"]);
    assert!(graphics.has_qa);
}

#[test]
fn a_scope_decision_accounts_for_removed_todos() {
    let plan = parse(BLOCK);
    assert_eq!(plan.removed, [38, 39]);
    assert!(plan.accounts_for(38) && plan.accounts_for(83) && !plan.accounts_for(40));
}
