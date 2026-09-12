//! Shared fixtures and unwind-safe, observational test instrumentation.
pub(crate) mod actors;
pub(crate) mod asm;

/// Supply explicit ordinary topology for fixtures that query only sector
/// metadata. No polygon is invented: geometric routing tests must install
/// their real geometry separately. Existing canonical sectors are preserved.
pub(crate) fn ensure_ordinary_sector(
    engine: &mut super::EngineInner,
    raw_sector: u16,
    layer: u16,
) -> crate::position_interface::SectorHandle {
    let number = crate::sector::SectorNumber::new(raw_sector as i16);
    let grid = engine.world.fast_grid_mut();
    if let Some(&index) = grid.level.sector_number_map.get(&number) {
        let sector = grid
            .level
            .sectors
            .get(index)
            .expect("fixture sector map must resolve");
        assert_eq!(sector.layer, layer, "fixture sector has a different layer");
    } else {
        grid.add_sector(
            crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer,
                sector_number: number,
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            },
            layer,
        );
    }
    crate::position_interface::SectorHandle::from_number(number)
}

use std::cell::RefCell;

/// A thread-local observation sink. Captures restore the previous scope even
/// during unwinding, so a panicking test cannot contaminate the next capture.
pub(crate) struct Probe<E>(RefCell<Option<Vec<E>>>);

impl<E> Probe<E> {
    pub(crate) const fn new() -> Self {
        Self(RefCell::new(None))
    }

    pub(crate) fn record(&self, event: E) {
        if let Some(events) = self.0.borrow_mut().as_mut() {
            events.push(event);
        }
    }

    pub(crate) fn capture<T>(&self, operation: impl FnOnce() -> T) -> (T, Vec<E>) {
        struct Restore<'a, E> {
            probe: &'a Probe<E>,
            previous: Option<Vec<E>>,
        }
        impl<E> Drop for Restore<'_, E> {
            fn drop(&mut self) {
                self.probe.0.replace(self.previous.take());
            }
        }
        let restore = Restore {
            probe: self,
            previous: self.0.replace(Some(Vec::new())),
        };
        let value = operation();
        let events = self.0.borrow_mut().take().expect("active capture");
        drop(restore);
        (value, events)
    }
}

#[test]
fn probe_scopes_are_nested_and_restore_after_panics() {
    let probe = Probe::new();
    probe.record(0);
    let (_, outer) = probe.capture(|| {
        probe.record(1);
        assert_eq!(probe.capture(|| probe.record(2)).1, [2]);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            probe.capture(|| {
                probe.record(3);
                panic!("fixture unwind");
            });
        }));
        assert!(panic.is_err());
        probe.record(4);
    });
    assert_eq!(outer, [1, 4]);
    assert_eq!(probe.capture(|| probe.record(5)).1, [5]);
}
