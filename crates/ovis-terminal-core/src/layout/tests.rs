use std::collections::BTreeMap;

use ovis_terminal_protocol::schema::scalar::Revision;
use proptest::prelude::*;

use super::edit::{Fresh, apply};
use super::{Axis, CloseScope, EXTENT, Edit, Layout, LayoutError, Target};
use crate::registry::{PaneId, SplitId, TabId, WorkspaceId};

#[derive(Debug, Clone)]
struct Step {
    kind: u8,
    first: usize,
    second: usize,
    ratio: u16,
    delta: i32,
}

fn step() -> impl Strategy<Value = Step> {
    (
        0_u8..10,
        0_usize..64,
        0_usize..64,
        0_u16..1100,
        -600_000_i32..600_000,
    )
        .prop_map(|(kind, first, second, ratio, delta)| Step {
            kind,
            first,
            second,
            ratio,
            delta,
        })
}

fn pick<T: Copy>(items: &[T], index: usize) -> Option<T> {
    items.get(index.checked_rem(items.len())?).copied()
}

fn ids(layout: &Layout) -> (Vec<WorkspaceId>, Vec<TabId>, Vec<PaneId>, Vec<SplitId>) {
    let workspaces = layout.workspaces.iter().map(|w| w.id).collect();
    let tabs = layout.workspaces.iter().flat_map(|w| &w.tabs);
    let panes = tabs.clone().flat_map(|t| &t.panes).map(|p| p.id).collect();
    let splits = tabs.clone().flat_map(|t| &t.splits).map(|s| s.id).collect();
    (workspaces, tabs.map(|t| t.id).collect(), panes, splits)
}

fn edit_for(layout: &Layout, step: &Step) -> Option<Edit> {
    let (workspaces, tabs, panes, splits) = ids(layout);
    let axis = if step.first.is_multiple_of(2) {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    Some(match step.kind {
        0 => Edit::CreateWorkspace {
            name: format!("w{}", step.first),
            group_with: pick(&workspaces, step.second).filter(|_| step.first.is_multiple_of(3)),
        },
        1 => Edit::CreateTab {
            workspace: pick(&workspaces, step.first)?,
        },
        2 => Edit::Split {
            pane: pick(&panes, step.first)?,
            axis,
            ratio_permille: step.ratio,
        },
        3 => Edit::Close {
            target: Target::Pane(pick(&panes, step.first)?),
            scope: CloseScope::Target,
        },
        4 => Edit::MoveTab {
            tab: pick(&tabs, step.first)?,
            workspace: pick(&workspaces, step.second)?,
            index: 0,
        },
        5 => Edit::MovePane {
            pane: pick(&panes, step.first)?,
            workspace: pick(&workspaces, step.second)?,
        },
        6 => Edit::Swap {
            first: pick(&panes, step.first)?,
            second: pick(&panes, step.second)?,
        },
        7 => Edit::Zoom {
            pane: pick(&panes, step.first)?,
            zoomed: step.second.is_multiple_of(2),
        },
        8 => Edit::SetRatio {
            split: pick(&splits, step.first)?,
            ratio_permille: step.ratio,
        },
        _ => Edit::Resize {
            split: pick(&splits, step.first)?,
            delta_ppm: step.delta,
        },
    })
}

fn fresh(counter: &mut u64) -> Result<Fresh, TestCaseError> {
    *counter = counter
        .checked_add(1)
        .ok_or_else(|| TestCaseError::fail("counter"))?;
    let next = *counter;
    let fail = |error: crate::registry::IdentityError| TestCaseError::fail(error.to_string());
    Ok(Fresh {
        workspace: Some(WorkspaceId::try_from(next).map_err(fail)?),
        tab: Some(TabId::try_from(next).map_err(fail)?),
        pane: Some(PaneId::try_from(next).map_err(fail)?),
        split: Some(SplitId::try_from(next).map_err(fail)?),
    })
}

fn splits_by_id(layout: &Layout) -> BTreeMap<SplitId, (Axis, super::Child, super::Child)> {
    let tabs = layout.workspaces.iter().flat_map(|w| &w.tabs);
    tabs.flat_map(|t| &t.splits)
        .map(|s| (s.id, (s.axis, s.first, s.second)))
        .collect()
}

fn check_invariants(layout: &Layout) -> Result<(), TestCaseError> {
    let (_, tabs, panes, splits) = ids(layout);
    let unique = |count: usize, mut items: Vec<String>| {
        items.sort();
        items.dedup();
        items.len() == count
    };
    prop_assert!(unique(
        tabs.len(),
        tabs.iter().map(ToString::to_string).collect()
    ));
    prop_assert!(unique(
        panes.len(),
        panes.iter().map(ToString::to_string).collect()
    ));
    prop_assert!(unique(
        splits.len(),
        splits.iter().map(ToString::to_string).collect()
    ));
    for tab in layout.workspaces.iter().flat_map(|w| &w.tabs) {
        prop_assert!(tab.validate().is_ok());
        let geometry = tab
            .geometry()
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(geometry.panes.len(), tab.panes.len());
        let area = geometry.panes.iter().try_fold(0_u64, |total, pane| {
            u64::from(pane.rect.width)
                .checked_mul(u64::from(pane.rect.height))
                .and_then(|cell| total.checked_add(cell))
        });
        if tab.root.is_some() && tab.zoomed.is_none() {
            prop_assert_eq!(area, u64::from(EXTENT).checked_mul(u64::from(EXTENT)));
        }
    }
    let encoded = serde_json::to_vec(layout).map_err(|e| TestCaseError::fail(e.to_string()))?;
    let decoded: Layout =
        serde_json::from_slice(&encoded).map_err(|e| TestCaseError::fail(e.to_string()))?;
    prop_assert_eq!(&decoded, layout);
    Ok(())
}

proptest! {
    #[test]
    fn edit_sequences_keep_identities_trees_and_exact_tiling(steps in prop::collection::vec(step(), 1..60)) {
        let mut layout = Layout::empty();
        let mut counter = 0;
        for step in &steps {
            let Some(edit) = edit_for(&layout, step) else { continue };
            let before = layout.clone();
            let splits_before = splits_by_id(&layout);
            match apply(&mut layout, &edit, fresh(&mut counter)?) {
                Ok(effect) => {
                    prop_assert_eq!(&layout.selections, &before.selections);
                    if effect.no_op.is_some() {
                        prop_assert_eq!(&layout, &before);
                    }
                    let after = splits_by_id(&layout);
                    for (id, (axis, _, _)) in &splits_before {
                        if let Some((kept, _, _)) = after.get(id) {
                            prop_assert_eq!(kept, axis, "split {} kept its identity and axis", id);
                        }
                    }
                }
                Err(error) => {
                    prop_assert!(matches!(
                        error,
                        LayoutError::InvalidGeometry(_)
                            | LayoutError::InvalidParams { .. }
                            | LayoutError::GroupScopeRequired { .. }
                            | LayoutError::LimitExceeded
                    ), "unexpected {error:?}");
                    prop_assert_eq!(&layout, &before, "a rejected edit left no partial change");
                }
            }
            check_invariants(&layout)?;
        }
    }

    #[test]
    fn revisions_round_trip_as_canonical_decimal_strings(value in any::<u64>()) {
        let mut layout = Layout::empty();
        layout.revision = Revision::new(value);
        let encoded = serde_json::to_value(&layout).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(encoded.get("revision"), Some(&serde_json::Value::String(value.to_string())));
        let decoded: Layout = serde_json::from_value(encoded).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(decoded.revision.get(), value);
    }
}

#[test]
fn numeric_revision_is_rejected_and_u64_max_round_trips() {
    // Given: the largest revision and the same value in numeric form.
    let text = format!(
        r#"{{"revision":"{}","workspaces":[],"selections":[]}}"#,
        u64::MAX
    );
    let numeric = format!(
        r#"{{"revision":{},"workspaces":[],"selections":[]}}"#,
        u64::MAX
    );

    // When: both are decoded at the layout boundary.
    let decoded: Result<Layout, _> = serde_json::from_str(&text);
    let rejected: Result<Layout, _> = serde_json::from_str(&numeric);

    // Then: only the canonical decimal string is accepted, without loss.
    assert_eq!(
        decoded.map(|layout| layout.revision.get()).ok(),
        Some(u64::MAX)
    );
    assert!(rejected.is_err());
}
