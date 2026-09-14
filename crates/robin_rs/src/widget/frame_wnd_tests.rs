use super::*;
use crate::ui::{MouseButtons, UiMsg};
use crate::widget::{WidgetButton, WidgetRadioButton};

#[test]
fn moving_widget_uses_live_bounds_for_pixel_hits() {
    let mut button = WidgetButton::new(1);
    button
        .base
        .create_with_resource("Move", ScreenBBox::from_coords(0.0, 0.0, 2.0, 2.0), 0, 99);
    button.base.appearance.as_mut().unwrap().alpha_mask =
        Some(crate::ui::AlphaMask::from_pixels(2, 2, 2, &[0, 1, 0, 0], 0));
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(10.0, 20.0, 30.0, 40.0), 0);
    frame.add_widget(Widget::Button(button));
    let base = frame.widget_mut(1).unwrap().base_mut();
    assert!(base.is_inside(ScreenPoint::new(11.0, 20.0)));
    assert!(!base.is_inside(ScreenPoint::new(10.0, 20.0)));
    base.set_position(ScreenBBox::from_coords(30.0, 40.0, 32.0, 42.0));
    assert!(!base.is_inside(ScreenPoint::new(11.0, 20.0)));
    assert!(base.is_inside(ScreenPoint::new(31.0, 40.0)));
    assert!(!base.is_inside(ScreenPoint::new(30.0, 40.0)));
    assert!(!base.is_inside(ScreenPoint::new(32.0, 40.0)));
}

use crate::widget::test_support::mouse_input as make_input;

fn make_button_widget(id: WidgetId, x: f32, y: f32, w: f32, h: f32) -> Widget {
    let mut btn = WidgetButton::new(id);
    let bbox = ScreenBBox::from_coords(x, y, x + w, y + h);
    btn.base.create("Test", bbox, 0);
    btn.base.appearance = Some(crate::ui::WidgetAppearance::default());
    Widget::Button(btn)
}

#[test]
fn add_widget_adjusts_position() {
    let mut frame = FrameWnd::new(
        "Test",
        ScreenBBox::from_coords(100.0, 50.0, 400.0, 300.0),
        0,
    );
    // Widget at (10, 10) relative to frame.
    let mut btn = WidgetButton::new(1);
    btn.base
        .create("Btn", ScreenBBox::from_coords(10.0, 10.0, 80.0, 30.0), 0);
    frame.add_widget(Widget::Button(btn));

    let widget_bbox = frame.widget(1).unwrap().base().bbox;
    // Should be adjusted by frame origin (100, 50).
    let rect = widget_bbox.0.unwrap();
    assert!((rect.min().x - 110.0).abs() < 0.01);
    assert!((rect.min().y - 60.0).abs() < 0.01);
}

#[test]
fn process_input_routes_to_widgets() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

    // Hover over button.
    let input = make_input(50.0, 20.0, MouseButtons::empty());
    let events = frame.process_input(&input);
    assert!(events.iter().any(|e| e.msg_type == UiMsg::FrameFocus));
    assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
}

#[test]
fn excluded_widget_skipped() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    frame.exclude_widget(1);

    let input = make_input(50.0, 20.0, MouseButtons::empty());
    let events = frame.process_input(&input);
    // Only FrameFocus, no widget events.
    assert!(!events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
}

#[test]
fn remove_widget_works() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    assert_eq!(frame.widget_count(), 1);

    let removed = frame.remove_widget(1);
    assert!(removed.is_some());
    assert_eq!(frame.widget_count(), 0);
}

#[test]
fn disabled_frame_returns_no_events() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    frame.set_enable(false);

    let input = make_input(50.0, 20.0, MouseButtons::LEFT_CLICK);
    let events = frame.process_input(&input);
    assert!(events.is_empty());
}

#[test]
fn exclude_widget_requires_membership() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

    // Unknown widget id must not be excluded.
    assert!(!frame.exclude_widget(999));
    assert!(!frame.is_excluded(999));

    // First exclusion of a known widget succeeds.
    assert!(frame.exclude_widget(1));
    assert!(frame.is_excluded(1));

    // Duplicate exclusion is a no-op.
    assert!(!frame.exclude_widget(1));
}

#[test]
fn remove_widget_leaves_exclusion_list() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    assert!(frame.exclude_widget(1));
    assert!(frame.is_excluded(1));

    // remove_widget intentionally leaves the exclusion list untouched.
    let removed = frame.remove_widget(1);
    assert!(removed.is_some());
    assert!(
        frame.is_excluded(1),
        "remove_widget must not prune the exclusion list",
    );
}

#[test]
fn clear_widgets_empties_tree() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    frame.add_widget_absolute(make_button_widget(2, 10.0, 50.0, 80.0, 30.0));
    assert_eq!(frame.widget_count(), 2);

    frame.clear_widgets();
    assert_eq!(frame.widget_count(), 0);
}

#[test]
fn has_tooltip_reflects_set_call_not_text() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    assert!(!frame.has_tooltip());

    // setting empty text still flags the tooltip present.
    frame.set_tooltip_text("");
    assert!(frame.has_tooltip());

    frame.set_tooltip_text("hello");
    assert!(frame.has_tooltip());
    assert_eq!(frame.tooltip_text, "hello");
}

#[test]
fn add_widget_adjusts_even_without_frame_bbox() {
    // Frame with no bbox — origin defaults to (0, 0); widget position
    // should stay unchanged.
    let mut frame = FrameWnd::new("Test", ScreenBBox::new(), 0);
    let mut btn = WidgetButton::new(1);
    btn.base
        .create("Btn", ScreenBBox::from_coords(10.0, 10.0, 80.0, 30.0), 0);
    frame.add_widget(Widget::Button(btn));

    let rect = frame.widget(1).unwrap().base().bbox.0.unwrap();
    assert!((rect.min().x - 10.0).abs() < 0.01);
    assert!((rect.min().y - 10.0).abs() < 0.01);
}

fn make_radio_widget(id: WidgetId, x: f32, y: f32, w: f32, h: f32) -> WidgetRadioButton {
    let mut rb = WidgetRadioButton::new(id);
    let bbox = ScreenBBox::from_coords(x, y, x + w, y + h);
    rb.base.create("Radio", bbox, 0);
    rb.base.appearance = Some(crate::ui::WidgetAppearance::default());
    rb
}

#[test]
fn radio_group_exclusion_deselects_siblings() {
    // Three radio buttons linked as a group — clicking one must
    // deselect the others.
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 400.0, 400.0), 0);
    let mut rb0 = make_radio_widget(10, 10.0, 10.0, 80.0, 20.0);
    let mut rb1 = make_radio_widget(11, 10.0, 40.0, 80.0, 20.0);
    let mut rb2 = make_radio_widget(12, 10.0, 70.0, 80.0, 20.0);
    rb0.group_members = vec![10, 11, 12];
    rb1.group_members = vec![10, 11, 12];
    rb2.group_members = vec![10, 11, 12];
    // Pre-select rb0 so we can confirm it gets kicked.
    rb0.set_selected(true);
    frame.add_widget_absolute(Widget::RadioButton(rb0));
    frame.add_widget_absolute(Widget::RadioButton(rb1));
    frame.add_widget_absolute(Widget::RadioButton(rb2));

    // Click inside rb1 (center of its bbox).
    let input = make_input(50.0, 50.0, MouseButtons::LEFT_CLICK);
    let events = frame.process_input(&input);
    assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));

    let get_second_state = |f: &FrameWnd, id: WidgetId| -> bool {
        match f.widget(id).unwrap() {
            Widget::RadioButton(rb) => rb.is_pushed(),
            _ => panic!("expected radio button"),
        }
    };
    assert!(
        !get_second_state(&frame, 10),
        "rb0 must be deselected after rb1 activation"
    );
    assert!(get_second_state(&frame, 11), "rb1 must stay selected");
    assert!(!get_second_state(&frame, 12), "rb2 must remain deselected");
}

#[test]
fn radio_group_activation_at_each_position_preserves_nonmembers() {
    for active in 0..3 {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 400.0, 400.0), 0);
        for index in 0..4 {
            let mut radio =
                make_radio_widget(10 + index, 10.0, 10.0 + index as f32 * 30.0, 80.0, 20.0);
            if index < 3 {
                // Repeated and missing IDs retain their existing no-op behavior.
                radio.group_members = vec![12, 10, 11, 10, 999];
            }
            radio.set_selected(index != active);
            frame.add_widget_absolute(Widget::RadioButton(radio));
        }
        let input = make_input(50.0, 20.0 + active as f32 * 30.0, MouseButtons::LEFT_CLICK);
        let events = frame.process_input(&input);
        assert!(
            events
                .iter()
                .any(|event| event.msg_type == UiMsg::WidgetActivated
                    && event.origin_widget_id == 10 + active)
        );
        for index in 0..4 {
            let Widget::RadioButton(radio) = frame.widget(10 + index).unwrap() else {
                panic!("expected radio button");
            };
            assert_eq!(radio.is_pushed(), index == active || index == 3);
            if index < 3 {
                assert_eq!(radio.group_members, [12, 10, 11, 10, 999]);
            }
        }
    }
}

#[test]
fn radio_activation_without_group_does_not_touch_others() {
    // Radio buttons with empty group_members must not interfere with
    // each other — matches the slider sub-button case where exclusion
    // is managed by the slider, not the frame.
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 400.0, 400.0), 0);
    let rb0 = make_radio_widget(10, 10.0, 10.0, 80.0, 20.0);
    let mut rb1 = make_radio_widget(11, 10.0, 40.0, 80.0, 20.0);
    rb1.set_selected(true);
    frame.add_widget_absolute(Widget::RadioButton(rb0));
    frame.add_widget_absolute(Widget::RadioButton(rb1));

    // Click rb0 to emit Activated with no group_members wired — rb1
    // must stay selected because nothing walks the chain.
    let input = make_input(50.0, 20.0, MouseButtons::LEFT_CLICK);
    let events = frame.process_input(&input);
    assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));

    let is_pushed = |f: &FrameWnd, id: WidgetId| -> bool {
        match f.widget(id).unwrap() {
            Widget::RadioButton(rb) => rb.is_pushed(),
            _ => panic!(),
        }
    };
    assert!(
        is_pushed(&frame, 10),
        "rb0 must be selected after being clicked",
    );
    assert!(
        is_pushed(&frame, 11),
        "rb1 must remain selected when rb0 has no group_members",
    );
}

#[test]
fn keyboard_navigation_uses_widget_ids_and_skips_disabled_entries() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    for id in [91, 4, 700] {
        frame.add_widget_absolute(make_button_widget(id, 0.0, 0.0, 20.0, 20.0));
    }
    frame.widget_mut(4).unwrap().base_mut().enabled = false;
    assert_eq!(frame.next_enabled_widget(91, true), Some(700));
    assert_eq!(frame.next_enabled_widget(700, true), Some(91));
    assert_eq!(frame.next_enabled_widget(91, false), Some(700));
    assert_eq!(frame.next_enabled_widget(700, false), Some(91));
    assert_eq!(frame.next_enabled_widget(4, true), Some(700));
    assert_eq!(frame.next_enabled_widget(4, false), Some(91));
    // A removed selection starts at the appropriate end, not at an unrelated ID/index.
    assert_eq!(frame.next_enabled_widget(999, true), Some(91));
    assert_eq!(frame.next_enabled_widget(999, false), Some(700));
}

#[test]
fn keyboard_navigation_handles_empty_disabled_and_single_widget_frames() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    for forward in [false, true] {
        assert_eq!(frame.next_enabled_widget(42, forward), None);
    }
    frame.add_widget_absolute(make_button_widget(42, 0.0, 0.0, 20.0, 20.0));
    for forward in [false, true] {
        assert_eq!(frame.next_enabled_widget(42, forward), Some(42));
    }
    frame.widget_mut(42).unwrap().base_mut().enabled = false;
    for forward in [false, true] {
        assert_eq!(frame.next_enabled_widget(42, forward), None);
        assert_eq!(frame.next_enabled_widget(999, forward), None);
    }
}
