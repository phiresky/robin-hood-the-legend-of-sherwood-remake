//! Atomic adoption of the self-contained v48 selection and feedback sections.
//!
//! These sections sit around `RHSequenceManager` in `RHEngine::Serialize`.
//! Conversion resolves every Original pointer before mutating the initialized
//! mission. Host-owned minimap widget state is deliberately returned to the
//! caller rather than discarded.

use thiserror::Error;

use crate::{
    coordinates::{ScreenBBox, WorldPoint3D},
    element::{Entity, EntityId},
    engine::{EngineInner, HostDisplayState},
    markers::GroundMarkEntry,
    minimap::{HighlightedElement, MinimapV48State},
    titbit::{ElementHandle, TitbitInfo, TitbitKind},
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    body::LegacyUserLockState,
    post_simple::{
        LegacyElementSelection, LegacyFollowViewRefs, LegacyGroundMarkState, LegacyMinimapState,
        LegacyTitbitsState,
    },
};

#[derive(Debug, Error)]
pub enum LegacySimpleAdoptError {
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error("saved {field} entry resolves to non-PC entity {entity_id}")]
    SelectionIsNotPc {
        field: &'static str,
        entity_id: EntityId,
    },
    #[error("saved titbit {index} has unknown RHtitbitKind value {value}")]
    UnknownTitbitKind { index: usize, value: i32 },
    #[error("saved titbit {index} uses reserved id 0xffffffff")]
    InvalidTitbitId { index: usize },
    #[error("saved minimap highlight {index} has a null element reference")]
    NullMinimapHighlight { index: usize },
    #[error("saved minimap field {field} contains non-finite value {value}")]
    NonFiniteMinimap { field: &'static str, value: f32 },
}

/// Engine-owned state plus host-owned values which must be installed together
/// at the loaded-save boundary.
#[derive(Clone, Debug)]
pub struct LegacySimpleAdoptionPlan {
    selected: Vec<EntityId>,
    selected_before_lock: Vec<EntityId>,
    user_locked: bool,
    follow: Option<EntityId>,
    view: Option<EntityId>,
    ground_marks: Vec<GroundMarkEntry>,
    titbit_current_id: u32,
    titbits: Vec<TitbitInfo>,
    minimap: MinimapV48State,
}

impl LegacySimpleAdoptionPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn preflight(
        engine: &EngineInner,
        entities: &LegacyEntityFixups,
        user_lock: &LegacyUserLockState,
        selected: &LegacyElementSelection,
        selected_before_lock: &LegacyElementSelection,
        follow_view: &LegacyFollowViewRefs,
        ground_mark: &LegacyGroundMarkState,
        titbits: &LegacyTitbitsState,
        minimap: &LegacyMinimapState,
    ) -> Result<Self, LegacySimpleAdoptError> {
        let selected = resolve_pc_selection(engine, entities, selected, "selected_elements")?;
        let selected_before_lock = resolve_pc_selection(
            engine,
            entities,
            selected_before_lock,
            "selected_before_lock",
        )?;
        let follow = entities.resolve_element(follow_view.follow)?;
        let view = entities.resolve_element(follow_view.view)?;
        let minimap = convert_minimap(minimap, entities)?;

        // `RHGroundMark` serializes the stored sprite position directly. On
        // load there is no render pass yet, so both Rust frame views begin at
        // the saved frame and the next normal tick advances them together.
        let ground_marks = ground_mark
            .marks
            .iter()
            .map(|mark| GroundMarkEntry {
                x: mark.position.x,
                y: mark.position.y,
                layer: mark.current_level,
                current_frame: mark.current_sprite_frame,
                render_frame: mark.current_sprite_frame,
            })
            .collect();

        let mut converted_titbits = Vec::with_capacity(titbits.titbits.len());
        for (index, saved) in titbits.titbits.iter().enumerate() {
            let kind =
                titbit_kind(saved.kind).ok_or(LegacySimpleAdoptError::UnknownTitbitKind {
                    index,
                    value: saved.kind,
                })?;
            let supplier = resolve_titbit_handle(entities, saved.element_info_supplier)?;
            let manager = resolve_titbit_handle(entities, saved.element_manager)?;
            converted_titbits.push(TitbitInfo {
                kind,
                phase: saved.phase,
                sprite_row: saved.sprite_row,
                sprite_frame: saved.sprite_frame,
                frame_count: saved.frame_count,
                element_supplier: supplier,
                element_manager: manager,
                layer: saved.layer,
                position: WorldPoint3D::new(saved.position.x, saved.position.y, saved.position.z),
                // The Original writes this member twice and overwrites it
                // with the second value while loading.
                display_order: saved.display_order_effective,
                blinking: saved.blinking,
                id: crate::titbit::TitbitId::new(saved.id)
                    .ok_or(LegacySimpleAdoptError::InvalidTitbitId { index })?,
            });
        }

        Ok(Self {
            selected,
            selected_before_lock,
            user_locked: user_lock.locked,
            follow,
            view,
            ground_marks,
            titbit_current_id: titbits.current_id,
            titbits: converted_titbits,
            minimap,
        })
    }

    /// Apply the preflighted engine-owned fields and return host-owned state.
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacySimpleHostState {
        let seat = engine
            .players
            .seats
            .first_mut()
            .expect("initialized v48 mission has no host player seat");
        seat.selection = self.selected;
        seat.follow_element = self.follow;
        engine.players.user_locked = self.user_locked;
        engine.players.selection_before_user_lock = self.selected_before_lock;
        engine.feedback.ground_mark.marks = self.ground_marks;
        engine
            .feedback
            .titbit_manager
            .adopt_v48_serialized_state(self.titbit_current_id, self.titbits);
        LegacySimpleHostState {
            selected_view_element: self.view,
            minimap: self.minimap,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LegacySimpleHostState {
    pub selected_view_element: Option<EntityId>,
    pub minimap: MinimapV48State,
}

impl LegacySimpleHostState {
    pub fn apply_minimap_to(self, display: &mut HostDisplayState) -> Option<EntityId> {
        display.minimap.restore_v48_serialized_state(self.minimap);
        self.selected_view_element
    }
}

fn convert_minimap(
    saved: &LegacyMinimapState,
    entities: &LegacyEntityFixups,
) -> Result<MinimapV48State, LegacySimpleAdoptError> {
    finite_minimap("transition_counter", saved.transition_counter)?;
    for (field, value) in [
        ("memory_box.top_left.x", saved.memory_box.top_left.x),
        ("memory_box.top_left.y", saved.memory_box.top_left.y),
        ("memory_box.bottom_right.x", saved.memory_box.bottom_right.x),
        ("memory_box.bottom_right.y", saved.memory_box.bottom_right.y),
    ] {
        finite_minimap(field, value)?;
    }
    let memory_box = if saved.memory_box.bounds_are_set {
        ScreenBBox::from_coords(
            saved.memory_box.top_left.x,
            saved.memory_box.top_left.y,
            saved.memory_box.bottom_right.x,
            saved.memory_box.bottom_right.y,
        )
    } else {
        ScreenBBox::new()
    };
    let highlighted_elements = saved
        .highlighted_elements
        .iter()
        .enumerate()
        .map(|(index, highlight)| {
            let entity = entities
                .resolve_element(highlight.element)?
                .ok_or(LegacySimpleAdoptError::NullMinimapHighlight { index })?;
            Ok(HighlightedElement {
                element_index: entity.index(),
                refresh: highlight.refresh,
            })
        })
        .collect::<Result<Vec<_>, LegacySimpleAdoptError>>()?;
    Ok(MinimapV48State {
        go_in: saved.go_in,
        map_displayed: saved.map_displayed,
        transition_counter: saved.transition_counter,
        highlight_refresh: saved.highlight_refresh,
        close_after_highlight: saved.close_after_highlight,
        restore: saved.restore,
        memory_box,
        highlighted_elements,
    })
}

fn finite_minimap(field: &'static str, value: f32) -> Result<(), LegacySimpleAdoptError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LegacySimpleAdoptError::NonFiniteMinimap { field, value })
    }
}

fn resolve_pc_selection(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    saved: &LegacyElementSelection,
    field: &'static str,
) -> Result<Vec<EntityId>, LegacySimpleAdoptError> {
    saved
        .elements
        .iter()
        .map(|&reference| {
            let entity_id = entities
                .resolve_element(reference)?
                .expect("Original selection pointers are asserted non-null while loading");
            if !matches!(engine.world.entities.get(entity_id), Some(Entity::Pc(_))) {
                return Err(LegacySimpleAdoptError::SelectionIsNotPc { field, entity_id });
            }
            Ok(entity_id)
        })
        .collect()
}

fn resolve_titbit_handle(
    entities: &LegacyEntityFixups,
    reference: super::payload_base::LegacyElementRef,
) -> Result<Option<ElementHandle>, LegacySaveAdoptError> {
    Ok(entities
        .resolve_element(reference)?
        .map(|entity| ElementHandle(entity.index())))
}

fn titbit_kind(value: i32) -> Option<TitbitKind> {
    Some(match value {
        0 => TitbitKind::GunImpact,
        1 => TitbitKind::UnconsciousStar,
        2 => TitbitKind::WeakStunned,
        3 => TitbitKind::QuickAction,
        4 => TitbitKind::Counter,
        5 => TitbitKind::Smoke,
        6 => TitbitKind::Dust,
        7 => TitbitKind::Water,
        8 => TitbitKind::Lock,
        9 => TitbitKind::Emoticon,
        10 => TitbitKind::DangerPoint,
        11 => TitbitKind::Plouf,
        12 => TitbitKind::Ghost,
        13 => TitbitKind::AppleSmell,
        14 => TitbitKind::Speak,
        15 => TitbitKind::Hidden,
        16 => TitbitKind::WorkIcon,
        17 => TitbitKind::QuickActionRun,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titbit_kind_mapping_is_exact_and_bounded() {
        assert_eq!(titbit_kind(0), Some(TitbitKind::GunImpact));
        assert_eq!(titbit_kind(17), Some(TitbitKind::QuickActionRun));
        assert_eq!(titbit_kind(-1), None);
        assert_eq!(titbit_kind(18), None);
    }
}
