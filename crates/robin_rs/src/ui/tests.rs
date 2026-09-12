use super::*;

// ── UiKeyboard tests ──

fn make_keys(pressed: &[KeyCode]) -> KeyboardState {
    let mut ks = KeyboardState::default();
    for &key in pressed {
        ks.keys.insert(key);
    }
    ks
}

#[test]
fn keyboard_first_refresh_initializes() {
    let mut kb = UiKeyboard::default();
    let ks = make_keys(&[KeyCode::KeyA]);
    assert!(!kb.refresh(&ks, 0));
    // Not initialized until second call.
    assert!(!kb.has_changed());
}

#[test]
fn keyboard_key_down_detected() {
    let mut kb = UiKeyboard::default();
    kb.refresh(&make_keys(&[]), 0);

    kb.refresh(&make_keys(&[KeyCode::Backspace]), 100);
    assert!(kb.has_changed());
    assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyDown);
}

#[test]
fn keyboard_key_pressed_on_release() {
    let mut kb = UiKeyboard::default();
    kb.refresh(&make_keys(&[]), 0);

    // Press key
    kb.refresh(&make_keys(&[KeyCode::Backspace]), 100);
    assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyDown);

    // Release key → KeyPressed
    kb.refresh(&make_keys(&[]), 700);
    assert_eq!(
        kb.get_state_of_key(KeyCode::Backspace),
        KeyState::KeyPressed
    );

    // Next frame → KeyUp (transient state cleaned up)
    kb.refresh(&make_keys(&[]), 800);
    assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyUp);
}

#[test]
fn keyboard_double_press_within_delay() {
    let mut kb = UiKeyboard::new(500); // 500ms double-press window
    // Use timestamps well past 0 so the initial last_key_press (0) is
    // outside the double-press window — matches a real monotonic clock.
    kb.refresh(&make_keys(&[]), 10_000);

    // First press + release
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_100);
    kb.refresh(&make_keys(&[]), 10_200);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyPressed);

    // Consume the pressed state
    kb.refresh(&make_keys(&[]), 10_250);

    // Second press + release within 500ms of first release
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_300);
    kb.refresh(&make_keys(&[]), 10_400);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyDouble);
}

#[test]
fn keyboard_no_double_press_outside_delay() {
    let mut kb = UiKeyboard::new(500);
    kb.refresh(&make_keys(&[]), 10_000);

    // First press + release
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_100);
    kb.refresh(&make_keys(&[]), 10_200);
    kb.refresh(&make_keys(&[]), 10_250);

    // Second press + release AFTER 500ms from first release
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_800);
    kb.refresh(&make_keys(&[]), 10_900);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyPressed);
}

#[test]
fn keyboard_typewriter_repeat() {
    let mut kb = UiKeyboard::default();
    kb.refresh(&make_keys(&[]), 0);

    // Press key
    kb.refresh(&make_keys(&[KeyCode::KeyB]), 100);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::None);

    // Hold — transitions to Touch
    kb.refresh(&make_keys(&[KeyCode::KeyB]), 200);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Touch);

    // Hold past REPEAT_FIRST (400ms) → Repeat
    kb.refresh(&make_keys(&[KeyCode::KeyB]), 550);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Repeat);

    // Next frame → Waiting
    kb.refresh(&make_keys(&[KeyCode::KeyB]), 560);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Waiting);

    // Wait past REPEAT_AFTER (50ms) → Repeat again
    kb.refresh(&make_keys(&[KeyCode::KeyB]), 620);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Repeat);
}

#[test]
fn keyboard_has_key_changed() {
    let mut kb = UiKeyboard::default();
    kb.refresh(&make_keys(&[]), 0);

    kb.refresh(&make_keys(&[KeyCode::Digit5]), 100);
    assert!(kb.has_key_changed(KeyCode::Digit5));
    assert!(!kb.has_key_changed(KeyCode::Digit6));
}

#[test]
fn keyboard_reset() {
    let mut kb = UiKeyboard::default();
    kb.refresh(&make_keys(&[]), 0);
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 100);
    kb.reset();
    assert!(kb.has_changed()); // reset sets changed = true
}

#[test]
fn keyboard_updates_tracked_and_raw_only_keys_once_per_frame() {
    let mut kb = UiKeyboard::default();
    // An initially held key exists only in the raw snapshot, not key_state.
    kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_000);
    kb.refresh(&make_keys(&[KeyCode::KeyA, KeyCode::KeyB]), 10_100);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyA), TypeWriter::Touch);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyB), KeyState::KeyDown);

    kb.refresh(&make_keys(&[KeyCode::KeyA, KeyCode::KeyB]), 10_200);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyA), TypeWriter::Repeat);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Touch);

    // A is raw-only; B is in both maps and both raw sets; C is newly observed.
    kb.refresh(
        &make_keys(&[KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC]),
        10_600,
    );
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyA), TypeWriter::Waiting);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Repeat);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyC), KeyState::KeyDown);

    kb.refresh(&make_keys(&[KeyCode::KeyC]), 10_700);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyB), KeyState::KeyPressed);
    assert!(kb.has_key_changed(KeyCode::KeyB));
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyC), TypeWriter::Touch);

    // Released tracked keys must still be visited after leaving both raw sets.
    kb.refresh(&make_keys(&[KeyCode::KeyC]), 10_800);
    assert_eq!(kb.get_state_of_key(KeyCode::KeyB), KeyState::KeyUp);
    assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::None);
    assert!(kb.has_key_changed(KeyCode::KeyB));
}

#[test]
fn renderer_base_defaults() {
    let r = RendererBase::default();
    assert_eq!(r.resource_id, -1);
    assert_eq!(r.last_rendered, [u32::MAX; 2]);
    assert!(r.alpha_mask.is_none());
}

#[test]
fn renderer_base_is_real_point_bbox_only() {
    let mut r = RendererBase::default();
    r.set_position_bbox(ScreenBBox::from_coords(10.0, 10.0, 30.0, 30.0));
    assert!(r.is_real_point(ScreenPoint::new(15.0, 15.0)));
    assert!(!r.is_real_point(ScreenPoint::new(5.0, 5.0)));
    // Without a mask, every in-bbox pixel is opaque.
    assert!(r.is_real_point(ScreenPoint::new(10.0, 10.0)));
}

#[test]
fn renderer_base_is_real_point_with_mask() {
    // 4x4 surface, color-key = 0x07C0; pixel (1,1) is opaque,
    // everything else is keyed transparent.
    const KEY: u16 = 0x07C0;
    let mut pixels = vec![KEY; 16];
    pixels[5] = 0x1234;
    let mask = AlphaMask::from_pixels(4, 4, 4, &pixels, KEY);

    let mut r = RendererBase::default();
    r.set_position_bbox(ScreenBBox::from_coords(10.0, 20.0, 14.0, 24.0));
    r.set_alpha_mask(Some(mask));

    // bbox top-left = (10, 20); only local (1, 1) is opaque.
    assert!(r.is_real_point(ScreenPoint::new(11.0, 21.0)));
    assert!(!r.is_real_point(ScreenPoint::new(10.0, 20.0)));
    assert!(!r.is_real_point(ScreenPoint::new(13.0, 23.0)));
    // Outside the bbox: rejected before the mask check.
    assert!(!r.is_real_point(ScreenPoint::new(50.0, 50.0)));
}

#[test]
fn renderer_listbox_defaults() {
    let lb = RendererListbox::new();
    assert_eq!(lb.indent_size, 20);
    assert_eq!(lb.font_height, 0);
    assert_eq!(lb.knob_width, 0);
    assert_eq!(lb.scrollbar_track_width, 0);
    // Sentinels: "surface not yet created".
    assert_eq!(lb.surface_knob, u32::MAX);
    assert_eq!(lb.surface_scrollbar, u32::MAX);
}

#[test]
fn renderer_listbox_displayable_items() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
    lb.set_font_height(20);
    assert_eq!(lb.displayable_item_count(), 5); // 100 / 20

    lb.set_font_height(0);
    assert_eq!(lb.displayable_item_count(), 0); // guard against zero
}

#[test]
fn renderer_listbox_knob_params() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
    lb.set_font_height(20);
    lb.set_scrollbar_track_width(16);

    // 50 items, starting at index 10
    lb.set_knob_parameters(10, 50);
    assert_eq!(lb.number_of_items, 50);
    // knob_ratio = 5/50 = 0.1, before_ratio = 10/50 = 0.2
    assert!((lb.knob_ratio - 0.1).abs() < 1e-6);
    assert!((lb.before_ratio - 0.2).abs() < 1e-6);
}

#[test]
fn renderer_listbox_knob_params_few_items() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
    lb.set_font_height(20);
    // Only 3 items, but can display 5 → full knob
    lb.set_knob_parameters(0, 3);
    assert!((lb.knob_ratio - 1.0).abs() < 1e-6);
    assert!((lb.before_ratio - 0.0).abs() < 1e-6);
}

#[test]
fn renderer_listbox_text_box() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(10.0, 20.0, 210.0, 120.0);
    lb.set_font_height(15);
    lb.set_scrollbar_track_width(16);

    // Item 0, no flags
    let b0 = lb.text_box_for_item(0, 0);
    let r0 = b0.0.unwrap();
    assert!((r0.min().x - 10.0).abs() < 1e-6); // bbox left
    assert!((r0.min().y - 20.0).abs() < 1e-6); // bbox top
    assert!((r0.max().x - (10.0 + 200.0 - 16.0)).abs() < 1e-6); // bbox left + width - scrollbar
    assert!((r0.max().y - 35.0).abs() < 1e-6); // bbox top + fontHeight

    // Item 2, with indent
    let b2 = lb.text_box_for_item(2, listbox_flags::INDENT);
    let r2 = b2.0.unwrap();
    assert!((r2.min().x - (10.0 + 20.0)).abs() < 1e-6); // indented by 20
    assert!((r2.min().y - (20.0 + 30.0)).abs() < 1e-6); // 2 * 15 offset
}

#[test]
fn renderer_listbox_scrollbar_bbox() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
    lb.set_scrollbar_track_width(16);
    lb.surface_scrollbar = 0; // not 0xFFFFFFFF

    let sb = lb.scrollbar_bbox();
    let r = sb.0.unwrap();
    assert!((r.min().x - 184.0).abs() < 1e-6); // 200 - 16
    assert!((r.min().y - 0.0).abs() < 1e-6);
    assert!((r.max().x - 200.0).abs() < 1e-6);
    assert!((r.max().y - 100.0).abs() < 1e-6);
}

#[test]
fn renderer_listbox_scrollbar_uninitialized() {
    let lb = RendererListbox::new();
    let sb = lb.scrollbar_bbox();
    // Returns degenerate `(0,0,0,0)` here, not "no box".
    let r = sb.0.expect("expected degenerate (0,0,0,0) box");
    assert_eq!(r.min().x, 0.0);
    assert_eq!(r.min().y, 0.0);
    assert_eq!(r.max().x, 0.0);
    assert_eq!(r.max().y, 0.0);
}

#[test]
fn renderer_listbox_knob_bbox() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
    lb.set_knob_width(16);
    lb.before_ratio = 0.0;
    lb.knob_ratio = 0.5;

    let kb = lb.knob_bbox();
    let r = kb.0.unwrap();
    // x: 200 - 1 - 16 = 183
    assert!((r.min().x - 183.0).abs() < 1e-6);
    // y_start: (100-2)*0.0 + 0 + 1 = 1
    assert!((r.min().y - 1.0).abs() < 1e-6);
    // y_end: (100-2)*0.5 + 0 + 1 = 50
    assert!((r.max().y - 50.0).abs() < 1e-6);
}

#[test]
fn renderer_listbox_knob_height_for_one_item() {
    let mut lb = RendererListbox::new();
    lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 102.0);
    lb.number_of_items = 10;
    // (102 - 2) / 10 = 10
    assert_eq!(lb.knob_height_for_one_item(), 10);
}

// ── Serde roundtrip tests ──

#[test]
fn serde_ui_msg_roundtrip() {
    let msg = UiMsg::WidgetDoubleClicked;
    let json = serde_json::to_string(&msg).unwrap();
    let back: UiMsg = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn serde_ui_event_roundtrip() {
    let ev = UiEvent {
        msg_type: UiMsg::WidgetActivated,
        origin_widget_id: 42,
        data: Some(UiEventData::SliderPosition(0.75)),
    };
    let json = serde_json::to_string(&ev).unwrap();
    let back: UiEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back.msg_type, UiMsg::WidgetActivated);
    assert_eq!(back.origin_widget_id, 42);
}
