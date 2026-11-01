//! Terminal output corpora: a colored build log, an SGR change on every cell, and wide
//! plus combining characters.

fn repeat_to(unit: &[u8], len: usize) -> Vec<u8> {
    unit.iter().copied().cycle().take(len).collect()
}

/// Colored build-log lines with cursor moves and erases.
pub(super) fn build_log(len: usize) -> Vec<u8> {
    let mut unit = Vec::new();
    for line in 0..64_usize {
        let colour = 31_usize.saturating_add(line % 7);
        let text = format!(
            "\x1b[{colour};1m[{line:03}]\x1b[0m compiling crate-{line} v0.1.0 (/src/{line})\x1b[K\r\n"
        );
        unit.extend_from_slice(text.as_bytes());
    }
    repeat_to(&unit, len)
}

/// A new SGR sequence before every printed cell: the parser's worst common case.
pub(super) fn sgr_dense(len: usize) -> Vec<u8> {
    let mut unit = Vec::new();
    for cell in 0..120_usize {
        let text = format!("\x1b[38;5;{}m#", cell % 256);
        unit.extend_from_slice(text.as_bytes());
    }
    unit.extend_from_slice(b"\r\n");
    repeat_to(&unit, len)
}

/// Double-width CJK, emoji and combining marks.
pub(super) fn wide(len: usize) -> Vec<u8> {
    let unit = "\u{4e2d}\u{6587}\u{1f600} e\u{301}a\u{308} \u{d55c}\u{ae00}\r\n".repeat(16);
    repeat_to(unit.as_bytes(), len)
}

/// A paced emitter's output for one second at `rate` bytes.
pub(super) fn second(rate: usize, seed: usize) -> Vec<u8> {
    let mut log = build_log(rate.saturating_add(seed % 64));
    log.drain(..seed % 64);
    log
}
