//! The end of a control boot: every screen saved before a planned handover, a goodbye
//! to every holder, then every link, viewer and reader thread joined.

use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use ovis_terminal_holder::protocol::GoodbyeReason;

use super::super::error::SoakError;
use super::super::wire::Event;
use super::links::Outbound;
use super::{Control, Inbound};

impl Control {
    /// Saves every screen at its head before a planned handover.
    pub(super) fn store_all(&mut self) -> Result<(), SoakError> {
        for screen in self.screens.iter_mut().flatten() {
            screen.unfreeze();
        }
        self.checkpoint(self.screens.len())?;
        let offsets = self.offsets();
        self.hub.publish(Event::HandedOver { offsets })
    }

    pub(super) fn finish(
        self,
        (handed_over, events): (bool, Receiver<Inbound>),
        (threads, reader): (Vec<JoinHandle<()>>, JoinHandle<bool>),
    ) -> Result<(), SoakError> {
        for link in self.links.iter().flatten() {
            if handed_over {
                link.send(Outbound::Goodbye(GoodbyeReason::Handover))
                    .map_err(|_| SoakError::ControlGone {
                        waiting_for: "a link to say goodbye",
                    })?;
            }
        }
        drop(self.links);
        drop(events);
        self.hub.close()?;
        for thread in threads {
            thread.join().map_err(|_| SoakError::Protocol {
                expected: "a holder link thread that ends",
            })?;
        }
        reader.join().map(drop).map_err(|_| SoakError::Protocol {
            expected: "a command reader that ends",
        })
    }
}
