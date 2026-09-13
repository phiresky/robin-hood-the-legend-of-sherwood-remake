//! Golden byte-level captures of parity payload lines. Replay tooling parses
//! these; any change to the expected strings is a protocol change.

use std::cell::RefCell;

thread_local! {
    static CAPTURED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Returns `true` when a test capture on this thread consumed the line.
pub(crate) fn capture_line(line: std::fmt::Arguments<'_>) -> bool {
    CAPTURED.with(|captured| match captured.borrow_mut().as_mut() {
        Some(lines) => {
            lines.push(line.to_string());
            true
        }
        None => false,
    })
}

/// Run `emit` and return every payload line it wrote (without the newline).
pub(crate) fn capture(emit: impl FnOnce()) -> Vec<String> {
    CAPTURED.with(|captured| {
        let previous = captured.borrow_mut().replace(Vec::new());
        assert!(previous.is_none(), "nested parity trace capture");
    });
    emit();
    CAPTURED.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("parity trace capture was installed")
    })
}

#[test]
fn forecast_line_is_byte_stable() {
    let lines = capture(|| {
        super::forecast(
            &(-1.5f32),
            &2.0f32,
            &7u16,
            &3usize,
            &("door", Some(-4i32)),
            &0u8,
            &15u16,
            &None::<u32>,
        )
    });
    assert_eq!(
        lines,
        [
            r#"FORECAST input=("door", Some(-4)) out=(-1.5, 2, sector=7, layer=0) dir=15 gates=3 entry=None"#
        ]
    );
}

#[test]
fn considerreport_merge_start_line_is_byte_stable() {
    let lines = capture(|| {
        super::considerreport_merge_start(
            &42u32,
            &vec![(1u32, "a\"b\\c\n")],
            &Vec::<u32>::new(),
            &9u32,
            &0x10u32,
        )
    });
    assert_eq!(
        lines,
        [
            r#"CONSIDERREPORT {"stage":"merge_start","frame":9,"owner":42,"flags":16,"incoming":[(1, "a\"b\\c\n")],"known_before":[]}"#
        ]
    );
}

#[test]
fn aidecision_goto_enter_line_is_byte_stable() {
    let lines = capture(|| {
        super::aidecision_goto_enter(
            &100u32,
            &5u32,
            &Some(12u32),
            &1.5f32.to_bits(),
            &(-1i32),
            &Some(3u16),
            &1u8,
            &0u32,
            &0xabcu32,
            &None::<u16>,
            &0u8,
            &false,
            &true,
            &vec!["walk"],
            &0b101u8,
        )
    });
    assert_eq!(
        lines,
        [
            r#"AIDECISION frame=100 owner=5 co=Some(12) stage=goto_enter destination=(3fc00000,ffffffff,sector=Some(3),level=1) flags=5 position=(00000000,00000abc,sector=None,level=0) couldnt_before=false already_before=true owner_work_before=["walk"]"#
        ]
    );
}
