use ovis_terminal_protocol::schema::limits::Limits;

use super::super::super::limits::StreamLimits;

const MIB: u64 = 1024 * 1024;

#[test]
fn served_streams_pass_a_sequence_of_the_advertised_8_mib() {
    let limits = StreamLimits::advertised(&Limits::v1());
    assert_eq!(limits.graphics.sequence_max_bytes.get(), 8 * MIB);
    assert_eq!(
        limits.graphics.sequence_max_bytes,
        Limits::v1().graphics_sequence_max_bytes,
        "the stream cap is the one hello advertises"
    );
}

#[test]
fn the_qa_cap_replaces_only_the_sequence_cap() {
    let advertised = StreamLimits::advertised(&Limits::v1());
    let lowered = advertised.with_graphics_sequence_max_bytes(4096);
    assert_eq!(lowered.graphics.sequence_max_bytes.get(), 4096);
    assert_eq!(
        lowered.graphics.terminal_store_max_bytes,
        advertised.graphics.terminal_store_max_bytes
    );
    assert_eq!(
        lowered.with_graphics_sequence_max_bytes(8 * MIB),
        advertised
    );
}
