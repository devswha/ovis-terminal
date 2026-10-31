//! Random interleavings of lease operations keep ownership, epochs and fencing consistent.

use std::collections::{BTreeMap, BTreeSet};

use ovis_terminal_core::leases::{
    Acquire, ControlSlot, LeaseEvent, LeaseSlot, TerminalLeases, Transition, Write, WriteKind,
};
use proptest::collection::vec;
use proptest::prelude::{ProptestConfig, proptest};
use proptest::test_runner::TestCaseError;

use super::TestResult;
use super::fixture::{Client, Fixture, OBSERVER, TAKEOVER, takeover};

#[derive(Debug, Default)]
struct Ledger {
    epochs: BTreeMap<LeaseSlot, u64>,
    ids: BTreeSet<u64>,
}

impl Ledger {
    fn record(&mut self, transition: &Transition) -> TestResult {
        if let [
            LeaseEvent::Revoked {
                displaced_by: Some(by),
                ..
            },
            next,
        ] = transition.events.as_slice()
        {
            let granted = match next {
                LeaseEvent::Acquired { lease, .. } | LeaseEvent::Geometry { lease, .. } => lease,
                LeaseEvent::Revoked { .. } | _ => return Err("takeover without a grant".into()),
            };
            if granted != by {
                return Err("revocation does not name the new owner".into());
            }
        }
        for event in &transition.events {
            let (LeaseEvent::Acquired { lease, .. } | LeaseEvent::Geometry { lease, .. }) = event
            else {
                continue;
            };
            if !self.ids.insert(lease.lease_id.get()) {
                return Err(format!("lease id {} granted twice", lease.lease_id).into());
            }
            let last = self.epochs.insert(lease.slot, lease.epoch.get());
            if lease.slot != LeaseSlot::Observe
                && last.is_some_and(|last| last >= lease.epoch.get())
            {
                return Err(format!("{:?} epoch did not advance", lease.slot).into());
            }
        }
        Ok(())
    }
}

fn step(
    fixture: &mut Fixture,
    leases: &mut TerminalLeases,
    (who, op): (&Client, u8),
) -> TestResult<Option<Transition>> {
    let slot = if op % 2 == 0 {
        ControlSlot::Interact
    } else {
        ControlSlot::Geometry
    };
    let view = leases.view();
    let current = match slot {
        ControlSlot::Interact => view.interact,
        ControlSlot::Geometry | _ => view.geometry,
    };
    let outcome = match op {
        0 => leases
            .observe(who.caller(), fixture.fresh()?)
            .map(|granted| granted.transition),
        1 | 2 => leases
            .acquire(who.caller(), Acquire::new(slot, fixture.fresh()?))
            .map(|granted| granted.transition),
        3 | 4 => match current {
            Some(lease) => {
                let request = takeover(slot, lease.epoch, fixture.fresh()?)?;
                leases
                    .take_over(who.caller(), &request)
                    .map(|granted| granted.transition)
            }
            None => return Ok(None),
        },
        5 | 6 => match current {
            Some(lease) => leases.release(who.caller(), lease.lease_id),
            None => return Ok(None),
        },
        _ => leases.disconnect(&who.owner()),
    };
    Ok(outcome.ok())
}

fn check_writes(leases: &TerminalLeases, clients: &[Client]) -> TestResult {
    let Some(interact) = leases.view().interact else {
        return Ok(());
    };
    for client in clients {
        let write = Write::new(WriteKind::Input, Some(interact.lease_id));
        let allowed = leases.authorize_write(client.caller(), write);
        match (client.owner() == interact.owner, allowed) {
            (true, Ok(_)) => {}
            (false, Err(error)) if matches!(error.kind(), "no_lease" | "forbidden") => {}
            (owner, outcome) => {
                return Err(format!("owner={owner} write outcome {outcome:?}").into());
            }
        }
    }
    Ok(())
}

fn run(ops: &[(usize, u8)]) -> TestResult {
    let mut fixture = Fixture::new()?;
    let clients = [
        fixture.client("alpha", TAKEOVER)?,
        fixture.client("beta", TAKEOVER)?,
        fixture.client("gamma", TAKEOVER)?,
        fixture.client("phone", OBSERVER)?,
    ];
    let mut leases = fixture.leases()?;
    let mut ledger = Ledger::default();
    for (index, op) in ops {
        let who = clients.get(*index).ok_or("actor out of range")?;
        if let Some(transition) = step(&mut fixture, &mut leases, (who, *op))? {
            ledger.record(&transition)?;
        }
        let view = leases.view();
        let mut held = BTreeSet::new();
        for lease in view
            .interact
            .iter()
            .chain(&view.geometry)
            .chain(&view.observers)
        {
            if !held.insert(lease.lease_id) {
                return Err("one lease id holds two slots".into());
            }
        }
        check_writes(&leases, &clients)?;
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn interleaved_operations_keep_one_owner_per_slot(ops in vec((0_usize..4, 0_u8..8), 1..40)) {
        run(&ops).map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
