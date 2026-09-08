use super::*;

/// Test callback recorder — captures all calls for assertions.
#[derive(Debug, Default)]
struct MockCallbacks {
    added: Vec<PortraitId>,
    removed: Vec<PortraitId>,
    minimap_fronted: usize,
    placements: Vec<(PortraitId, usize)>,
    enabled: Vec<(PortraitId, bool)>,
}

impl MockCallbacks {
    fn clear(&mut self) {
        self.added.clear();
        self.removed.clear();
        self.minimap_fronted = 0;
        self.placements.clear();
        self.enabled.clear();
    }
}

impl PortraitBarCallbacks for MockCallbacks {
    fn add_window(&mut self, id: PortraitId) {
        self.added.push(id);
    }
    fn remove_window(&mut self, id: PortraitId) {
        self.removed.push(id);
    }
    fn move_minimap_to_front(&mut self) {
        self.minimap_fronted += 1;
    }
    fn displace_portrait(&mut self, id: PortraitId, slot: usize) {
        self.placements.push((id, slot));
    }
    fn set_portrait_enabled(&mut self, id: PortraitId, enabled: bool) {
        self.enabled.push((id, enabled));
    }
}

fn make_portrait(id: u32, priority: u16) -> PortraitInfo {
    PortraitInfo {
        id: PortraitId(id),
        pc_id: PcId(id),
        pc_description_id: PcDescriptionId(id),
        priority,
        enabled: true,
        displayed: false,
        is_open: false,
        is_burned: false,
        is_trumpet_enabled: false,
        pc_is_playable: true,
        pc_is_in_coma: false,
        pc_is_guarded: false,
        pc_is_waiting_for_reinforcement: false,
        pc_is_selectable: true,
    }
}

#[test]
fn add_and_count() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    assert_eq!(bar.portrait_count(), 2);
    assert_eq!(cb.added.len(), 2);
}

#[test]
fn sorted_by_priority_descending() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 3));
    bar.add_portrait(make_portrait(2, 10));
    bar.add_portrait(make_portrait(3, 7));
    bar.update(&mut cb);

    let priorities: Vec<u16> = bar.portraits().iter().map(|p| p.priority).collect();
    assert_eq!(priorities, vec![10, 7, 3]);
}

#[test]
fn remove_portrait() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);
    assert_eq!(bar.portrait_count(), 2);

    cb.clear();
    bar.remove_portrait(PortraitId(1));
    bar.update(&mut cb);

    assert_eq!(bar.portrait_count(), 1);
    assert_eq!(cb.removed, vec![PortraitId(1)]);
    assert_eq!(bar.portraits()[0].id, PortraitId(2));
}

#[test]
fn remove_all_portraits() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.add_portrait(make_portrait(3, 7));
    bar.update(&mut cb);

    cb.clear();
    bar.remove_all_portraits();
    bar.update(&mut cb);

    assert_eq!(bar.portrait_count(), 0);
    assert_eq!(cb.removed.len(), 3);
}

#[test]
fn find_portrait_by_pc() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    assert!(bar.find_portrait_by_pc(PcId(1)).is_some());
    assert!(bar.find_portrait_by_pc(PcId(99)).is_none());
}

#[test]
fn find_portrait_by_description() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.update(&mut cb);

    assert!(
        bar.find_portrait_by_description(PcDescriptionId(1))
            .is_some()
    );
    assert!(
        bar.find_portrait_by_description(PcDescriptionId(99))
            .is_none()
    );
}

#[test]
fn portrait_index() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    assert_eq!(bar.portrait_index(PortraitId(1)), Some(0));
    assert_eq!(bar.portrait_index(PortraitId(2)), Some(1));
    assert_eq!(bar.portrait_index(PortraitId(99)), None);
}

#[test]
fn set_all_portraits_opened() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    bar.set_all_portraits_opened(true);
    assert!(bar.portraits().iter().all(|p| p.is_open));

    bar.set_all_portraits_opened(false);
    assert!(bar.portraits().iter().all(|p| !p.is_open));
}

#[test]
fn burned_portrait_not_opened() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    let mut p = make_portrait(1, 10);
    p.is_burned = true;
    bar.add_portrait(p);
    bar.update(&mut cb);

    bar.set_all_portraits_opened(true);
    assert!(!bar.portraits()[0].is_open);
}

#[test]
fn set_portrait_opened_specific() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    bar.set_portrait_opened(PcId(1), true);
    assert!(bar.portraits()[0].is_open);
    assert!(!bar.portraits()[1].is_open);

    bar.set_portrait_opened(PcId(1), false);
    assert!(!bar.portraits()[0].is_open);
}

#[test]
#[should_panic(expected = "Cannot open a burned portrait")]
fn open_burned_portrait_panics() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    let mut p = make_portrait(1, 10);
    p.is_burned = true;
    bar.add_portrait(p);
    bar.update(&mut cb);

    bar.set_portrait_opened(PcId(1), true);
}

#[test]
fn placement_all_fit() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    for i in 0..3 {
        bar.add_portrait(make_portrait(i, 10 - i as u16));
    }
    bar.update(&mut cb);

    // All 3 should be placed in slots 0..2 and enabled.
    assert_eq!(cb.placements.len(), 3);
    assert!(cb.enabled.iter().all(|&(_, e)| e));
}

#[test]
fn scrolling_when_more_than_max() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    // Add 7 portraits (more than MAX_VISIBLE_PORTRAITS = 5).
    for i in 0..7 {
        bar.add_portrait(make_portrait(i, 10 - i as u16));
    }
    bar.update(&mut cb);

    // 5 enabled, 2 disabled.
    let enabled_count = cb.enabled.iter().filter(|&&(_, e)| e).count();
    let disabled_count = cb.enabled.iter().filter(|&&(_, e)| !e).count();
    assert_eq!(enabled_count, 5);
    assert_eq!(disabled_count, 2);
}

#[test]
fn shift_left_and_right() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    for i in 0..7 {
        bar.add_portrait(make_portrait(i, 10 - i as u16));
    }
    bar.update(&mut cb);

    // Shift right, then update to apply placement.
    cb.clear();
    bar.shift_portraits_to_right();
    bar.update(&mut cb);
    assert!(!cb.placements.is_empty());

    // Shift left reverses the shift.
    cb.clear();
    bar.shift_portraits_to_left();
    bar.update(&mut cb);
    assert!(!cb.placements.is_empty());
}

#[test]
fn shift_ignored_when_all_fit() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.update(&mut cb);

    cb.clear();
    bar.shift_portraits_to_right();
    bar.update(&mut cb);

    // No placement callbacks because shift is a no-op when <= MAX.
    assert!(cb.placements.is_empty());
}

#[test]
fn reset_clears_everything() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    bar.reset(&mut cb);

    assert_eq!(bar.portrait_count(), 0);
}

#[test]
fn add_then_cancel_with_remove_before_update() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.remove_portrait(PortraitId(1));
    bar.update(&mut cb);

    // Should not have been added at all.
    assert_eq!(bar.portrait_count(), 0);
    assert!(cb.added.is_empty());
}

#[test]
fn remove_then_cancel_with_add_before_update() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.update(&mut cb);

    cb.clear();
    bar.remove_portrait(PortraitId(1));
    bar.add_portrait(make_portrait(1, 10));
    bar.update(&mut cb);

    // Removal should have been cancelled — portrait still present.
    assert_eq!(bar.portrait_count(), 1);
}

#[test]
fn duplicate_add_ignored() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.update(&mut cb);

    cb.clear();
    let mut p = make_portrait(1, 10);
    p.displayed = true; // already displayed
    bar.add_portrait(p);
    bar.update(&mut cb);

    assert_eq!(bar.portrait_count(), 1);
}

#[test]
fn resize_displaces_visible() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    for i in 0..3 {
        bar.add_portrait(make_portrait(i, 10 - i as u16));
    }
    bar.update(&mut cb);

    cb.clear();
    bar.resize(&mut cb);

    assert_eq!(cb.placements.len(), 3);
}

#[test]
fn find_in_pending_adds() {
    let mut bar = PortraitBar::new();

    bar.add_portrait(make_portrait(1, 10));
    // Not yet updated — should still be findable.
    assert!(bar.find_portrait_by_pc(PcId(1)).is_some());
}

#[test]
#[should_panic(expected = "PC is in invalid state")]
fn add_invalid_portrait_panics() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    let mut p = make_portrait(1, 10);
    p.pc_is_playable = false;
    // All conditions false — should panic during update.
    bar.add_portrait(p);
    bar.update(&mut cb);
}

#[test]
fn serde_roundtrip() {
    let mut bar = PortraitBar::new();
    let mut cb = MockCallbacks::default();

    bar.add_portrait(make_portrait(1, 10));
    bar.add_portrait(make_portrait(2, 5));
    bar.update(&mut cb);

    let json = serde_json::to_string(&bar).expect("serialize");
    let bar2: PortraitBar = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(bar2.portrait_count(), 2);
    assert_eq!(bar2.portraits()[0].priority, 10);
    assert_eq!(bar2.portraits()[1].priority, 5);
}
