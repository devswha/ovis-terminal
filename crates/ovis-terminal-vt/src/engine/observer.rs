//! The tracker's `vte::Perform`: it records which kind of completion one byte
//! produced and classifies OSC strings from their raw size.

use alacritty_terminal::vte::{Params, Perform};

use crate::engine::osc::{OscContent, OscTerminator, RawOsc, classify};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    /// Printable character, CSI or ESC dispatch: the parser is in ground state.
    Ground,
    /// OSC or DCS string end: ground, unless ESC ended it and opened an escape.
    StringEnd { bell: bool },
    /// C0 control executed, possibly inside an open sequence.
    Control,
}

#[derive(Default)]
pub(super) struct Observer {
    pub(super) completions: u64,
    pub(super) last: Option<Completion>,
    pub(super) osc: Option<(OscTerminator, OscContent)>,
    /// Raw size of the OSC being parsed, counted before vte drops fields.
    pub(super) raw: RawOsc,
}

impl Observer {
    pub(super) fn new(raw: RawOsc) -> Self {
        Self {
            raw,
            ..Self::default()
        }
    }

    const fn complete(&mut self, completion: Completion) {
        self.completions = self.completions.saturating_add(1);
        self.last = Some(completion);
    }
}

/// Implements the vte callbacks whose signatures vte fixes at four or five
/// parameters; the observer only needs to know that they completed.
macro_rules! dispatch_observers {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) { self.complete(Completion::Ground) })*
    };
}

impl Perform for Observer {
    dispatch_observers! {
        csi_dispatch(_params: &Params, _intermediates: &[u8], _ignore: bool, _action: char);
        esc_dispatch(_intermediates: &[u8], _ignore: bool, _byte: u8);
    }

    fn print(&mut self, _: char) {
        self.complete(Completion::Ground);
    }

    fn execute(&mut self, _: u8) {
        self.complete(Completion::Control);
    }

    fn unhook(&mut self) {
        self.complete(Completion::StringEnd { bell: false });
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.complete(Completion::StringEnd {
            bell: bell_terminated,
        });
        let terminator = if bell_terminated {
            OscTerminator::Bell
        } else {
            OscTerminator::StringTerminator
        };
        self.osc = classify(params, self.raw).map(|content| (terminator, content));
    }
}
