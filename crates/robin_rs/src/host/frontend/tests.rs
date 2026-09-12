use super::*;

#[cfg(test)]
mod interaction_reset_tests {
    use super::*;

    #[test]
    fn focus_loss_retires_both_buttons_captures_and_path_without_changing_planning() {
        let mut frontend = HostFrontend::default();
        frontend.planning.update_preference(true);
        frontend.planning.toggle_touch();
        frontend.begin_left_pointer(Default::default(), 2);
        frontend.begin_right_pointer(2);
        frontend.begin_minimap_drag(true);
        frontend.add_gesture_point(Default::default());
        frontend.route_hud_event(&crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1), true);
        frontend.lose_pointer_focus();
        assert!(!frontend.input.left_mouse_down());
        assert!(!frontend.input.controls.right_mouse_down);
        assert!(!frontend.release_left_pointer());
        assert!(!frontend.release_right_pointer());
        assert!(!frontend.pointer_capture().minimap_drag_active());
        assert!(frontend.mouse_way().is_empty());
        assert!(!frontend.route_hud_event(&crate::gfx_types::GameEvent::MouseUp(0, 0, 1), false));
        assert!(frontend.planning.touch_latched());
    }

    #[test]
    fn modal_entry_cancels_captures_and_gesture_but_preserves_held_button_semantics() {
        let mut frontend = HostFrontend::default();
        frontend.begin_left_pointer(Default::default(), 1);
        frontend.begin_right_pointer(2);
        frontend.begin_minimap_drag(true);
        frontend.add_gesture_point(Default::default());
        frontend.input.controls.is_alt = true;
        frontend.reset_modal_input();
        assert!(frontend.input.left_mouse_down());
        assert!(!frontend.input.is_dragging());
        assert!(!frontend.input.controls.is_alt);
        assert!(!frontend.pointer_capture().minimap_drag_active());
        assert!(!frontend.release_right_pointer());
        assert!(frontend.mouse_way().is_empty());
    }

    #[test]
    fn modal_reset_preserves_view_target_but_snapshot_reset_retires_it() {
        let mut frontend = HostFrontend::default();
        let selected = EntityId::Soldier(robin_engine::entity_id::SoldierId(7));
        frontend.set_selected_view_element(Some(selected));
        frontend.arm_tactical_patrol(vec![selected], TacticalFormation::Line);
        frontend.apply_trajectory_preview(engine_api::input::TrajectoryPreview::HitNoArc);
        frontend.reset_interaction(InteractionReset::ModalClosed);
        assert_eq!(frontend.selected_view_element(), Some(selected));
        assert!(!frontend.tactical_targeting().is_armed());
        assert!(!frontend.trajectory_preview().is_valid());
        frontend.reset_interaction(InteractionReset::SnapshotRestored);
        assert_eq!(frontend.selected_view_element(), None);
    }

    #[test]
    fn hover_observation_retires_explanations_without_cancelling_targeting() {
        let mut frontend = HostFrontend::default();
        frontend.arm_tactical_patrol(Vec::new(), TacticalFormation::Line);
        frontend.set_item_effect_preview(Some(ItemEffectPreview {
            center: MapPoint::ZERO,
            radius: None,
            localization_key: "test",
            fallback_text: "test",
            blocked: false,
        }));
        frontend.set_host_titbit_preview(Some(HostTitbitPreview::JumpHelperGhost {
            position: WorldPoint3D::new(0.0, 0.0, 0.0),
            layer: 0,
            sector_dir: 0,
            display_order: 0.0,
        }));
        frontend.observe_hover_feedback(false, Default::default(), MapPoint::ZERO);
        assert!(frontend.item_effect_preview().is_none());
        assert!(frontend.host_titbit_preview().is_none());
        assert!(frontend.tactical_targeting().is_armed());
        assert_eq!(frontend.trajectory_preview().hover_ticks(), 1);
    }

    #[test]
    fn interaction_diagnostics_cannot_restore_live_feedback() {
        let feedback = FrontendInteraction::default();
        let diagnostic = serde_json::to_vec(&feedback).unwrap();
        assert!(serde_json::from_slice::<FrontendInteraction>(&diagnostic).is_err());
    }

    #[test]
    fn frontend_snapshot_boundary_retires_requests_but_preserves_live_resources_and_presentation() {
        let mut host = Host::scratch(640.0, 480.0);
        let sprites = Arc::clone(host.frontend.resources.frame_holder());
        host.frontend
            .request_print_screen(PrintScreenRequest::Median3x3);
        host.frontend
            .diagnostics_mut()
            .queue_console_output("old mission output".into());
        host.frontend.diagnostics_mut().record_frame(100, 7);
        host.frontend.diagnostics_mut().observe_present_cost(42);
        host.frontend.presentation.fade_to_black = Some(FadeToBlack {
            speed: 20,
            frames_remaining: 13,
        });
        host.frontend.presentation.pc_info_overlay.visible = true;
        host.frontend.presentation.skip_render = true;
        host.frontend.slow_motion = true;
        let before = serde_json::to_value(&host.frontend.presentation).unwrap();

        host.post_load_reset();
        host.post_load_reset(); // Retirement is idempotent, not a second rebuild.

        assert!(host.frontend.take_print_screen().is_none());
        assert!(
            host.frontend
                .diagnostics_mut()
                .take_console_output()
                .is_empty()
        );
        assert_eq!(host.frontend.diagnostics().max_pending_sounds(), 7);
        assert_eq!(
            host.frontend.diagnostics().native_refresh_present_cost_us(),
            42
        );
        assert!(Arc::ptr_eq(
            &sprites,
            host.frontend.resources.frame_holder()
        ));
        assert_eq!(
            serde_json::to_value(&host.frontend.presentation).unwrap(),
            before
        );
        assert!(host.frontend.slow_motion);

        // Mission replacement constructs a fresh Host instead of resetting the
        // old mission's resources in place. Camera/preferences are then bound
        // explicitly by startup, and no pending output crosses that boundary.
        let next = Host::scratch(640.0, 480.0);
        assert!(!Arc::ptr_eq(
            &sprites,
            next.frontend.resources.frame_holder()
        ));
        assert!(next.frontend.presentation.fade_to_black.is_none());
        assert!(!next.frontend.presentation.pc_info_overlay.visible);
        assert!(!next.frontend.slow_motion);
    }

    #[test]
    fn capture_slot_preserves_wide_branch_and_latest_request_wins() {
        let mut frontend = HostFrontend::default();
        frontend.request_print_screen(PrintScreenRequest::Median3x3);
        assert!(!frontend.take_wide_snapshot_request());
        assert_eq!(
            frontend.take_print_screen(),
            Some(PrintScreenRequest::Median3x3)
        );
        assert!(frontend.take_print_screen().is_none());
        frontend.request_print_screen(PrintScreenRequest::Plain);
        frontend.request_print_screen(PrintScreenRequest::WideSnapshot);
        assert!(frontend.take_wide_snapshot_request());
        assert!(!frontend.take_wide_snapshot_request());
        assert!(frontend.take_print_screen().is_none());
        frontend.request_print_screen(PrintScreenRequest::Plain);
        frontend.reset_interaction(InteractionReset::ModalClosed);
        assert_eq!(
            frontend.take_print_screen(),
            Some(PrintScreenRequest::Plain)
        );
    }

    #[test]
    fn lifecycle_owner_diagnostics_cannot_restore_runtime_authority() {
        let resources = serde_json::to_value(FrontendResources::default()).unwrap();
        assert!(serde_json::from_value::<FrontendResources>(resources).is_err());
        let presentation = serde_json::to_value(FrontendPresentation::default()).unwrap();
        assert!(serde_json::from_value::<FrontendPresentation>(presentation).is_err());
    }

    #[test]
    fn ready_context_rejects_bootstrap_and_its_serialized_form() {
        let bootstrap = ApplicationContext::default();
        let bytes = serde_json::to_vec(&bootstrap).unwrap();
        assert!(ReadyApplicationContext::try_from(bootstrap).is_err());
        assert!(serde_json::from_slice::<ReadyApplicationContext>(&bytes).is_err());
    }

    #[test]
    fn snapshot_restore_discards_entity_and_pointer_state_but_preserves_preferences_and_pose() {
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend
            .input
            .press_left_pointer(Default::default(), 1);
        host.frontend.begin_right_pointer(2);
        host.frontend.planning.update_preference(true);
        host.frontend.route_touch_plan_event(
            &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
            true,
            |_, _| true,
        );
        host.frontend
            .interaction
            .trajectory_preview
            .apply(robin_engine::engine::input::TrajectoryPreview::HitNoArc);
        host.frontend.interaction.item_effect_preview = Some(ItemEffectPreview {
            center: MapPoint::ZERO,
            radius: Some(20),
            localization_key: "test",
            fallback_text: "test",
            blocked: false,
        });
        host.frontend
            .interaction
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::default());
        host.frontend.viewport.view_position = MapPoint::new(100.0, 200.0);
        host.frontend.viewport.zoom_factor = 2.0;
        host.frontend.planning.update_preference(true);

        host.post_load_reset();

        assert!(!host.frontend.input.is_dragging());
        assert!(!host.frontend.pointer_capture().touch_plan_captured());
        assert!(!host.frontend.release_right_pointer());
        assert!(!host.frontend.planning.touch_latched());
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert!(!host.frontend.interaction.tactical_targeting.is_armed());
        assert!(host.frontend.interaction.item_effect_preview.is_none());
        assert_eq!(
            host.frontend.viewport.view_position,
            MapPoint::new(100.0, 200.0)
        );
        assert_eq!(host.frontend.viewport.zoom_factor, 2.0);
        assert!(host.frontend.planning.enabled());
        assert!(host.frontend.resources.mission_surfaces.map().is_none());
    }

    #[test]
    fn modal_and_engine_resets_preserve_sticky_planning_but_cancel_pointer_capture() {
        for reason in [
            InteractionReset::ModalClosed,
            InteractionReset::EngineRequested,
        ] {
            let mut host = Host::scratch(640.0, 480.0);
            host.frontend.planning.update_preference(true);
            host.frontend.route_touch_plan_event(
                &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
                true,
                |_, _| true,
            );
            host.frontend
                .input
                .press_left_pointer(Default::default(), 1);
            host.frontend.viewport.begin_touch_transform(true);
            host.frontend.reset_interaction(reason);
            assert!(host.frontend.planning.touch_latched());
            assert!(!host.frontend.pointer_capture().touch_plan_captured());
            assert!(!host.frontend.input.left_mouse_down());
            assert!(!host.frontend.viewport.advance_touch_inertia(100));
        }
    }

    #[test]
    fn action_and_input_effects_keep_their_distinct_preview_reset_scopes() {
        use robin_engine::engine::input::TrajectoryPreview;
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend.interaction.trajectory_preview.observe_hover(
            false,
            Default::default(),
            MapPoint::ZERO,
        );
        host.frontend.interaction.trajectory_preview.observe_hover(
            false,
            Default::default(),
            MapPoint::ZERO,
        );
        host.frontend
            .interaction
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.frontend
            .interaction
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::Line);
        host.apply_side_effects(SideEffects {
            invalidate_trajectory_preview: true,
            ..Default::default()
        });
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert_eq!(
            host.frontend.interaction.trajectory_preview.hover_ticks(),
            2
        );
        assert!(host.frontend.interaction.tactical_targeting.is_armed());
        host.frontend
            .interaction
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.apply_side_effects(SideEffects {
            reset_input: true,
            ..Default::default()
        });
        assert_eq!(
            host.frontend.interaction.trajectory_preview.hover_ticks(),
            0
        );
        assert!(host.frontend.interaction.trajectory_preview.is_valid());
        assert!(host.frontend.interaction.tactical_targeting.is_armed());
        host.post_load_reset();
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert!(!host.frontend.interaction.tactical_targeting.is_armed());
    }
}

#[cfg(test)]
mod host_resource_tests {
    use super::*;
    use robin_assets::frame_holder::{SHADOW_KEY, SpriteVariant, TRANSPARENT_COLOR_16};
    use robin_assets::shipping_datadir::{ShippingSprite, ShippingSpriteBank};
    use robin_engine::campaign::Campaign;
    use robin_engine::coordinates::{SpriteAnchor, SpriteFrameOffset};
    use robin_engine::element::{ElementData, ElementFx, ElementKind, Entity};
    use robin_engine::sprite::Sprite;
    use robin_engine::sprite_script::SpriteScript;

    #[test]
    fn effect_batches_preserve_domain_order_and_coalesce_signals() {
        let mut effects = HostEffectBatches::default();
        effects.extend_dialogues([7]);
        effects.extend_popup_texts([11]);
        effects.extend_dialogues([8, 9]);
        effects.request_sherwood_report();
        effects.request_sherwood_report();
        effects.request_signal(HostSignal::ResetInput);
        effects.request_signal(HostSignal::ShowConsole);
        effects.request_signal(HostSignal::ResetInput);

        assert_eq!(effects.take_dialogues(), vec![7, 8, 9]);
        assert_eq!(effects.take_popup_texts(), vec![11]);
        assert!(effects.take_sherwood_report());
        assert!(!effects.take_sherwood_report());
        assert!(effects.take_signal(HostSignal::ResetInput));
        assert!(!effects.take_signal(HostSignal::ResetInput));
        assert!(effects.take_signal(HostSignal::ShowConsole));
    }

    #[test]
    fn trading_signal_requires_host_enabled_rule_and_sherwood() {
        use robin_engine::trading::TradeRejectReason;

        let allowed = SherwoodTradingAccess {
            local_is_host: true,
            enabled: true,
            in_sherwood: true,
        };
        let cases = [
            (
                SherwoodTradingAccess {
                    local_is_host: false,
                    ..allowed
                },
                TradeRejectReason::HostOnly,
            ),
            (
                SherwoodTradingAccess {
                    enabled: false,
                    ..allowed
                },
                TradeRejectReason::TradingDisabled,
            ),
            (
                SherwoodTradingAccess {
                    in_sherwood: false,
                    ..allowed
                },
                TradeRejectReason::NotInSherwood,
            ),
        ];

        for (access, reason) in cases {
            let mut effects = HostEffectBatches::default();
            assert_eq!(effects.request_sherwood_trading(access), Err(reason));
            assert!(!effects.has_signal(HostSignal::SherwoodTrading));
        }

        let mut effects = HostEffectBatches::default();
        assert_eq!(effects.request_sherwood_trading(allowed), Ok(()));
        assert!(effects.has_signal(HostSignal::SherwoodTrading));
        assert_eq!(effects.take_sherwood_trading(allowed), Ok(true));
        assert_eq!(effects.take_sherwood_trading(allowed), Ok(false));
    }

    #[test]
    fn queued_trading_signal_is_revalidated_and_drained_before_modal_dispatch() {
        use robin_engine::trading::TradeRejectReason;

        let allowed = SherwoodTradingAccess {
            local_is_host: true,
            enabled: true,
            in_sherwood: true,
        };
        let mut effects = HostEffectBatches::default();
        effects.request_sherwood_trading(allowed).unwrap();

        let disabled = SherwoodTradingAccess {
            enabled: false,
            ..allowed
        };
        assert_eq!(
            effects.take_sherwood_trading(disabled),
            Err(TradeRejectReason::TradingDisabled)
        );
        assert!(!effects.has_signal(HostSignal::SherwoodTrading));
    }

    fn dictionary_frame_holder(shadow_color: u16) -> FrameHolder {
        let mut shipping = ShippingDatadir::default();
        shipping.sprite_bank = Some(ShippingSpriteBank {
            signature: 0x51A0_0001,
            dictionaries: vec![robin_assets::frame_holder::FrameDictionary::from_raw(
                1,
                vec![SHADOW_KEY, 0x0841, TRANSPARENT_COLOR_16, 0x1234],
            )],
            sprite_count: 1,
            sprites: vec![(
                0,
                ShippingSprite {
                    width: 4,
                    height: 1,
                    dictionary_index: 0,
                    packed_data: std::sync::Arc::new(vec![0]),
                    raster: None,
                },
            )],
            vq_chunks: Vec::new(),
            rle_jxl_chunks: Vec::new(),
        });

        let installed = robin_assets::shipping_datadir::ShippingAssets::install(
            std::sync::Arc::new(shipping),
            std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()),
        )
        .expect("install synthetic dictionary bank");
        let mut holder = FrameHolder::new();
        holder
            .initialize_sprite_bank_with_progress(".", &mut |_| {}, Some(installed.datadir()))
            .expect("load synthetic dictionary bank");
        holder.generate_night_dictionaries();
        holder.apply_arno_law(shadow_color);
        holder
    }

    fn rendered_dictionary_pixel_is_opaque(
        holder: &FrameHolder,
        variant: SpriteVariant,
        shadow_color: u16,
        x: usize,
    ) -> bool {
        let mut pixels = [TRANSPARENT_COLOR_16; 4];
        holder.uncompress_frame(&mut pixels, 4, 0, variant, shadow_color, 16);
        let pixel = pixels[x];
        pixel != TRANSPARENT_COLOR_16 && pixel != SHADOW_KEY && pixel != shadow_color
    }

    fn dictionary_sprite_entity() -> Entity {
        let script = SpriteScript {
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        };
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element.sprite = Sprite {
                current_width: 4,
                current_height: 1,
                scripts: Arc::new(vec![script]),
                center: SpriteAnchor::ZERO,
                ..Default::default()
            };
            initial_element
        };
        element.set_position_map(MapPoint::new(100.0, 100.0));
        Entity::Fx(ElementFx {
            element,
            fx: Default::default(),
        })
    }

    #[test]
    fn ambiance_rebind_publishes_renderer_dictionary_generation_to_engine_hit_testing() {
        const INITIAL_NIGHT_COLOR: u16 = 0x0040;
        const REBOUND_NIGHT_COLOR: u16 = 0x0841;

        let mut host = Host::scratch(1024.0, 768.0);
        host.frontend
            .resources
            .install_frame_holder_before_publication(dictionary_frame_holder(INITIAL_NIGHT_COLOR));
        let reader = host.frontend.resources.publish_frame_holder_opacity();
        let published = host
            .frontend
            .resources
            .frame_holder_opacity
            .as_ref()
            .unwrap()
            .clone();
        let old_renderer = Arc::clone(host.frontend.resources.frame_holder());
        let old_opacity_snapshot = published.snapshot();

        let mut assets = engine_api::LevelAssets::new();
        assets.attachments.pixel_opacity = Some(reader);
        let engine =
            engine_api::Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
                .expect("construct sprite-hit-test engine");
        let entity = dictionary_sprite_entity();
        let shadow_point = MapPoint::new(100.0, 100.0);
        let solid_point = MapPoint::new(101.0, 100.0);

        assert!(Arc::ptr_eq(
            &host.frontend.resources.frame_holder,
            &published.snapshot()
        ));
        assert!(!rendered_dictionary_pixel_is_opaque(
            &host.frontend.resources.frame_holder,
            SpriteVariant::Day,
            INITIAL_NIGHT_COLOR,
            0,
        ));
        assert!(!engine.is_point_on_sprite(&assets, &entity, shadow_point, false));
        assert!(engine.is_point_on_sprite(&assets, &entity, shadow_point, true));
        let cloned_assets = assets.clone();

        // Mirrors a scripted Weather::night_color change observed by the
        // runtime visual refresh: COW-rebind the renderer generation, then
        // publish that exact Arc to the original and cloned LevelAssets
        // opacity handles.
        host.frontend
            .resources
            .rebind_frame_holder_shadow_color(REBOUND_NIGHT_COLOR);

        // Retained readers are immutable snapshots. Live engine handles follow
        // publication, while an old render generation stays byte-for-byte old.
        assert!(Arc::ptr_eq(&old_renderer, &old_opacity_snapshot));
        assert!(!Arc::ptr_eq(
            &old_renderer,
            host.frontend.resources.frame_holder()
        ));
        assert_eq!(
            old_renderer.dictionaries()[0].shadow_color(),
            INITIAL_NIGHT_COLOR
        );
        assert_eq!(
            host.frontend.resources.frame_holder().dictionaries()[0].shadow_color(),
            REBOUND_NIGHT_COLOR
        );

        assert!(Arc::ptr_eq(
            &host.frontend.resources.frame_holder,
            &published.snapshot()
        ));
        for variant in [SpriteVariant::Day, SpriteVariant::Night] {
            let renderer_shadow = rendered_dictionary_pixel_is_opaque(
                &host.frontend.resources.frame_holder,
                variant,
                REBOUND_NIGHT_COLOR,
                0,
            );
            let engine_shadow = engine.is_point_on_sprite(&assets, &entity, shadow_point, false);
            assert_eq!(renderer_shadow, engine_shadow);
            assert_eq!(
                renderer_shadow,
                engine.is_point_on_sprite(&cloned_assets, &entity, shadow_point, false)
            );
            assert!(!renderer_shadow);
        }
        assert!(rendered_dictionary_pixel_is_opaque(
            &host.frontend.resources.frame_holder,
            SpriteVariant::Day,
            REBOUND_NIGHT_COLOR,
            1,
        ));
        assert!(engine.is_point_on_sprite(&assets, &entity, solid_point, false));
        assert!(engine.is_point_on_sprite(&cloned_assets, &entity, solid_point, false));
    }

    #[test]
    #[should_panic(expected = "published frame holder cannot be mutated")]
    fn published_sprite_bank_cannot_reopen_loading_mutation() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend.resources.frame_holder_before_publication_mut();
    }

    #[test]
    #[should_panic(expected = "frame-holder opacity was already published")]
    fn sprite_publication_cannot_replace_the_live_reader() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend.resources.publish_frame_holder_opacity();
    }

    #[test]
    #[should_panic(expected = "published frame holder cannot be mutated")]
    fn sprite_bank_installation_cannot_replace_a_published_generation() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend
            .resources
            .install_frame_holder_before_publication(FrameHolder::new());
    }

    #[test]
    fn ambiance_variant_rebind_publishes_one_new_generation_and_keeps_old_snapshot() {
        let mut frontend = HostFrontend::default();
        frontend
            .resources
            .install_frame_holder_before_publication(dictionary_frame_holder(0x0040));
        frontend.resources.publish_frame_holder_opacity();
        let old_renderer = Arc::clone(frontend.resources.frame_holder());
        let published = frontend
            .resources
            .frame_holder_opacity
            .as_ref()
            .unwrap()
            .clone();

        frontend
            .resources
            .rebind_frame_holder_ambiance(engine_api::Ambiance::Fog, false, 0x1234);

        assert!(!Arc::ptr_eq(
            &old_renderer,
            frontend.resources.frame_holder()
        ));
        assert!(Arc::ptr_eq(
            frontend.resources.frame_holder(),
            &published.snapshot()
        ));
        assert!(
            !old_renderer
                .variant_dictionaries(SpriteVariant::Night)
                .is_empty()
        );
        assert!(
            old_renderer
                .variant_dictionaries(SpriteVariant::Fog)
                .is_empty()
        );
        assert!(
            frontend
                .resources
                .frame_holder()
                .variant_dictionaries(SpriteVariant::Night)
                .is_empty()
        );
        assert!(
            !frontend
                .resources
                .frame_holder()
                .variant_dictionaries(SpriteVariant::Fog)
                .is_empty()
        );
        assert_eq!(frontend.resources.frame_holder().global_shadow(), 10);
        assert_eq!(old_renderer.dictionaries()[0].shadow_color(), 0x0040);
        assert_eq!(
            published.snapshot().dictionaries()[0].shadow_color(),
            0x1234
        );
    }
}
