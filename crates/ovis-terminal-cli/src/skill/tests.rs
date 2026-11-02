use super::MANUAL;

const SHIPPED_MANUAL: &str = include_str!("../../../../docs/skill/ovis-terminal.md");

#[test]
fn the_shipped_manual_is_the_embedded_manual() {
    assert_eq!(SHIPPED_MANUAL, MANUAL);
}
