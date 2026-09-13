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

#[test]
fn considerreport_merge_start_line_is_byte_stable() {
    let line = super::ConsiderreportMergeStart {
        owner: &42u32,
        incoming: &vec![(1u32, "a\"b\\c\n")],
        known_before: &Vec::<u32>::new(),
        frame: &9u32,
        flags: &0x10u32,
    }
    .to_string();
    assert_eq!(
        line,
        r#"CONSIDERREPORT {"stage":"merge_start","frame":9,"owner":42,"flags":16,"incoming":[(1, "a\"b\\c\n")],"known_before":[]}"#
    );
}

#[test]
fn aidecision_goto_enter_line_is_byte_stable() {
    let line = super::AidecisionGotoEnter {
        frame: &100u32,
        owner: &5u32,
        co: &Some(12u32),
        destination_x_bits: &1.5f32.to_bits(),
        destination_y_bits: &(-1i32),
        destination_sector: &Some(3u16),
        destination_level: &1u8,
        position_x_bits: &0u32,
        position_y_bits: &0xabcu32,
        position_sector: &None::<u16>,
        position_level: &0u8,
        couldnt_before: &false,
        already_before: &true,
        owner_work_before: &vec!["walk"],
        flags: &0b101u8,
    }
    .to_string();
    assert_eq!(
        line,
        r#"AIDECISION frame=100 owner=5 co=Some(12) stage=goto_enter destination=(3fc00000,ffffffff,sector=Some(3),level=1) flags=5 position=(00000000,00000abc,sector=None,level=0) couldnt_before=false already_before=true owner_work_before=["walk"]"#
    );
}
