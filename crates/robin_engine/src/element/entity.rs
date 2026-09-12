//! Entity dispatch and derived body geometry.
use super::*;

/// Helper macro — dispatch `$self` to the `element` field of every variant.
macro_rules! dispatch_element {
    ($self:expr_2021, $field:ident) => {
        match $self {
            Entity::Pc(e) => &e.element.$field,
            Entity::Soldier(e) => &e.element.$field,
            Entity::Civilian(e) => &e.element.$field,
            Entity::Fx(e) => &e.element.$field,
            Entity::Target(e) => &e.element.$field,
            Entity::Bonus(e) => &e.element.$field,
            Entity::Scroll(e) => &e.element.$field,
            Entity::Projectile(e) => &e.element.$field,
            Entity::Net(e) => &e.element.$field,
        }
    };
}

macro_rules! entity_variant_accessors {
    ($as_ref:ident, $as_mut:ident, $variant:ident, $entity:ty) => {
        pub fn $as_ref(&self) -> Option<&$entity> {
            match self {
                Self::$variant(entity) => Some(entity),
                _ => None,
            }
        }

        pub fn $as_mut(&mut self) -> Option<&mut $entity> {
            match self {
                Self::$variant(entity) => Some(entity),
                _ => None,
            }
        }
    };
}

impl Entity {
    fn human_element(&self) -> Option<&ElementData> {
        self.human_data()?;
        Some(self.element_data())
    }

    /// Resolve the original-game element kind whose per-frame update chain
    /// owns this Rust entity.
    ///
    /// `Entity`, `ElementKind`, and (for objects) `ObjectType` jointly replace
    /// the original game's concrete entity kind. Keep the match exhaustive so adding a
    /// Rust kind or object type cannot silently inherit an unrelated update.
    pub(crate) fn original_hourglass_class(&self) -> OriginalHourglassClass {
        let (expected_kind, class) = match self {
            Self::Pc(_) => (ElementKind::ActorPc, OriginalHourglassClass::ActorPc),
            Self::Soldier(_) => (
                ElementKind::ActorSoldier,
                OriginalHourglassClass::ActorSoldier,
            ),
            Self::Civilian(_) => (
                ElementKind::ActorCivilian,
                OriginalHourglassClass::ActorCivilian,
            ),
            Self::Fx(fx) => (
                ElementKind::Fx,
                if fx.fx.mobile_index.is_some() {
                    OriginalHourglassClass::FxMasked
                } else {
                    OriginalHourglassClass::Fx
                },
            ),
            Self::Target(_) => (ElementKind::Target, OriginalHourglassClass::Target),
            Self::Bonus(bonus) => match bonus.object.object_type {
                ObjectType::Ale => (ElementKind::ObjectOther, OriginalHourglassClass::Ale),
                // Cape construction creates an object with
                // OBJECT_BONUS even though Cape has its own entity kind.
                ObjectType::Cape => (ElementKind::ObjectBonus, OriginalHourglassClass::Cape),
                ObjectType::BonusAmulet
                | ObjectType::BonusAle
                | ObjectType::BonusApple
                | ObjectType::BonusArrow
                | ObjectType::BonusBlazon
                | ObjectType::BonusLambLeg
                | ObjectType::BonusNet
                | ObjectType::BonusPlants
                | ObjectType::BonusPurse
                | ObjectType::BonusRansom
                | ObjectType::BonusStone
                | ObjectType::BonusWaspNest
                | ObjectType::BonusAmpulla
                | ObjectType::BonusCoronationSpoon
                | ObjectType::BonusRichardsCrown
                | ObjectType::BonusRoyalSeal
                | ObjectType::BonusRoyalSceptre
                | ObjectType::BonusDomesdayBook
                | ObjectType::BonusSwordOfTheState => {
                    (ElementKind::ObjectBonus, OriginalHourglassClass::Bonus)
                }
                ObjectType::None
                | ObjectType::VirtualJumper
                | ObjectType::VirtualListen
                | ObjectType::Apple
                | ObjectType::Arrow
                | ObjectType::Stone
                | ObjectType::Purse
                | ObjectType::Coin
                | ObjectType::Net
                | ObjectType::Wasp
                | ObjectType::WaspNest
                | ObjectType::Scroll => panic!(
                    "Entity::Bonus has no original-game concrete-kind mapping for ObjectType::{:?}",
                    bonus.object.object_type
                ),
            },
            Self::Scroll(scroll) => match scroll.object.object_type {
                ObjectType::Scroll => (ElementKind::ObjectScroll, OriginalHourglassClass::Scroll),
                object_type => panic!(
                    "Entity::Scroll has invalid ObjectType::{object_type:?}; expected Scroll"
                ),
            },
            Self::Projectile(projectile) => {
                let class = match projectile.object.object_type {
                    ObjectType::Arrow => OriginalHourglassClass::Arrow,
                    ObjectType::Apple => OriginalHourglassClass::Apple,
                    ObjectType::Stone => OriginalHourglassClass::Stone,
                    ObjectType::Purse => OriginalHourglassClass::Purse,
                    ObjectType::Coin => OriginalHourglassClass::Coin,
                    ObjectType::WaspNest | ObjectType::BonusWaspNest => {
                        OriginalHourglassClass::WaspNest
                    }
                    ObjectType::Wasp => OriginalHourglassClass::Wasp,
                    ObjectType::None
                    | ObjectType::VirtualJumper
                    | ObjectType::VirtualListen
                    | ObjectType::Ale
                    | ObjectType::Net
                    | ObjectType::Scroll
                    | ObjectType::Cape
                    | ObjectType::BonusAmulet
                    | ObjectType::BonusAle
                    | ObjectType::BonusApple
                    | ObjectType::BonusArrow
                    | ObjectType::BonusBlazon
                    | ObjectType::BonusLambLeg
                    | ObjectType::BonusNet
                    | ObjectType::BonusPlants
                    | ObjectType::BonusPurse
                    | ObjectType::BonusRansom
                    | ObjectType::BonusStone
                    | ObjectType::BonusAmpulla
                    | ObjectType::BonusCoronationSpoon
                    | ObjectType::BonusRichardsCrown
                    | ObjectType::BonusRoyalSeal
                    | ObjectType::BonusRoyalSceptre
                    | ObjectType::BonusDomesdayBook
                    | ObjectType::BonusSwordOfTheState => panic!(
                        "Entity::Projectile has no original-game concrete-kind mapping for ObjectType::{:?}",
                        projectile.object.object_type
                    ),
                };
                (ElementKind::ObjectProjectile, class)
            }
            Self::Net(net) => match net.object.object_type {
                ObjectType::Net | ObjectType::BonusNet => {
                    (ElementKind::ObjectNet, OriginalHourglassClass::Net)
                }
                object_type => panic!(
                    "Entity::Net has invalid ObjectType::{object_type:?}; expected Net or BonusNet"
                ),
            },
        };
        assert_eq!(
            self.kind(),
            expected_kind,
            "Rust entity variant/ElementKind invariant failed for Original {class:?}"
        );
        class
    }

    pub fn entity_id_kind(&self) -> EntityIdKind {
        match self {
            Self::Pc(_) => EntityIdKind::Pc,
            Self::Soldier(_) => EntityIdKind::Soldier,
            Self::Civilian(_) => EntityIdKind::Civilian,
            Self::Fx(_) => EntityIdKind::Fx,
            Self::Target(_) => EntityIdKind::Target,
            Self::Bonus(_) => EntityIdKind::Bonus,
            Self::Scroll(_) => EntityIdKind::Scroll,
            Self::Projectile(_) => EntityIdKind::Projectile,
            Self::Net(_) => EntityIdKind::Net,
        }
    }

    entity_variant_accessors!(as_pc, as_pc_mut, Pc, ActorPc);
    entity_variant_accessors!(as_soldier, as_soldier_mut, Soldier, ActorSoldier);
    entity_variant_accessors!(as_civilian, as_civilian_mut, Civilian, ActorCivilian);
    entity_variant_accessors!(as_fx, as_fx_mut, Fx, ElementFx);
    entity_variant_accessors!(as_target, as_target_mut, Target, ElementTarget);
    entity_variant_accessors!(as_bonus, as_bonus_mut, Bonus, ElementBonus);
    entity_variant_accessors!(as_scroll, as_scroll_mut, Scroll, ElementScroll);
    entity_variant_accessors!(
        as_projectile,
        as_projectile_mut,
        Projectile,
        ElementProjectile
    );
    entity_variant_accessors!(as_net, as_net_mut, Net, ElementNet);

    // — Element data access —

    pub fn element_data(&self) -> &ElementData {
        match self {
            Self::Pc(e) => &e.element,
            Self::Soldier(e) => &e.element,
            Self::Civilian(e) => &e.element,
            Self::Fx(e) => &e.element,
            Self::Target(e) => &e.element,
            Self::Bonus(e) => &e.element,
            Self::Scroll(e) => &e.element,
            Self::Projectile(e) => &e.element,
            Self::Net(e) => &e.element,
        }
    }

    pub fn element_data_mut(&mut self) -> &mut ElementData {
        match self {
            Self::Pc(e) => &mut e.element,
            Self::Soldier(e) => &mut e.element,
            Self::Civilian(e) => &mut e.element,
            Self::Fx(e) => &mut e.element,
            Self::Target(e) => &mut e.element,
            Self::Bonus(e) => &mut e.element,
            Self::Scroll(e) => &mut e.element,
            Self::Projectile(e) => &mut e.element,
            Self::Net(e) => &mut e.element,
        }
    }

    pub fn kind(&self) -> ElementKind {
        *dispatch_element!(self, kind)
    }

    pub fn is_active(&self) -> bool {
        self.element_data().active
    }

    pub fn posture(&self) -> Posture {
        self.element_data().posture
    }

    /// Set posture through the corpse-transition guard.  Delegates to
    /// [`ElementData::set_posture`].
    pub fn set_posture(&mut self, p: Posture) {
        self.element_data_mut().set_posture(p);
    }

    /// Reveal a blipped shadow.  Pulls `direction` from the actor's
    /// `PositionInterface` and delegates to
    /// [`ElementData::reveal_blip`].
    pub fn reveal_blip(&mut self) {
        let direction = (self.position_iface().get_direction().as_u8()) as u16;
        self.element_data_mut().reveal_blip(direction);
    }

    // — Sub-data accessors (return None if the entity doesn't have that level) —

    pub fn actor_data(&self) -> Option<&ActorData> {
        match self {
            Self::Pc(e) => Some(&e.actor),
            Self::Soldier(e) => Some(&e.actor),
            Self::Civilian(e) => Some(&e.actor),
            _ => None,
        }
    }

    pub fn actor_data_mut(&mut self) -> Option<&mut ActorData> {
        match self {
            Self::Pc(e) => Some(&mut e.actor),
            Self::Soldier(e) => Some(&mut e.actor),
            Self::Civilian(e) => Some(&mut e.actor),
            _ => None,
        }
    }

    pub fn human_data(&self) -> Option<&HumanData> {
        match self {
            Self::Pc(e) => Some(&e.human),
            Self::Soldier(e) => Some(&e.human),
            Self::Civilian(e) => Some(&e.human),
            _ => None,
        }
    }

    /// Stable content/body archetype. This deliberately says nothing about
    /// allegiance, player commandability, or who owns moment-to-moment
    /// decisions.
    pub fn human_archetype(&self) -> Option<HumanArchetype> {
        match self {
            Self::Pc(_) => Some(HumanArchetype::Hero),
            Self::Soldier(_) => Some(HumanArchetype::Soldier),
            Self::Civilian(_) => Some(HumanArchetype::Civilian),
            _ => None,
        }
    }

    pub fn decision_policy(&self) -> Option<DecisionPolicy> {
        let from_brain = |brain: &AiBrain| match brain {
            AiBrain::Enemy(_) => Some(DecisionPolicy::EnemyAi),
            AiBrain::Friendly(_) => Some(DecisionPolicy::FriendlyAi),
            AiBrain::None => None,
        };
        match self {
            Self::Pc(hero) => hero
                .pc
                .ai
                .as_deref()
                .and_then(|ai| from_brain(&ai.ai_brain))
                .or(Some(
                    if hero.pc.command_interface == CommandInterface::HeroActions {
                        DecisionPolicy::PlayerDirected
                    } else {
                        DecisionPolicy::Scripted
                    },
                )),
            Self::Soldier(soldier) => {
                from_brain(&soldier.npc.ai.ai_brain).or(Some(DecisionPolicy::Scripted))
            }
            Self::Civilian(civilian) => {
                from_brain(&civilian.npc.ai.ai_brain).or(Some(DecisionPolicy::Scripted))
            }
            _ => None,
        }
    }

    pub fn command_interface(&self) -> Option<CommandInterface> {
        match self {
            Self::Pc(hero) => Some(hero.pc.command_interface),
            Self::Soldier(soldier) => Some(soldier.soldier.command_interface),
            Self::Civilian(_) => Some(CommandInterface::None),
            _ => None,
        }
    }

    pub fn mission_role(&self) -> Option<MissionRole> {
        match self {
            Self::Pc(hero) => Some(hero.pc.mission_role),
            Self::Soldier(soldier) => Some(soldier.soldier.mission_role),
            Self::Civilian(_) => Some(MissionRole::Civilian),
            _ => None,
        }
    }

    pub fn combat_stance(&self) -> Option<CombatStance> {
        match self {
            Self::Pc(hero) => Some(hero.pc.combat_stance),
            Self::Soldier(soldier) => Some(soldier.soldier.combat_stance),
            Self::Civilian(_) => Some(CombatStance::Defensive),
            _ => None,
        }
    }

    pub fn human_control_profile(&self) -> Option<HumanControlProfile> {
        Some(HumanControlProfile {
            archetype: self.human_archetype()?,
            decision_policy: self.decision_policy()?,
            command_interface: self.command_interface()?,
            mission_role: self.mission_role()?,
            combat_stance: self.combat_stance()?,
        })
    }

    pub fn is_ai_controlled_human(&self) -> bool {
        matches!(
            self.decision_policy(),
            Some(DecisionPolicy::EnemyAi | DecisionPolicy::FriendlyAi)
        )
    }

    pub fn accepts_hero_commands(&self) -> bool {
        self.command_interface() == Some(CommandInterface::HeroActions)
    }

    pub fn accepts_tactical_orders(&self) -> bool {
        self.command_interface() == Some(CommandInterface::TacticalOrders)
    }

    pub fn npc_data(&self) -> Option<&NpcData> {
        match self {
            Self::Soldier(e) => Some(&e.npc),
            Self::Civilian(e) => Some(&e.npc),
            _ => None,
        }
    }

    /// Perception and decision state for every actor which owns an AI brain.
    /// This includes ordinary NPCs and opt-in AI-controlled heroes.
    pub fn ai_actor_data(&self) -> Option<&AiActorData> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref(),
            Self::Soldier(e) => Some(&e.npc.ai),
            Self::Civilian(e) => Some(&e.npc.ai),
            _ => None,
        }
    }

    pub fn human_data_mut(&mut self) -> Option<&mut HumanData> {
        match self {
            Self::Pc(e) => Some(&mut e.human),
            Self::Soldier(e) => Some(&mut e.human),
            Self::Civilian(e) => Some(&mut e.human),
            _ => None,
        }
    }

    /// Get mutable references to both HumanData and life points simultaneously.
    ///
    /// These live on disjoint fields (human vs pc/npc), so both can be
    /// borrowed mutably at the same time — the single match arm proves
    /// non-aliasing to the borrow checker.
    pub fn human_and_life_points_mut(&mut self) -> Option<(&mut HumanData, &mut i16)> {
        match self {
            Self::Pc(e) => Some((&mut e.human, &mut e.pc.life_points)),
            Self::Soldier(e) => Some((&mut e.human, &mut e.npc.life_points)),
            Self::Civilian(e) => Some((&mut e.human, &mut e.npc.life_points)),
            _ => None,
        }
    }

    /// Get mutable references to both HumanData and the entity's Posture simultaneously.
    ///
    /// These live on disjoint fields (human vs element.posture), so both can be
    /// borrowed mutably at the same time.
    ///
    /// Test-only: production posture mutations must go through
    /// [`Entity::set_posture`] or the focused entity helpers below.
    #[cfg(test)]
    pub fn human_and_posture_mut(&mut self) -> Option<(&mut HumanData, &mut Posture)> {
        match self {
            Self::Pc(e) => Some((&mut e.human, &mut e.element.posture)),
            Self::Soldier(e) => Some((&mut e.human, &mut e.element.posture)),
            Self::Civilian(e) => Some((&mut e.human, &mut e.element.posture)),
            _ => None,
        }
    }

    /// Release a tied human while preserving their neurological state.
    ///
    /// This is deliberately strict: interaction validation guarantees a
    /// living tied human, so a stale or non-human completion is an invariant
    /// violation rather than a silent no-op.
    pub fn untie_human(&mut self) {
        let (human, posture) = match self {
            Self::Pc(e) => (&mut e.human, &mut e.element.posture),
            Self::Soldier(e) => (&mut e.human, &mut e.element.posture),
            Self::Civilian(e) => (&mut e.human, &mut e.element.posture),
            _ => panic!("cannot untie a non-human entity"),
        };
        crate::combat::untie(human, posture);
    }

    pub fn set_posture_stuck_under_net_for_human(&mut self) -> bool {
        match self {
            Self::Pc(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            Self::Soldier(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            Self::Civilian(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            _ => false,
        }
    }

    pub fn remove_net_from_human(&mut self) -> bool {
        fn apply(element: &mut ElementData, human: &mut HumanData) -> bool {
            let prev_counter = human.stuck_under_nets_counter;
            if human.stuck_under_nets_counter > 0 {
                human.stuck_under_nets_counter -= 1;
            }
            if human.stuck_under_nets_counter == 0 && element.posture == Posture::StuckUnderNet {
                element.set_posture(Posture::Lying);
            }
            prev_counter > 0 && human.stuck_under_nets_counter == 0
        }

        match self {
            Self::Pc(e) => apply(&mut e.element, &mut e.human),
            Self::Soldier(e) => apply(&mut e.element, &mut e.human),
            Self::Civilian(e) => apply(&mut e.element, &mut e.human),
            _ => false,
        }
    }

    pub fn npc_data_mut(&mut self) -> Option<&mut NpcData> {
        match self {
            Self::Soldier(e) => Some(&mut e.npc),
            Self::Civilian(e) => Some(&mut e.npc),
            _ => None,
        }
    }

    pub fn ai_actor_data_mut(&mut self) -> Option<&mut AiActorData> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut(),
            Self::Soldier(e) => Some(&mut e.npc.ai),
            Self::Civilian(e) => Some(&mut e.npc.ai),
            _ => None,
        }
    }

    pub fn soldier_data(&self) -> Option<&SoldierData> {
        match self {
            Self::Soldier(e) => Some(&e.soldier),
            _ => None,
        }
    }

    pub fn pc_data(&self) -> Option<&PcData> {
        match self {
            Self::Pc(e) => Some(&e.pc),
            _ => None,
        }
    }

    pub fn pc_data_mut(&mut self) -> Option<&mut PcData> {
        match self {
            Self::Pc(e) => Some(&mut e.pc),
            _ => None,
        }
    }

    pub fn object_data(&self) -> Option<&ObjectData> {
        match self {
            Self::Bonus(e) => Some(&e.object),
            Self::Scroll(e) => Some(&e.object),
            Self::Projectile(e) => Some(&e.object),
            Self::Net(e) => Some(&e.object),
            _ => None,
        }
    }

    pub fn object_data_mut(&mut self) -> Option<&mut ObjectData> {
        match self {
            Self::Bonus(e) => Some(&mut e.object),
            Self::Scroll(e) => Some(&mut e.object),
            Self::Projectile(e) => Some(&mut e.object),
            Self::Net(e) => Some(&mut e.object),
            _ => None,
        }
    }

    pub fn fx_data(&self) -> Option<&FxData> {
        match self {
            Self::Fx(e) => Some(&e.fx),
            Self::Target(e) => Some(&e.fx),
            _ => None,
        }
    }

    /// Whether this is an ordinary elevation-zero FX that belongs in the
    /// dedicated background-animation pass.
    ///
    /// This mirrors original-game element insertion: only base effects at
    /// elevation zero are removed from the normal display-order lists.
    /// Targets are FX-derived but not FX-base, and mobile child sprites are
    /// managed by their mobile owner, so both remain in the sorted list.
    pub fn is_background_animation(&self) -> bool {
        matches!(
            self,
            Self::Fx(e)
                if e.fx.mobile_index.is_none() && e.element.position().z == 0.0
        )
    }

    /// For FX-kind entities (Fx / Target) the per-frame draw is
    /// suppressed when the player has disabled "Display Animations"
    /// in the graphics options, unless one of the override conditions
    /// holds:
    ///   - `force_display` is set on this FX,
    ///   - the FX is a patch animation (`patch_index.is_some()`),
    ///   - the FX is elevated (z != 0).
    ///
    /// Non-FX entities (PCs, NPCs, soldiers, civilians, …) always
    /// display — the check exists only on FX-base entities.
    pub fn is_to_be_displayed(&self, display_anim: bool) -> bool {
        let Some(fx) = self.fx_data() else {
            return true;
        };
        let elem = self.element_data();
        fx.force_display || fx.patch_index.is_some() || elem.position().z != 0.0 || display_anim
    }

    /// Return the display masking polyline for this entity.
    ///
    /// FX-base entities (Fx, Target) may have a polyline that
    /// determines whether other entities render in front of or behind
    /// them.  For Targets the polyline lives on `TargetData`; for Fx
    /// it lives on `FxData`.
    pub fn display_polyline(&self) -> &[MapPoint] {
        match self {
            Self::Target(e) => &e.target.display_polyline,
            Self::Fx(e) => &e.fx.display_polyline,
            _ => &[],
        }
    }

    // — Type checks (delegated to ElementKind) —

    pub fn is_actor(&self) -> bool {
        self.kind().is_actor()
    }
    pub fn is_human(&self) -> bool {
        self.kind().is_human()
    }
    pub fn is_pc(&self) -> bool {
        self.kind().is_pc()
    }
    pub fn is_npc(&self) -> bool {
        self.kind().is_npc()
    }
    pub fn is_soldier(&self) -> bool {
        self.kind().is_soldier()
    }
    pub fn is_civilian(&self) -> bool {
        self.kind().is_civilian()
    }
    pub fn is_fx(&self) -> bool {
        self.kind().is_fx()
    }
    pub fn is_fx_target(&self) -> bool {
        self.kind().is_fx_target()
    }
    pub fn is_object(&self) -> bool {
        self.kind().is_object()
    }
    pub fn is_projectile(&self) -> bool {
        self.kind().is_projectile()
    }
    pub fn is_bonus(&self) -> bool {
        self.kind().is_bonus()
    }

    /// Map-space anchor used for sprite placement and sprite-shaped UI
    /// affordances such as hit tests and hover outlines.
    ///
    /// Targets are special: their `position_map` is the action/interact
    /// point, while the visible sprite is authored at the preserved 3D
    /// `position` (effect elements draw the generated edge map from the
    /// same sprite position as the target sprite). Other actors keep using
    /// `position_map`, adjusted by jump height so airborne sprites and
    /// their outlines stay together.
    pub fn sprite_visual_map_position(&self) -> MapPoint {
        let elem = self.element_data();
        if self.is_fx_target() {
            elem.position().to_map()
        } else {
            let pos = elem.position_map();
            let jump_z = self.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
            MapPoint::new(pos.x, pos.y - jump_z)
        }
    }

    /// Camp allegiance for fighter-camp-keyed iteration. PCs, soldiers, and
    /// civilians read their authored cached camp. Non-actor entities have no
    /// camp and return `Camp::Error`.
    pub fn camp(&self) -> Camp {
        match self {
            Self::Pc(pc) => pc.pc.cached_camp,
            Self::Soldier(s) => s.soldier.cached_camp,
            Self::Civilian(c) => c.civilian.cached_camp,
            _ => Camp::Error,
        }
    }

    /// Build a [`crate::gate::ActorAuthInfo`] for door/gate authorization checks.
    ///
    /// Extracts the kind, lock-bypass abilities, rider status and posture
    /// from the entity.  For PCs the auth bit is `1 << profile_index`
    /// and lockpick/climb availability comes from `disabled_actions`.
    pub fn actor_auth_info(&self) -> crate::gate::ActorAuthInfo {
        let kind = self.kind();
        let posture = self.element_data().posture;

        // PC-specific fields
        let (pc_auth_bit, has_lockpick, has_climb, has_jump) = if let Some(pc) = self.pc_data() {
            let auth_bit = 1u16 << u32::from(pc.profile_index).min(15);
            (auth_bit, pc.has_lockpick, pc.has_climb, pc.has_jump)
        } else {
            (0, false, false, false)
        };

        let is_rider = self.soldier_data().map(|s| s.rider).unwrap_or(false);

        crate::gate::ActorAuthInfo {
            kind,
            pc_auth_bit,
            has_lockpick,
            has_climb,
            has_jump,
            is_rider,
            posture,
        }
    }

    // Per-type behavior dispatch

    pub fn is_immortal(&self) -> bool {
        match self {
            Self::Pc(e) => e.pc.immortal,
            _ => false,
        }
    }

    pub fn is_dead(&self) -> bool {
        match self {
            Self::Pc(e) => e.pc.life_points <= 0,
            Self::Soldier(e) => e.npc.life_points <= 0,
            Self::Civilian(e) => e.npc.life_points <= 0,
            _ => true,
        }
    }

    /// Live life points of a human element; `0` for everything else.
    pub fn human_life_points(&self) -> i16 {
        match self {
            Self::Pc(e) => e.pc.life_points,
            Self::Soldier(e) => e.npc.life_points,
            Self::Civilian(e) => e.npc.life_points,
            _ => 0,
        }
    }

    /// Maximum life points of a human element: the difficulty-scaled
    /// soldier-profile value for soldiers, a flat `100` for PCs and
    /// civilians, `0` for everything else.
    pub fn human_max_life_points(&self) -> i16 {
        match self {
            Self::Pc(_) | Self::Civilian(_) => 100,
            Self::Soldier(e) => e.soldier.cached_max_life_points,
            _ => 0,
        }
    }

    pub fn is_transporting(&self) -> bool {
        match self {
            Self::Pc(e) => e.element.posture == Posture::CarryingCorpse,
            _ => false,
        }
    }

    /// Door-transit half of "is inside a building": true while an
    /// actor is in the middle of a pass-door animation, before its
    /// sector pointer has been swapped to the inside-building sector.
    pub fn is_in_door_transit(&self) -> bool {
        self.position_iface().get_door().is_some()
    }

    /// Motion query provided on `Entity` so `&Entity` callers (e.g.
    /// the right-click handler) don't need to downcast to a concrete
    /// actor variant.  See [`Actor::is_in_motion`] for the semantic.
    pub fn is_in_motion(&self) -> bool {
        let pi = self.position_iface();
        let goal = pi.map_goal();
        let pos = pi.map_position();
        (goal != pos && goal != MapPoint::ZERO) || pi.is_moving_map()
    }

    // — Cross-module accessors —

    /// Get the entity's sprite.
    pub fn sprite(&self) -> &Sprite {
        &self.element_data().sprite
    }

    /// Get a mutable reference to the entity's sprite.
    pub fn sprite_mut(&mut self) -> &mut Sprite {
        &mut self.element_data_mut().sprite
    }

    /// Get the entity's position interface. Every entity has one (it's
    /// stored on the sprite).
    pub fn position_iface(&self) -> &PositionInterface {
        &self.element_data().sprite.position_iface
    }

    /// Get the entity's position interface mutably.
    pub fn position_iface_mut(&mut self) -> &mut PositionInterface {
        &mut self.element_data_mut().sprite.position_iface
    }

    /// Original-game sprite position for gameplay hotspot lookups.
    ///
    /// `Sprite::center` is the anchor loaded from the sprite data and used by
    /// rendering/input. The original game computes sprite position as
    /// `position_map - sprite_center`, so reconstruct that here. Targets keep
    /// their visible sprite anchor separate from their interaction point:
    /// Target initialization computes the sprite position from
    /// the authored 3D placement, then overwrites only map position with the
    /// action point and marks both values computed. `sprite_visual_map_position`
    /// preserves that distinction.
    pub fn gameplay_sprite_position(&self) -> SpriteTopLeft {
        if self.is_fx_target()
            && let Some(position) = self.position_iface().cached_sprite_position()
        {
            return SpriteTopLeft::new(position.x, position.y);
        }
        let map = if self.is_fx_target() {
            self.sprite_visual_map_position()
        } else {
            self.element_data().position_map()
        };
        let center = self.sprite().center;
        SpriteTopLeft::new((map.x - center.x).floor(), (map.y - center.y).floor())
    }

    /// Original-game current map point.
    ///
    /// Sprite-script hotspots are relative to the integer sprite top-left,
    /// not to the entity's map position.  Keeping that distinction matters
    /// for use-point seeks: the original game compares and paths toward
    /// this map-space point.
    pub fn current_gameplay_point_map(&self) -> Option<MapPoint> {
        let hotspot = self.sprite().current_hotspot()?;
        let sprite_position = self.gameplay_sprite_position();
        Some(MapPoint::new(
            sprite_position.x + hotspot.x,
            sprite_position.y + hotspot.y,
        ))
    }

    /// Original-game ground-position equivalent.
    ///
    /// The original game returns the X/Y components of the stored 3D position;
    /// it does not reconstruct them from map position and the current plane.
    /// That distinction is observable when the two positions are deliberately
    /// independent and also avoids a second plane evaluation changing a
    /// direction-sector boundary.
    pub fn ground_position(&self) -> GroundPoint {
        let position = self.element_data().position();
        GroundPoint::new(position.x, position.y)
    }

    /// Get an actor's base AI controller, if it owns an AI brain.
    pub fn ai_controller(&self) -> Option<&AiController> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref()?.ai_brain.base(),
            Self::Soldier(e) => e.npc.ai_brain.base(),
            Self::Civilian(e) => e.npc.ai_brain.base(),
            _ => None,
        }
    }

    /// Get an actor's base AI controller mutably.
    pub fn ai_controller_mut(&mut self) -> Option<&mut AiController> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut()?.ai_brain.base_mut(),
            Self::Soldier(e) => e.npc.ai_brain.base_mut(),
            Self::Civilian(e) => e.npc.ai_brain.base_mut(),
            _ => None,
        }
    }

    /// Get the enemy AI subclass, if this actor has enemy AI.
    pub fn enemy_ai(&self) -> Option<&EnemyAi> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref()?.ai_brain.enemy(),
            Self::Soldier(e) => e.npc.ai_brain.enemy(),
            _ => None,
        }
    }

    /// Get the enemy AI subclass mutably.
    pub fn enemy_ai_mut(&mut self) -> Option<&mut EnemyAi> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut()?.ai_brain.enemy_mut(),
            Self::Soldier(e) => e.npc.ai_brain.enemy_mut(),
            _ => None,
        }
    }

    /// Get the friendly AI subclass, if this actor has friendly AI.
    pub fn friendly_ai(&self) -> Option<&FriendlyAi> {
        match self {
            Self::Civilian(e) => e.npc.ai_brain.friendly(),
            _ => None,
        }
    }

    /// Get the friendly AI subclass mutably.
    pub fn friendly_ai_mut(&mut self) -> Option<&mut FriendlyAi> {
        match self {
            Self::Civilian(e) => e.npc.ai_brain.friendly_mut(),
            _ => None,
        }
    }

    /// Compute the 3D eye point of a Human actor (PC / soldier / civilian).
    ///
    /// Used by the shadow polygon / view cone overlay: the overlay
    /// only renders when `eye.z >= 0`, which filters out dead or
    /// teleported-away characters.
    ///
    /// `override_posture`: if `Some`, use this posture instead of the
    /// entity's current one.  Default (None) reads `element.posture`.
    ///
    /// Returns `None` for non-Human entities (FX, objects).
    pub fn compute_eyes_point(&self, override_posture: Option<Posture>) -> Option<WorldPoint3D> {
        // Only Human actors have posture-dependent eye offsets.
        let e = self.human_element()?;

        // Rider flag — only mounted soldiers ride.
        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);

        // Emergency-lying-box halves crawling offsets.
        let emergency_lying = self.position_iface().is_using_emergency_lying_box();

        // The authoritative ground position lives in
        // `element.position_map`; see `human_feet_point_3d`.
        let mut eyes = self.human_feet_point_3d();
        // `element.posture` is not initialised at entity load (only
        // combat / ability code writes to it), so we treat
        // `Undefined` as `Upright` here to match the normal human
        // resting state.
        let raw_posture = override_posture.unwrap_or(e.posture);
        let posture = if raw_posture == Posture::Undefined {
            Posture::Upright
        } else {
            raw_posture
        };
        let dir = (e.direction().rem_euclid(16)) as usize;

        use Posture::*;
        match posture {
            HelpingToClimb | CarryingOnShoulders | Upright | OnLadder | OnWall | Flying
            | CarryingCorpse | Leisure | Spy | AnonymousArcher | Siesta => {
                eyes.z += if is_rider { 60.0 } else { 45.0 };
            }
            OnShoulders => {
                eyes.z += 85.0;
            }
            Crouched | Sitting | SimulatingBeggar | Tree => {
                eyes.z += 25.0;
            }
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                let scale = if emergency_lying { 0.5 } else { 1.0 };
                eyes.x += scale * CRAWLING_OFFSETS_X[dir];
                eyes.y += scale * CRAWLING_OFFSETS_Y[dir];
                eyes.z += 5.0;
            }
            LeaningOut => {
                // Bend forward by 40 units along the facing direction.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                eyes.x += dx * 40.0;
                eyes.y += dy * 40.0;
                eyes.z += 45.0;
            }
            // Carried / Unused / Undefined — return the feet position
            // with a small offset so the overlay still works.
            _ => {
                eyes.z += 25.0;
            }
        }

        Some(eyes)
    }

    /// Compute the detection point of a human actor.
    ///
    /// This is the *target side* 3D point used by NPC
    /// `compute_visibility`.
    ///
    /// Differs from [`compute_eyes_point`]:
    /// - Lying / Dead / DeadBack / StuckUnderNet / Tied: z+2, no
    ///   `crawlingOffsets` lateral shift (eyes uses z+5 with shift).
    /// - Carried: enumerated at z+25 (eyes asserts).
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_detection_point(&self) -> Option<WorldPoint3D> {
        let e = self.human_element()?;

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);

        // The original game's detection-point calculation copies the raw position,
        // retained 3-D cache. During the bounded elevation-crossing callback
        // window that cache still names the outgoing plane; resolving it
        // here exposes the incoming plane one callback too early.
        let mut pt = self.element_data().position();
        let raw_posture = e.posture;
        let posture = if raw_posture == Posture::Undefined {
            Posture::Upright
        } else {
            raw_posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                pt.z += if is_rider { 60.0 } else { 45.0 };
            }
            LeaningOut => {
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                pt.x += dx * 40.0;
                pt.y += dy * 40.0;
                pt.z += 45.0;
            }
            OnShoulders => {
                pt.z += 85.0;
            }
            Crouched | Sitting | SimulatingBeggar | Tree | Carried => {
                pt.z += 25.0;
            }
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                pt.z += 2.0;
            }
            // Unused / Undefined — mirror eyes-point's permissive
            // fallback so callers don't crash on unset postures during
            // entity load.
            _ => {
                pt.z += 25.0;
            }
        }

        Some(pt)
    }

    /// Compute the position for star titbits above a human actor.
    ///
    /// Differs from [`compute_eyes_point`] for specific postures:
    /// - **Dead/DeadBack**: offset 30 units along/against facing direction
    /// - **Carried**: half-crawling offset, z+32
    /// - **LeaningOut**: offset 10 units along facing direction
    /// - **Rider**: offset -10 along direction, z+65
    /// - **Default**: falls through to `compute_eyes_point`
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_stars_point(&self) -> Option<WorldPoint3D> {
        let e = self.human_element()?;

        // Live feet point — see note on `human_feet_point_3d`.
        let base = self.human_feet_point_3d();

        // Rider: offset backward from facing direction, high Z.
        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        if is_rider {
            let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
            return Some(WorldPoint3D {
                x: base.x - dx * 10.0,
                y: base.y - dy * 10.0,
                z: base.z + 65.0,
            });
        }

        use Posture::*;
        match e.posture {
            Lying | StuckUnderNet | Tied => {
                //   pt_map = floor(position_map - sprite.center) + sprite_hotspot
                //   pt_stars = (pt_map.x, pt_map.y + elev+5, elev+5)
                // The sprite-position floor is applied before the row hotspot
                // is added, so the titbit anchor matches the rendered
                // body.
                //
                // Y carries `elevation + 5` to match the codebase-wide
                // iso-Y invariant (`y = map.y + z`) — same as every
                // other arm built off `human_feet_point_3d`.
                let sprite = self.sprite();
                let map = self.element_data().position_map();
                let center = sprite.center;
                let hp = sprite.hotspot_for_row(sprite.current_row);
                let elevation = base.z;
                Some(WorldPoint3D {
                    x: (map.x - center.x).floor() + hp.x,
                    y: (map.y - center.y).floor() + hp.y + elevation + 5.0,
                    z: elevation + 5.0,
                })
            }
            Dead => {
                // Head fell forward — offset 30 units along facing.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x + dx * 30.0,
                    y: base.y + dy * 30.0,
                    z: base.z + 5.0,
                })
            }
            DeadBack => {
                // Fell backward — offset 30 units against facing.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x - dx * 30.0,
                    y: base.y - dy * 30.0,
                    z: base.z + 5.0,
                })
            }
            Carried => {
                // Flip direction by 180° (`(dir + 8) & 15`) when the
                // current animation is `BeingCarriedLittleJohn` or
                // `BeingCarriedPeasantC`.  `Posture::Carried` is only
                // set after the lift transition completes, so when
                // posture is Carried the animation is always one of
                // those two and we apply the flip unconditionally.
                let flipped_dir = ((e.direction().wrapping_add(8)) & 15) as usize;
                Some(WorldPoint3D {
                    x: base.x + 0.5 * CRAWLING_OFFSETS_X[flipped_dir],
                    y: base.y + 0.5 * CRAWLING_OFFSETS_Y[flipped_dir],
                    z: base.z + 32.0,
                })
            }
            LeaningOut => {
                // Leaning forward out of a window — small forward offset.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x + dx * 10.0,
                    y: base.y + dy * 10.0,
                    z: base.z + 45.0,
                })
            }
            // All other postures use the general eyes point.
            _ => self.compute_eyes_point(None),
        }
    }

    /// Compute the belt point (centre of mass) of a human actor.
    ///
    /// Returns `None` for non-Human entities (FX, objects).
    pub fn compute_belt_point(&self) -> Option<WorldPoint3D> {
        let e = self.human_element()?;

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Original-game belt-point computation copies the current position,
        // whose release-build accessor returns the retained mpointPosition
        // bytes without forcing 3D position recomputation. This matters during the
        // bounded elevation-crossing callback window: an arrow released there
        // must aim from the outgoing cached plane, just like the shipped game.
        let mut belt = self.element_data().position();
        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | LeaningOut | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                belt.z += if is_rider {
                    RIDER_ELEVATION_BELT_UPRIGHT
                } else {
                    HUMAN_ELEVATION_BELT_UPRIGHT
                };
            }
            OnShoulders => belt.z += 65.0,
            Carried => belt.z += 55.0,
            Sitting | Crouched | SimulatingBeggar | Tree => belt.z += 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => belt.z += 5.0,
            // Unknown postures get a safe fallback.
            _ => belt.z += 10.0,
        }

        Some(belt)
    }

    /// Live base 3D point every `compute_*_point` posture switch starts from:
    /// the actor's stored 3D position.
    ///
    /// It satisfies `y = map.y + z`, but only the stored value is the numeric
    /// contract. Recomputing it as `map.y + elevation` lands one bit away
    /// whenever the map coordinates were themselves projected down from a
    /// 3D-authoritative position, which then shows up in the endpoints of
    /// opaque-reachability queries.
    pub(super) fn human_feet_point_3d(&self) -> WorldPoint3D {
        self.element_data().position()
    }

    /// Compute the hand point of a human actor.
    ///
    /// Per-frame sprite hand-anchor for the current animation row,
    /// with `+elevation` folded into Y.
    ///
    /// `forced_elevation`: when `Some(value)`, the Z is set to
    /// `elevation + value` and the posture switch is skipped.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_hand_point(&self, forced_elevation: Option<f32>) -> Option<WorldPoint3D> {
        let e = self.human_element()?;

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Seed X/Y from the per-frame sprite hotspot; fall back to
        // the feet point if the sprite has no script bound (headless
        // test).
        let elevation = self.position_iface().get_elevation();
        let mut hand = match self.sprite().current_hotspot() {
            Some(hp) => {
                let ps = self.gameplay_sprite_position();
                WorldPoint3D {
                    x: ps.x + hp.x,
                    y: ps.y + hp.y + elevation,
                    z: elevation,
                }
            }
            None => self.human_feet_point_3d(),
        };

        // When `forced_elevation` is provided, skip the posture switch.
        if let Some(fe) = forced_elevation {
            hand.z = elevation + fe;
            return Some(hand);
        }

        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                hand.z = elevation
                    + if is_rider {
                        45.0
                    } else {
                        HUMAN_ELEVATION_BELT_UPRIGHT
                    };
            }
            LeaningOut => hand.z = elevation + 25.0,
            OnShoulders => hand.z = elevation + 65.0,
            Sitting | Crouched | SimulatingBeggar | Tree => hand.z = elevation + 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => hand.z = elevation + 5.0,
            // Unknown postures get a safe fallback.
            _ => hand.z = elevation + 10.0,
        }

        Some(hand)
    }

    /// Compute the hand point with explicit direction, animation, and posture.
    ///
    /// Used for throwing projectiles where the animation and facing
    /// direction may differ from the entity's current state.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_hand_point_for_posture(
        &self,
        direction: i16,
        animation: OrderType,
        posture: Posture,
    ) -> Option<WorldPoint3D> {
        // Validate this is a human entity.
        match self {
            Self::Pc(_) | Self::Soldier(_) | Self::Civilian(_) => {}
            _ => return None,
        }

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Seed X/Y from the sprite hotspot for the requested
        // animation+direction (mirrors bow_shot::shoot_order_type_for_mode
        // sprite lookup pattern).  Fall back to feet point if the lookup
        // fails (e.g. unmapped animation).
        let elevation = self.position_iface().get_elevation();
        let mut hand = match self.sprite().get_point(animation, direction as u16) {
            Some(hp) => {
                let ps = self.gameplay_sprite_position();
                WorldPoint3D {
                    x: ps.x + hp.x,
                    y: ps.y + hp.y + elevation,
                    z: elevation,
                }
            }
            None => self.human_feet_point_3d(),
        };
        let posture = if posture == Posture::Undefined {
            Posture::Upright
        } else {
            posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                hand.z = elevation
                    + if is_rider {
                        40.0
                    } else {
                        HUMAN_ELEVATION_BELT_UPRIGHT
                    };
            }
            LeaningOut => hand.z = elevation + 25.0,
            OnShoulders => hand.z = elevation + 65.0,
            Sitting | Crouched | SimulatingBeggar | Tree => hand.z = elevation + 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => hand.z = elevation + 5.0,
            _ => hand.z = elevation + 10.0,
        }

        Some(hand)
    }

    /// Compute the feet point of a human actor.
    ///
    /// Used by the Lock titbit kind to display at foot level.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_feet_point(&self) -> Option<WorldPoint3D> {
        let e = self.human_element()?;

        let emergency_lying = self.position_iface().is_using_emergency_lying_box();

        let mut feet = self.human_feet_point_3d();
        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            // Standing postures: feet at ground level + 5.
            Upright | Spy | LeaningOut | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                feet.z += 5.0;
            }
            // Crouching/sitting: same small offset.
            Tree | Crouched | Sitting | SimulatingBeggar => {
                feet.z += 5.0;
            }
            // On shoulders / carried: elevated.
            OnShoulders | Carried => {
                feet.z += 45.0;
            }
            // Lying/dead: feet displaced opposite to facing direction.
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                let dir = (e.direction().rem_euclid(16)) as usize;
                let scale = if emergency_lying { 0.5 } else { 1.0 };
                feet.x -= scale * CRAWLING_OFFSETS_X[dir];
                feet.y -= scale * CRAWLING_OFFSETS_Y[dir];
                feet.z += 5.0;
            }
            _ => {
                feet.z += 5.0;
            }
        }

        Some(feet)
    }

    /// 3D centre point used as the projectile aim anchor for FX targets.
    ///
    /// Start from the element position and lift `z` by half the
    /// sprite's screen-Y extent.  In this isometric projection
    /// screen-Y and world-Z run along the same visual axis, so the
    /// half-pixel-height add lands the centre roughly midway up the
    /// sprite.
    ///
    /// Returns `None` for non-FX-target entities — those have their own
    /// dedicated centre helpers (`compute_eyes_point`, `compute_hand_point`,
    /// etc.) that the caller should reach for instead.
    pub fn compute_target_center(&self) -> Option<WorldPoint3D> {
        if !self.is_fx_target() {
            return None;
        }
        let elem = self.element_data();
        let half_h = elem.sprite.current_max_height() as f32 * 0.5;
        Some(WorldPoint3D {
            x: elem.position().x,
            y: elem.position().y,
            z: elem.position().z + half_h,
        })
    }

    /// Get the AI top-level state for NPCs.
    pub fn ai_state(&self) -> Option<AiTopState> {
        match self {
            Self::Soldier(e) => Some(e.npc.ai_state()),
            Self::Civilian(e) => Some(e.npc.ai_state()),
            _ => None,
        }
    }
}
