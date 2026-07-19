use super::*;
use crate::ui::{MouseButtons, UiKeyboard, UiMsg, resource_widget_id::NO_RESOURCE};
use crate::widget::{WidgetButton, WidgetRadioButton, WidgetRenderer};

fn make_keyboard() -> &'static UiKeyboard {
    Box::leak(Box::new(UiKeyboard::default()))
}

fn make_input(x: f32, y: f32, buttons: MouseButtons) -> WidgetInput<'static> {
    WidgetInput {
        mouse_position: engine_coordinates::ScreenPoint::new(x, y),
        mouse_z: 0,
        mouse_button: buttons,
        keyboard: make_keyboard(),
        text_input: "",
        capture: None,
    }
}

fn make_button_widget(id: WidgetId, x: f32, y: f32, w: f32, h: f32) -> Widget {
    let mut btn = WidgetButton::new(id);
    let bbox = ScreenBBox::from_coords(x, y, x + w, y + h);
    btn.base.create("Test", bbox, 0);
    btn.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
        base: crate::ui::RendererBase {
            bbox,
            ..Default::default()
        },
    });
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
fn disabled_frame_refresh_skips_children() {
    use crate::ui::resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE};

    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .sub_resource,
        NO_RESOURCE,
    );

    frame.set_enable(false);
    frame.refresh();
    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .sub_resource,
        NO_RESOURCE,
        "refresh() on disabled frame must not render children",
    );

    frame.set_enable(true);
    frame.refresh();
    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .sub_resource,
        BUTTON_DEFAULT,
        "refresh() on enabled frame must render children",
    );
}

#[test]
fn disabled_frame_restore_and_probe_skip_children() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    // Dirty last_rendered so we can detect whether restore() cleared it.
    frame
        .widget_mut(1)
        .unwrap()
        .base_mut()
        .renderer
        .base_mut()
        .unwrap()
        .last_rendered = [42, 42];

    frame.set_enable(false);
    frame.restore();
    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .last_rendered,
        [42, 42],
        "restore() on disabled frame must not touch children",
    );

    let region = ScreenBBox::from_coords(0.0, 0.0, 100.0, 100.0);
    frame.restore_region(&region);
    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .last_rendered,
        [42, 42],
        "restore_region() on disabled frame must not touch children",
    );

    let probes = frame.probe_refresh(0);
    assert!(
        probes.is_empty(),
        "probe_refresh() on disabled frame must return no probes",
    );
}

#[test]
fn restore_region_calls_both_restore_and_refresh() {
    use crate::ui::resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE};

    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
    // Sentinel for restore() detection: reset_save will reset this to [MAX; 2].
    frame
        .widget_mut(1)
        .unwrap()
        .base_mut()
        .renderer
        .base_mut()
        .unwrap()
        .last_rendered = [42, 42];

    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .sub_resource,
        NO_RESOURCE,
    );

    let region = ScreenBBox::from_coords(0.0, 0.0, 100.0, 100.0);
    frame.restore_region(&region);

    let rbase = frame.widget(1).unwrap().base().renderer.base().unwrap();
    assert_eq!(
        rbase.last_rendered,
        [u32::MAX; 2],
        "restore() must have cleared last_rendered",
    );
    assert_eq!(
        rbase.sub_resource, BUTTON_DEFAULT,
        "refresh() must have run after restore() for intersecting widget",
    );
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
    rb.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
        base: crate::ui::RendererBase {
            bbox,
            ..Default::default()
        },
    });
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
fn restore_region_skips_non_intersecting() {
    let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
    frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

    // Region far outside the widget's bbox.
    let region = ScreenBBox::from_coords(150.0, 150.0, 200.0, 200.0);
    frame.restore_region(&region);

    assert_eq!(
        frame
            .widget(1)
            .unwrap()
            .base()
            .renderer
            .base()
            .unwrap()
            .sub_resource,
        NO_RESOURCE,
        "non-intersecting widget must not be refreshed",
    );
}
