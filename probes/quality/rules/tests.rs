use super::syntax_rules;

fn rules_of(source: &str) -> Vec<&'static str> {
    let file = syn::parse_file(source).unwrap();
    syntax_rules("t.rs", &file, 3)
        .into_iter()
        .map(|finding| finding.rule)
        .collect()
}

#[test]
fn self_comparisons_of_pure_expressions_cannot_fail() {
    for source in [
        "fn t() { let since = 1; assert!(since >= since); }",
        "fn t(state: S) { assert_eq!(state.seq, state.seq); }",
        "fn t() { debug_assert!(true); }",
    ] {
        assert_eq!(rules_of(source), ["tautological-assert"], "{source}");
    }
}

#[test]
fn comparing_two_evaluations_of_a_call_is_a_real_check() {
    for source in [
        "fn t() { assert_ne!(fresh_challenge(), fresh_challenge()); }",
        "fn t() -> R { assert_eq!(read_os_boot()?, read_os_boot()?); Ok(()) }",
        "fn t(a: u8, b: u8) { assert!(a >= b); }",
    ] {
        assert_eq!(rules_of(source), Vec::<&str>::new(), "{source}");
    }
}

#[test]
fn only_values_worth_handling_are_dropped_results() {
    assert_eq!(
        rules_of("fn t() { let _ = std::fs::remove_file(\"x\"); }"),
        ["dropped-result"]
    );
    assert_eq!(
        rules_of("fn t(v: V) { let _ = v.flush(); }"),
        ["dropped-result"]
    );
    assert_eq!(
        rules_of("fn t(v: V) { let _ = v; let _guard = v.lock(); }"),
        Vec::<&str>::new()
    );
}

#[test]
fn allow_is_found_directly_and_inside_cfg_attr() {
    assert_eq!(
        rules_of("#[allow(dead_code)] fn t() {}"),
        ["allow-attribute"]
    );
    assert_eq!(
        rules_of("#[cfg_attr(test, allow(unused))] fn t() {}"),
        ["allow-attribute"]
    );
    assert_eq!(
        rules_of("#[expect(dead_code, reason = \"seam\")] fn t() {}"),
        Vec::<&str>::new()
    );
}

#[test]
fn only_numeric_cast_targets_are_reported() {
    assert_eq!(
        rules_of("fn t(v: u64) -> f64 { v as f64 }"),
        ["numeric-cast"]
    );
    assert_eq!(
        rules_of("fn t(v: &u8) -> *const u8 { v as *const u8 }"),
        Vec::<&str>::new()
    );
}

#[test]
fn the_receiver_counts_toward_three_parameters() {
    assert_eq!(
        rules_of("impl S { fn m(&self, a: u8, b: u8, c: u8) {} }"),
        ["max-parameters"]
    );
    assert_eq!(
        rules_of("impl S { fn m(&self, a: u8, b: u8) {} }"),
        Vec::<&str>::new()
    );
}
