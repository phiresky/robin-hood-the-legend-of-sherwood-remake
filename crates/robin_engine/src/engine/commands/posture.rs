//! Post-recording shield and posture commands.
//! Selection iteration and macro-stop timing follow the original input handlers.

use crate::element::{Command, EntityId};
use crate::engine::EngineInner;
use crate::sequence::{Field, FieldValue, Sequence, SequenceElement, SequenceElementData};

impl EngineInner {
    pub(super) fn dispatch_player_raise_shield(
        &mut self,
        actor: &EntityId,
        protected_pc: &EntityId,
        danger_point: &crate::coordinates::WorldPoint3D,
        danger_point_layer: &u16,
    ) {
        if self.players.qa_recording_for.contains(actor) {
            // The first shield click already selected the protectee.
            // Original's second click updates the prompt state, stores
            // the concrete Seek -> RaiseShield quick action, and stops
            // recording without launching it against the live actor.
            self.world.shield.is_protected = true;
            self.world.shield.protected_pc = Some(*protected_pc);
            self.world.shield.danger_point = *danger_point;
            self.world.shield.danger_point_layer = *danger_point_layer;
            self.stop_recording_macro();
            return;
        }
        self.apply_raise_shield_with_danger(
            *actor,
            *protected_pc,
            *danger_point,
            *danger_point_layer,
        );
    }

    /// Handle the second click of the Shield two-click protocol.
    ///
    /// 1. Flip `is_protected = true` so the cursor returns to the
    ///    first-click YES/NO state.
    /// 2. Build a compound `Seek(protected_pc, tolerance=50) →
    ///    RaiseShield(Generic with ShieldDangerPoint/Layer/Protected)`
    ///    sequence and launch it.  `dispatch_raise_shield`
    ///    (`melee.rs:L1947-L1969`) reads the `ShieldDangerPoint`
    ///    property for facing.
    /// 3. Re-sync the `DangerPoint` titbit on the carrier via
    ///    `sync_danger_point_titbits`.  The titbit manager code
    ///    already sweeps `shield_danger_point` each tick, so stamping
    ///    the new value on the actor is enough.
    pub(super) fn apply_raise_shield_with_danger(
        &mut self,
        actor: EntityId,
        protected_pc: EntityId,
        danger_point: crate::coordinates::WorldPoint3D,
        danger_point_layer: u16,
    ) {
        use crate::order::OrderType;

        self.world.shield.is_protected = true;
        self.world.shield.protected_pc = Some(protected_pc);
        self.world.shield.danger_point = danger_point;
        self.world.shield.danger_point_layer = danger_point_layer;

        // Stamp the new danger point on the acting PC so
        // `sync_danger_point_titbits` refreshes the `DangerPoint`
        // titbit next tick.
        if let Some(entity) = self.world.entities.get_mut(actor)
            && let Some(actor_data) = entity.actor_data_mut()
        {
            actor_data.shield_face_point = Some(danger_point.to_map());
        }

        // Build Seek(protected_pc, tol=50, RUNNING_UPRIGHT) → RaiseShield.
        let mut seek_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(actor), OrderType::RunningUpright);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            ..
        } = &mut seek_elem.data
        {
            *element = Some(protected_pc);
            *tolerance = 50.0;
            *flags |= crate::sequence::MoveFlags::SEEK | crate::sequence::MoveFlags::SEEK_SHIELD;
        }

        let mut raise_elem = SequenceElement::new_generic(2, Command::RaiseShield, Some(actor));
        raise_elem.set_property(
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: danger_point.x,
                y: danger_point.y,
                z: danger_point.z,
            },
        );
        raise_elem.set_property(
            Field::ShieldDangerPointLayer,
            FieldValue::Integer(u32::from(danger_point_layer)),
        );
        raise_elem.set_property(Field::ShieldProtected, FieldValue::Element(protected_pc));

        let mut post_seek = Sequence::new();
        post_seek.append_element(raise_elem);
        if let SequenceElementData::Movement {
            post_seek_sequence, ..
        } = &mut seek_elem.data
        {
            *post_seek_sequence = Some(post_seek.into_post_seek());
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        self.launch_sequence(sequence);
    }

    /// Set the protector's `shield_protected` forward pointer.
    ///
    /// Passing `protectee = None` unlinks and zeroes the protector's
    /// `shield_danger_point`; when assigning a new protectee the danger
    /// point is left untouched — the shield-raise pipeline fills it (see
    /// `dispatch_raise_shield`).
    ///
    /// Silently no-ops when the protector is not a PC; non-PC entries
    /// cannot carry the shield-protection fields.
    pub(crate) fn set_shield_protected(
        &mut self,
        protector_id: EntityId,
        protectee: Option<EntityId>,
    ) {
        if protectee.is_none()
            && let Some(me) = self.world.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_danger_point = crate::coordinates::WorldPoint3D::default();
        }

        if let Some(me) = self.world.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_protected = protectee;
        }
    }

    pub(super) fn apply_crouch_down(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        seat: usize,
    ) {
        // Route through actor-level crouched-posture conversion so a PC
        // already walking/running gets its queued orders rewritten to
        // crouched variants instead of always launching a fresh
        // CrouchDown sequence.
        //
        // The macro step was already recorded by `record_macro_step_for`
        // at the top of dispatch; here we just skip the live posture
        // change for the currently-recording PC so the two paths don't
        // double up.  After the loop, stop the recording if a
        // recording PC was in the selection.
        let mut recorded_here = false;
        for &pc_id in &self.players.seats[seat].selection.clone() {
            if self.players.qa_recording_for.contains(&pc_id) {
                recorded_here = true;
                continue;
            }
            self.actor_make_crouched(sim, pc_id);
        }
        if recorded_here {
            self.stop_recording_macro();
        }
    }

    pub(super) fn apply_stand_up(&mut self, sim: &crate::sim_rng::SimulationContext, seat: usize) {
        let mut recorded_here = false;
        for &pc_id in &self.players.seats[seat].selection.clone() {
            if self.players.qa_recording_for.contains(&pc_id) {
                recorded_here = true;
                continue;
            }
            let posture = self
                .get_entity(pc_id)
                .map(|e| e.element_data().posture())
                .unwrap_or(crate::element::Posture::Upright);

            match posture {
                crate::element::Posture::Crouched => {
                    // Try rewriting the active movement sequence
                    // first, falling back to a fresh CrouchUp launch
                    // only when no active sequence is present.
                    self.actor_make_upright(sim, pc_id);
                }
                crate::element::Posture::SimulatingBeggar => {
                    let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                }
                crate::element::Posture::Spy
                | crate::element::Posture::Cloaked
                | crate::element::Posture::AnonymousArcher => {
                    let elem = SequenceElement::new(1, Command::LeaveSpy, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                }
                crate::element::Posture::Tree => {
                    let elem = SequenceElement::new(1, Command::LeaveTree, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                }
                _ => continue,
            };
        }
        if recorded_here {
            self.stop_recording_macro();
        }
    }
}
