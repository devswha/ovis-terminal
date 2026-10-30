//! Cross-checks the mirror against the real vte parser: every callback vte
//! makes for a byte must be one vte can only make from the state the mirror
//! claims it was in, for arbitrary bytes including C0s inside sequences.

use alacritty_terminal::vte::{Params, Parser, Perform};
use proptest::prelude::*;

use super::VteState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Callback {
    Print,
    Execute(u8),
    Csi,
    Esc,
    Osc,
    Hook,
    Put,
    Unhook,
}

#[derive(Default)]
struct Recorder(Vec<Callback>);

/// vte fixes these four- and five-parameter signatures.
macro_rules! record {
    ($($name:ident($($arg:ident: $ty:ty),*) => $callback:expr;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) { self.0.push($callback) })*
    };
}

impl Perform for Recorder {
    record! {
        csi_dispatch(_p: &Params, _i: &[u8], _g: bool, _a: char) => Callback::Csi;
        esc_dispatch(_i: &[u8], _g: bool, _b: u8) => Callback::Esc;
        hook(_p: &Params, _i: &[u8], _g: bool, _a: char) => Callback::Hook;
        print(_c: char) => Callback::Print;
        execute(byte: u8) => Callback::Execute(byte);
        put(_b: u8) => Callback::Put;
        unhook() => Callback::Unhook;
        osc_dispatch(_p: &[&[u8]], _b: bool) => Callback::Osc;
    }
}

fn possible_from(state: VteState, callback: Callback) -> bool {
    match callback {
        Callback::Print => state == VteState::Ground,
        Callback::Execute(0x18 | 0x1a) => true,
        Callback::Execute(_) => matches!(
            state,
            VteState::Ground | VteState::Escape | VteState::EscapeIntermediate | VteState::Csi
        ),
        Callback::Csi => state == VteState::Csi,
        Callback::Esc => matches!(state, VteState::Escape | VteState::EscapeIntermediate),
        Callback::Osc => state == VteState::Osc,
        Callback::Hook => {
            matches!(
                state,
                VteState::DcsEntry | VteState::DcsParam | VteState::DcsIntermediate
            )
        }
        Callback::Put | Callback::Unhook => state == VteState::DcsPassthrough,
    }
}

/// Callbacks vte must make for `byte` in `state`, beyond those it may make.
fn required(state: VteState, byte: u8) -> Option<Callback> {
    match (state, byte) {
        (VteState::Osc, 0x07) => Some(Callback::Osc),
        (VteState::DcsPassthrough, 0x20..=0x7e) => Some(Callback::Put),
        (VteState::Escape, 0x30..=0x4f) => Some(Callback::Esc),
        _ => None,
    }
}

fn interesting_byte() -> impl Strategy<Value = u8> {
    prop_oneof![
        any::<u8>(),
        prop::sample::select(vec![
            0x1b, b'[', b']', b'P', b'X', b'^', b'_', 0x07, 0x18, 0x1a, 0x9c, b';', b'\\', 0x00,
            0x01, 0x0e, 0x0f, 0x7f, b'q', b'm', b'7', b' ',
        ]),
    ]
}

proptest! {
    #[test]
    fn mirror_agrees_with_every_vte_callback(bytes in prop::collection::vec(interesting_byte(), 0..400)) {
        let mut parser = Parser::new();
        let mut state = VteState::Ground;
        for byte in bytes {
            let mut recorder = Recorder::default();
            parser.advance(&mut recorder, &[byte]);
            for callback in &recorder.0 {
                prop_assert!(possible_from(state, *callback), "{callback:?} from {state:?} on {byte:#04x}");
            }
            if let Some(callback) = required(state, byte) {
                prop_assert!(recorder.0.contains(&callback), "{state:?} on {byte:#04x} missed {callback:?}");
            }
            state = state.next(byte);
        }
    }
}
