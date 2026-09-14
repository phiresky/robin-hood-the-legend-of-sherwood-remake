//! Golden byte-level captures of parity payload lines. Replay tooling parses
//! these; any change to the expected strings is a protocol change.

#[test]
fn forecast_line_is_byte_stable() {
    let line = super::Forecast {
        out_x: &(-1.5f32),
        out_y: &2.0f32,
        sector: &7u16,
        gates: &3usize,
        input: &("door", Some(-4i32)),
        layer: &0u8,
        direction: &15u16,
        entry_gate: &None::<u32>,
    }
    .to_string();
    assert_eq!(
        line,
        r#"FORECAST input=("door", Some(-4)) out=(-1.5, 2, sector=7, layer=0) dir=15 gates=3 entry=None"#
    );
}
