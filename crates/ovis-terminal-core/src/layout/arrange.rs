//! Swap, zoom, ratio and resize; ranges are reported, never clamped or rounded.

use super::edit::{Effect, missing};
use super::geometry::first_extent;
use super::model::{Child, Layout, Ratio, Tab};
use super::{GeometryReason, GeometryRejection, LayoutError};
use crate::events::ResourceId;
use crate::registry::{PaneId, SplitId};

fn substitute(tab: &mut Tab, first: PaneId, second: PaneId) {
    let flip = |pane: PaneId| match pane {
        pane if pane == first => second,
        pane if pane == second => first,
        pane => pane,
    };
    tab.for_each_slot(|slot| {
        if let Child::Pane(pane) = slot {
            *pane = flip(*pane);
        }
    });
    tab.zoomed = tab.zoomed.map(flip);
}

pub(super) fn swap(
    layout: &mut Layout,
    first: PaneId,
    second: PaneId,
) -> Result<Effect, LayoutError> {
    let (at_first, at_second) = (layout.pane_at(first)?, layout.pane_at(second)?);
    if first == second {
        return Ok(Effect::unchanged());
    }
    let record = |tab: &Tab, pane: PaneId| tab.panes.iter().find(|p| p.id == pane).cloned();
    let moved_first = record(layout.tab(at_first)?, first).ok_or_else(missing)?;
    let moved_second = record(layout.tab(at_second)?, second).ok_or_else(missing)?;
    let mut tabs = Vec::new();
    let mut scopes = Vec::new();
    for (at, leaving, arriving) in [
        (at_first, first, &moved_second),
        (at_second, second, &moved_first),
    ] {
        scopes.push(layout.workspaces.get(at.0).ok_or_else(missing)?.id);
        let tab = layout.tab_mut(at)?;
        if !tabs.contains(&tab.id) {
            substitute(tab, first, second);
            tabs.push(tab.id);
        }
        if at_first != at_second
            && let Some(slot) = tab.panes.iter_mut().find(|pane| pane.id == leaving)
        {
            *slot = arriving.clone();
        }
    }
    let mut changed = vec![ResourceId::Pane(first), ResourceId::Pane(second)];
    changed.extend(tabs.iter().copied().map(ResourceId::Tab));
    Ok(Effect {
        changed,
        scopes,
        tabs,
        ..Effect::default()
    })
}

pub(super) fn zoom(layout: &mut Layout, pane: PaneId, zoomed: bool) -> Result<Effect, LayoutError> {
    let at = layout.pane_at(pane)?;
    let scope = layout.workspaces.get(at.0).ok_or_else(missing)?.id;
    let tab = layout.tab_mut(at)?;
    let next = if zoomed { Some(pane) } else { None };
    if (zoomed && tab.zoomed == next) || (!zoomed && tab.zoomed != Some(pane)) {
        return Ok(Effect::unchanged());
    }
    let mut changed = vec![ResourceId::Pane(pane), ResourceId::Tab(tab.id)];
    changed.extend(
        tab.zoomed
            .filter(|previous| *previous != pane)
            .map(ResourceId::Pane),
    );
    tab.zoomed = next;
    Ok(Effect {
        changed,
        scopes: vec![scope],
        tabs: vec![tab.id],
        ..Effect::default()
    })
}

pub(super) fn set_ratio(
    layout: &mut Layout,
    split: SplitId,
    permille: u16,
) -> Result<Effect, LayoutError> {
    let at = layout.split_at(split)?;
    let ratio = Ratio::new(permille)?;
    apply_ratio(layout, (at, split), ratio)
}

fn apply_ratio(
    layout: &mut Layout,
    (at, split): (super::model::TabAt, SplitId),
    ratio: Ratio,
) -> Result<Effect, LayoutError> {
    let scope = layout.workspaces.get(at.0).ok_or_else(missing)?.id;
    let tab = layout.tab_mut(at)?;
    let node = tab
        .splits
        .iter_mut()
        .find(|node| node.id == split)
        .ok_or_else(missing)?;
    if node.ratio == ratio {
        return Ok(Effect::unchanged());
    }
    node.ratio = ratio;
    Ok(Effect {
        changed: vec![ResourceId::Split(split), ResourceId::Tab(tab.id)],
        scopes: vec![scope],
        tabs: vec![tab.id],
        ..Effect::default()
    })
}

const fn gcd(mut left: i64, mut right: i64) -> i64 {
    while let Some(rest) = left.checked_rem(right) {
        left = right;
        right = rest;
    }
    left
}

/// Accepted divider movements: `[min, max]` in steps that land on a whole permille.
struct Range {
    min: i64,
    max: i64,
    step: i64,
    empty: bool,
}

fn range(extent: i64, current: i64) -> Option<Range> {
    let ratio_step = 1000_i64.checked_div(gcd(extent, 1000))?;
    let min_ratio = i64::from(Ratio::MIN)
        .checked_add(ratio_step)?
        .checked_sub(1)?
        .checked_div(ratio_step)?
        .checked_mul(ratio_step)?;
    let max_ratio = i64::from(Ratio::MAX)
        .checked_div(ratio_step)?
        .checked_mul(ratio_step)?;
    let to_extent = |ratio: i64| extent.checked_mul(ratio)?.checked_div(1000);
    Some(Range {
        min: to_extent(min_ratio)?.checked_sub(current)?,
        max: to_extent(max_ratio)?.checked_sub(current)?,
        step: to_extent(ratio_step)?,
        empty: min_ratio > max_ratio,
    })
}

/// Moves a divider by a tab-relative amount when the result is an exact permille share.
pub(super) fn resize(
    layout: &mut Layout,
    split: SplitId,
    delta_ppm: i32,
) -> Result<Effect, LayoutError> {
    let at = layout.split_at(split)?;
    if delta_ppm == 0 {
        return Ok(Effect::unchanged());
    }
    let tab = layout.tab(at)?;
    let node = tab
        .splits
        .iter()
        .find(|node| node.id == split)
        .ok_or_else(missing)?;
    let rect = tab
        .geometry()?
        .splits
        .into_iter()
        .find(|geometry| geometry.split == split)
        .ok_or_else(missing)?
        .rect;
    let extent = i64::from(rect.along(node.axis)).max(1);
    let current = i64::from(first_extent(rect.along(node.axis), node.ratio.permille())?);
    let Range {
        min,
        max,
        step,
        empty,
    } = range(extent, current).ok_or(LayoutError::LimitExceeded)?;
    let requested = i64::from(delta_ppm);
    let offset = requested
        .checked_sub(min)
        .and_then(|offset| offset.checked_rem(step));
    let reason = if empty || requested < min || requested > max {
        Some(GeometryReason::OutOfRange)
    } else if offset != Some(0) {
        Some(GeometryReason::Unrepresentable)
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(LayoutError::InvalidGeometry(GeometryRejection {
            reason,
            requested,
            min,
            max,
            step,
        }));
    }
    let permille = current
        .checked_add(requested)
        .and_then(|target| target.checked_mul(1000))
        .and_then(|scaled| scaled.checked_div(extent))
        .ok_or(LayoutError::LimitExceeded)?;
    let ratio = Ratio::new(u16::try_from(permille).map_err(|_| missing())?)?;
    apply_ratio(layout, (at, split), ratio)
}
