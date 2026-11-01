//! Graphics placements the emulator cannot hold (plan todo 84).
//!
//! alacritty keeps no image state: an APC is skipped, and a sixel `DCS q` and an
//! `OSC 1337` are unhandled (`docs/evidence/task-84/before.log`). This module reads each
//! Kitty, iTerm2 or sixel sequence by its header only, keyed by the raw offset where it
//! began, and remembers where it was placed: the cursor cell at receipt. A placement is
//! released when its anchor row leaves the visible screen, when its anchor cell is
//! erased or overwritten, when its alternate screen is left, when a Kitty `a=d` names it
//! or a transmission reuses its id, on RIS, or when more than [`IMAGE_LIMIT`] images are
//! retained. A paint names where each image still shown is re-sent; the bytes
//! themselves are kept by the terminal's image store, never here.

mod anchor;
mod header;
mod plan;
mod scan;
mod set;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use alacritty_terminal::grid::Grid;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Cell;
use ovis_terminal_protocol::domain::graphics::GraphicsProtocol;
use serde::{Deserialize, Serialize};

use self::anchor::{Anchor, Placing, Presence};
use self::header::{Header, Kitty};
pub(super) use self::scan::{Scanner, Seen};
pub use self::set::{ImagePlace, ImageSet, RetainedImage};
use crate::engine::Vt;
use crate::engine::units::OutputOffset;

/// Images one engine retains; the oldest is released past this.
pub const IMAGE_LIMIT: usize = 1024;
/// Releases remembered, so the image store can say why an image left.
const RELEASED_LIMIT: usize = 4096;

/// Why a retained graphics sequence is no longer shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ImageRelease {
    /// Its anchor row left the visible screen.
    ScrolledOut,
    /// Its anchor cell was erased or overwritten, or its alternate screen was left.
    Erased,
    /// A Kitty `a=d` command named it.
    Deleted,
    /// A Kitty transmission reused its image id.
    Replaced,
    /// RIS reset the terminal.
    Reset,
    /// More than [`IMAGE_LIMIT`] images were retained; the oldest went first.
    Limit,
}

/// Where a paint re-sends one retained image.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageSplice {
    /// Index into the paint's bytes the image is inserted before.
    pub at: usize,
    /// The image: the raw offset where its sequence began.
    pub key: OutputOffset,
    /// Bytes sent first: a cursor position on its anchor cell, empty for a
    /// transmission that displays nothing itself.
    pub cursor: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A Kitty transmission without a placement (`a=t`).
    Data,
    /// A sequence that displays itself: Kitty `a=T`, iTerm2 inline, sixel.
    Shown,
    /// A Kitty put (`a=p`) of an earlier image.
    Put,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct KittyRef {
    id: Option<u32>,
    number: Option<u32>,
}

impl KittyRef {
    fn named_by(self, other: Self) -> bool {
        (other.id.is_some() && self.id == other.id)
            || (other.number.is_some() && self.number == other.number)
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    key: OutputOffset,
    protocol: GraphicsProtocol,
    role: Role,
    kitty: KittyRef,
    extent: (u16, u16),
    anchor: Option<Anchor>,
}

/// Retained images of one engine, in receipt order.
#[derive(Debug, Default)]
pub(super) struct Images {
    entries: VecDeque<Entry>,
    released: VecDeque<(OutputOffset, ImageRelease)>,
    /// A Kitty transmission announced another chunk; its chunks add nothing.
    series: bool,
}

impl Images {
    /// Applies a completed sequence whose cursor cell is `placing`'s.
    pub(super) fn seen(&mut self, seen: Seen, placing: &mut Placing<'_>) {
        match seen {
            Seen::Reset => {
                self.series = false;
                self.release_where(|_| true, ImageRelease::Reset);
            }
            Seen::Graphics { key, header } => self.graphics((key, header), placing),
        }
        while self.entries.len() > IMAGE_LIMIT {
            if let Some(oldest) = self.entries.pop_front() {
                self.note(oldest.key, ImageRelease::Limit);
            }
        }
    }

    fn graphics(&mut self, (key, header): (OutputOffset, Header), placing: &mut Placing<'_>) {
        let mut entry = Entry {
            key,
            protocol: header.protocol(),
            role: Role::Shown,
            kitty: KittyRef::default(),
            extent: (0, 0),
            anchor: None,
        };
        match header {
            Header::Kitty(kitty) => self.kitty((entry, kitty), placing),
            Header::Iterm2(image) if image.inline => {
                entry.extent = (image.rows, image.columns);
                self.place(entry, placing);
            }
            Header::Iterm2(_) => {}
            Header::Sixel => self.place(entry, placing),
        }
    }

    fn kitty(&mut self, (mut entry, kitty): (Entry, Kitty), placing: &mut Placing<'_>) {
        if std::mem::replace(&mut self.series, kitty.more) {
            return;
        }
        entry.kitty = KittyRef {
            id: kitty.id,
            number: kitty.number,
        };
        entry.extent = (kitty.rows, kitty.columns);
        match kitty.action {
            b'd' => self.delete(kitty),
            b't' => {
                self.replace(entry.kitty);
                entry.role = Role::Data;
                self.entries.push_back(entry);
            }
            b'T' => {
                self.replace(entry.kitty);
                self.place(entry, placing);
            }
            b'p' if entry.kitty.id.is_some() || entry.kitty.number.is_some() => {
                entry.role = Role::Put;
                self.place(entry, placing);
            }
            _ => {}
        }
    }

    fn place(&mut self, mut entry: Entry, placing: &mut Placing<'_>) {
        entry.anchor = Some(Anchor::place(placing));
        self.entries.push_back(entry);
    }

    /// A new transmission with an id replaces every image of that id.
    fn replace(&mut self, kitty: KittyRef) {
        if let Some(id) = kitty.id {
            self.release_where(
                |entry| entry.protocol == GraphicsProtocol::Kitty && entry.kitty.id == Some(id),
                ImageRelease::Replaced,
            );
        }
    }

    /// `a=d`: lower case removes placements, upper case also their image data. Targets
    /// other than ids and numbers are treated as "all", so nothing deleted is re-sent.
    fn delete(&mut self, kitty: Kitty) {
        let target = KittyRef {
            id: kitty.id,
            number: kitty.number,
        };
        let data_too = kitty.delete.is_ascii_uppercase();
        let named = |entry: &Entry| match kitty.delete.to_ascii_lowercase() {
            b'i' => target.id.is_some() && entry.kitty.id == target.id,
            b'n' => target.number.is_some() && entry.kitty.number == target.number,
            _ => true,
        };
        self.release_where(
            |entry| {
                entry.protocol == GraphicsProtocol::Kitty
                    && (entry.role != Role::Data || data_too)
                    && named(entry)
            },
            ImageRelease::Deleted,
        );
    }

    fn release_where(&mut self, released: impl Fn(&Entry) -> bool, reason: ImageRelease) {
        let (gone, kept): (Vec<Entry>, Vec<Entry>) = self.entries.drain(..).partition(released);
        self.entries = kept.into();
        for entry in gone {
            self.note(entry.key, reason);
        }
    }

    fn note(&mut self, key: OutputOffset, reason: ImageRelease) {
        self.released.push_back((key, reason));
        while self.released.len() > RELEASED_LIMIT {
            self.released.pop_front();
        }
    }

    /// Releases placements output moved off the screen or erased.
    pub(super) fn follow(&mut self, grid: &Grid<Cell>, alternate: bool) {
        let mut gone = Vec::new();
        for entry in &mut self.entries {
            let Some(anchor) = entry.anchor.as_mut() else {
                continue;
            };
            let presence = match (anchor.alternate, alternate) {
                (true, false) => Presence::Erased,
                (false, true) => continue,
                _ => anchor.follow(grid),
            };
            match presence {
                Presence::Shown => {}
                Presence::ScrolledOut => gone.push((entry.key, ImageRelease::ScrolledOut)),
                Presence::Erased => gone.push((entry.key, ImageRelease::Erased)),
            }
        }
        for (key, reason) in gone {
            self.release_where(|entry| entry.key == key, reason);
        }
    }
}

impl Vt {
    /// Retained graphics sequences in receipt order, with where each is placed now.
    pub fn images(&self) -> ImageSet {
        self.images.set()
    }

    pub(super) fn receive_image(&mut self, seen: Seen) {
        let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
        let mut placing = Placing {
            grid: self.term.grid_mut(),
            alternate,
        };
        self.images.seen(seen, &mut placing);
    }

    pub(super) fn follow_images(&mut self) {
        let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
        self.images.follow(self.term.grid(), alternate);
    }
}
