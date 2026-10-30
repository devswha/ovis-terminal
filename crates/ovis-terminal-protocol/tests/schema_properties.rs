//! Property tests: any narrowing edit at any depth is reported; any widening edit is not.

#[path = "schema_properties/edits.rs"]
pub mod edits;
#[path = "schema_properties/tree.rs"]
pub mod tree;

use edits::{count, edit_at, narrow, widen};
use ovis_terminal_protocol::schema::breaking_changes;
use proptest::prelude::*;
use tree::{document, node};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn any_narrowing_at_any_depth_is_reported(root in node(), pick in any::<prop::sample::Index>()) {
        let mut target = Some(pick.index(count(&root)));
        let narrowed = edit_at(&root, &mut target, narrow);
        let breaks = breaking_changes(&document(&root), &document(&narrowed)).expect("finite schemas compare");
        prop_assert!(!breaks.is_empty(), "narrowing passed:\n{root:?}\n->\n{narrowed:?}");
    }

    #[test]
    fn any_widening_at_any_depth_is_compatible(root in node(), pick in any::<prop::sample::Index>()) {
        let mut target = Some(pick.index(count(&root)));
        let widened = edit_at(&root, &mut target, widen);
        let breaks = breaking_changes(&document(&root), &document(&widened)).expect("finite schemas compare");
        prop_assert!(breaks.is_empty(), "widening broke {breaks:?}:\n{root:?}\n->\n{widened:?}");
    }
}
