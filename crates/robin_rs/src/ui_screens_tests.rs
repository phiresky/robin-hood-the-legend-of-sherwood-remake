use super::*;

// -- InfoPopup ----------------------------------------------------------

#[test]
fn info_popup_default_is_hidden() {
    let popup = InfoPopup::default();
    assert!(!popup.visible);
    assert_eq!(popup.text_id, 0);
}

#[test]
fn info_popup_show_and_tick() {
    let mut popup = InfoPopup::default();
    popup.show(42, 3);

    assert!(popup.visible);
    assert_eq!(popup.text_id, 42);

    assert!(popup.tick()); // frame 1
    assert!(popup.tick()); // frame 2
    assert!(!popup.tick()); // frame 3 — expires
    assert!(!popup.visible);
}

#[test]
fn info_popup_tick_when_hidden_returns_false() {
    let mut popup = InfoPopup::default();
    assert!(!popup.tick());
}

#[test]
fn info_popup_hide_mid_timeout() {
    let mut popup = InfoPopup::default();
    popup.show(1, 100);
    assert!(popup.tick());
    popup.hide();
    assert!(!popup.visible);
    assert!(!popup.tick());
}

#[test]
fn info_popup_reshow_resets_frame() {
    let mut popup = InfoPopup::default();
    popup.show(1, 2);
    assert!(popup.tick());
    // Re-show before expiry resets the timer.
    popup.show(2, 5);
    assert_eq!(popup.current_frame, 0);
    assert_eq!(popup.text_id, 2);
    assert_eq!(popup.timeout_frames, 5);
}

#[test]
fn info_popup_zero_timeout_expires_immediately() {
    let mut popup = InfoPopup::default();
    popup.show(1, 0);
    assert!(!popup.tick());
    assert!(!popup.visible);
}

// -- PopupScroll --------------------------------------------------------

#[test]
fn popup_scroll_single_page() {
    let mut popup = PopupScroll::new("Short text");
    assert!(!popup.has_more_pages());
    popup.on_ok();
    assert!(popup.closed);
    assert!(!popup.advance_page());
}

#[test]
fn popup_scroll_multi_page() {
    let mut popup = PopupScroll::new("First page");
    popup.text_remaining = "Second page".to_string();
    popup.on_ok();
    assert!(popup.closed);
    assert!(popup.has_more_pages());
    assert!(popup.advance_page());
    assert_eq!(popup.text, "Second page");
    assert!(!popup.closed);
    assert!(!popup.has_more_pages());
}

#[test]
fn popup_scroll_with_picture() {
    let popup = PopupScroll::new("text").with_picture(42).with_alignment(1);
    assert_eq!(popup.picture_id, 42);
    assert_eq!(popup.text_alignment, 1);
}

// -- DialogueScreen -----------------------------------------------------

#[test]
fn dialogue_sentence_progression() {
    let sentences = vec![
        DialogueSentence {
            text: "Hello".into(),
            sound_id: "snd1".into(),
            portrait_index: 0,
        },
        DialogueSentence {
            text: "Goodbye".into(),
            sound_id: "snd2".into(),
            portrait_index: 1,
        },
    ];
    let mut screen = DialogueScreen::new(1, sentences);
    assert_eq!(screen.current_sentence, -1);

    let s1 = screen.next_sentence().unwrap();
    assert_eq!(s1.text, "Hello");
    assert_eq!(screen.current_sentence, 0);

    let s2 = screen.next_sentence().unwrap();
    assert_eq!(s2.text, "Goodbye");

    assert!(screen.next_sentence().is_none());
    assert!(screen.finished);
}

#[test]
fn dialogue_stop() {
    let mut screen = DialogueScreen::new(1, vec![DialogueSentence::default()]);
    screen.on_stop();
    assert!(screen.finished);
    assert!(screen.abandoned);
}

#[test]
fn dialogue_portrait_animation() {
    let mut screen = DialogueScreen::default();
    screen.update_portrait(0.0);
    assert_eq!(screen.mouth_frame, 0);

    screen.update_portrait(0.25);
    assert_eq!(screen.mouth_frame, 3);

    screen.update_portrait(0.5);
    assert_eq!(screen.mouth_frame, 4);
}

#[test]
fn dialogue_portrait_blink_after_repeated() {
    let mut screen = DialogueScreen::default();
    // Same low volume 3 times triggers blink
    screen.update_portrait(0.005);
    assert_eq!(screen.mouth_frame, 0);
    screen.update_portrait(0.005);
    assert_eq!(screen.mouth_frame, 0);
    screen.update_portrait(0.005);
    // After MAX_FACE_COUNT (3), should alternate
    assert_eq!(screen.mouth_frame, 1);
}

// -- DebriefingScreen ---------------------------------------------------

#[test]
fn debriefing_win_ok() {
    let mut screen = DebriefingScreen::new(true, false, "Victory".into(), "You won!".into());
    assert!(screen.win);
    screen.on_ok();
    assert!(screen.closed);
    assert_eq!(screen.action, DebriefingAction::Continue);
}

#[test]
fn debriefing_restart() {
    let mut screen = DebriefingScreen::new(false, true, "Defeat".into(), "Try again".into());
    screen.on_restart();
    assert_eq!(screen.action, DebriefingAction::Restart);
    assert_eq!(screen.game_code, GameCode::LevelLoad);
    assert!(screen.load_requested);
}

#[test]
fn debriefing_pagination() {
    let mut screen = DebriefingScreen::new(true, false, "T".into(), "Page 1".into());
    screen.text_remaining = "Page 2".into();
    screen.on_ok();
    assert!(screen.has_more_pages());
    assert!(screen.advance_page());
    assert_eq!(screen.text, "Page 2");
    assert!(!screen.closed);
}

// -- MissionDescriptionScreen -------------------------------------------

#[test]
fn mission_description_start() {
    let mut screen = MissionDescriptionScreen::default();
    screen.on_start_mission();
    assert_eq!(screen.user_choice, MissionChoice::StartMission);
    assert!(!screen.men_to_blazon_mode);
    assert!(screen.closed);
}

#[test]
fn mission_description_convert_peasants() {
    let mut screen = MissionDescriptionScreen::default();
    screen.on_convert_peasants();
    assert_eq!(screen.user_choice, MissionChoice::StartMission);
    assert!(screen.men_to_blazon_mode);
}

#[test]
fn mission_description_convert_mission() {
    let mut screen = MissionDescriptionScreen::default();
    screen.on_convert_mission();
    assert_eq!(screen.user_choice, MissionChoice::ShowPendingMissions);
}

#[test]
fn mission_description_picture_default_when_no_descriptor() {
    // `get_mission_picture` falls through to the default popup
    // scroll picture when the mission's `.red` descriptor is missing.
    let picture = MissionDescriptionScreen::get_mission_picture(None);
    assert_eq!(
        picture,
        robin_engine::resource_ids::RHID_DEFAULT_POPUP_SCROLL_PICTURE
    );
}

#[test]
fn mission_description_text_message_when_no_descriptor() {
    // `get_mission_text` returns the "Unable to find..." sentinel
    // when the level descriptor is missing, without touching the
    // resource manager.
    let mut text_res = ResourceManager::new();
    let text = MissionDescriptionScreen::get_mission_text(None, &mut text_res, 0);
    assert!(text.contains("Unable to find"));
}

#[test]
fn mission_description_buttons_non_blazon() {
    // Non-blazon missions show just start + cancel.
    let screen = MissionDescriptionScreen {
        requires_blazons: false,
        show_start_mission: true,
        ..Default::default()
    };
    assert_eq!(
        screen.buttons(),
        vec![
            MissionDescriptionButton::StartMission,
            MissionDescriptionButton::Cancel
        ]
    );
}

#[test]
fn mission_description_buttons_blazon_non_pseudo() {
    // Blazon + non-pseudo = three converts + start + cancel.
    let screen = MissionDescriptionScreen {
        requires_blazons: true,
        show_start_mission: true,
        ..Default::default()
    };
    assert_eq!(
        screen.buttons(),
        vec![
            MissionDescriptionButton::ConvertPeasants,
            MissionDescriptionButton::ConvertMoney,
            MissionDescriptionButton::ConvertMission,
            MissionDescriptionButton::StartMission,
            MissionDescriptionButton::Cancel,
        ]
    );
}

#[test]
fn mission_description_buttons_blazon_pseudo() {
    // Blazon + pseudo (last-mission style) = three converts +
    // cancel, no start button (start-mission is gated on
    // `type != PSEUDO`).
    let screen = MissionDescriptionScreen {
        requires_blazons: true,
        show_start_mission: false,
        ..Default::default()
    };
    assert_eq!(
        screen.buttons(),
        vec![
            MissionDescriptionButton::ConvertPeasants,
            MissionDescriptionButton::ConvertMoney,
            MissionDescriptionButton::ConvertMission,
            MissionDescriptionButton::Cancel,
        ]
    );
}

#[test]
fn mission_description_is_enabled_reflects_convert_flags() {
    let screen = MissionDescriptionScreen {
        can_convert_peasants: true,
        can_convert_money: false,
        can_convert_mission: true,
        ..Default::default()
    };
    assert!(screen.is_enabled(MissionDescriptionButton::Cancel));
    assert!(screen.is_enabled(MissionDescriptionButton::StartMission));
    assert!(screen.is_enabled(MissionDescriptionButton::ConvertPeasants));
    assert!(!screen.is_enabled(MissionDescriptionButton::ConvertMoney));
    assert!(screen.is_enabled(MissionDescriptionButton::ConvertMission));
}

#[test]
fn mission_description_activate_disabled_is_noop() {
    let mut screen = MissionDescriptionScreen {
        can_convert_peasants: false,
        ..Default::default()
    };
    screen.activate(MissionDescriptionButton::ConvertPeasants);
    assert_eq!(screen.user_choice, MissionChoice::None);
    assert!(!screen.closed);
    assert!(!screen.men_to_blazon_mode);
}

#[test]
fn mission_description_drop_cap_non_blazon() {
    // Description top is 125; picture starts at y=40 with h=200 →
    // carveout height = 200 + 40 - 125 + 5 = 120, width = 300 + 10.
    let screen = MissionDescriptionScreen {
        requires_blazons: false,
        ..Default::default()
    };
    assert_eq!(screen.description_drop_cap(300, 200), Some((310, 120)));
}

#[test]
fn mission_description_drop_cap_blazon_is_none() {
    // Blazon layout places the description below the picture, so
    // no drop-cap carveout.
    let screen = MissionDescriptionScreen {
        requires_blazons: true,
        ..Default::default()
    };
    assert!(screen.description_drop_cap(300, 200).is_none());
}

#[test]
fn center_horizontally_three_buttons() {
    // Three 60-wide buttons with gap 8 in a 496-wide window:
    // total = 60*3 + 8*2 = 196, offset = (496 - 196) / 2 = 150.
    let xs = center_horizontally_x(&[60, 60, 60], 496, 8);
    assert_eq!(xs, vec![150, 218, 286]);
}

#[test]
fn center_horizontally_empty_is_empty() {
    assert!(center_horizontally_x(&[], 496, 8).is_empty());
}

#[test]
fn mission_description_tooltip_lookup() {
    struct StubMenuText;
    impl engine_sherwood_stat::MenuTextLookup for StubMenuText {
        fn get(&self, id: usize) -> String {
            format!("tip:{id}")
        }
    }
    let lookup = StubMenuText;
    assert_eq!(
        MissionDescriptionScreen::tooltip(MissionDescriptionButton::Cancel, &lookup),
        format!(
            "tip:{}",
            crate::ingame_menu::resources::MT_INFOBULLE_BUTTON_CANCEL
        )
    );
    assert_eq!(
        MissionDescriptionScreen::tooltip(MissionDescriptionButton::StartMission, &lookup),
        format!(
            "tip:{}",
            crate::ingame_menu::resources::MT_INFOBULLE_BUTTON_PLAY_MISSION
        )
    );
    assert_eq!(
        MissionDescriptionScreen::tooltip(MissionDescriptionButton::ConvertMoney, &lookup),
        format!(
            "tip:{}",
            crate::ingame_menu::resources::MT_INFOBULLE_BUTTON_MONEY_TO_BLAZON
        )
    );
}

// -- ShortMissionDescription -------------------------------------------

#[test]
fn short_mission_description_tracking() {
    let mut desc = ShortMissionDescription::default();
    assert!(!desc.visible);

    desc.set_mission(0, "Test mission".into(), Some(2), false);
    assert!(desc.visible);
    assert_eq!(desc.lifetime_indicator(), 2);

    desc.track_mouse(700.0, 500.0, 800.0, 600.0, 220.0, 100.0);
    // Should clamp: 700+25=725, but 725+220=945 > 800, so x = 580
    assert!((desc.position_x - 580.0).abs() < 0.1);

    desc.clear();
    assert!(!desc.visible);
}

#[test]
fn short_mission_lifetime_indicator() {
    let mut desc = ShortMissionDescription {
        remaining_lifetime: Some(0),
        ..Default::default()
    };
    assert_eq!(desc.lifetime_indicator(), 0);
    desc.remaining_lifetime = Some(3);
    assert_eq!(desc.lifetime_indicator(), 3);
    desc.remaining_lifetime = Some(10);
    assert_eq!(desc.lifetime_indicator(), 4);
    desc.remaining_lifetime = None;
    assert_eq!(desc.lifetime_indicator(), 4);
}

// -- IntroScreen -------------------------------------------------------

#[test]
fn intro_start_game() {
    let mut screen = IntroScreen::new("Robin".into(), "Level 5".into());
    screen.on_start_game();
    assert_eq!(screen.operation, IntroOperation::Start);
    assert!(screen.closed);
}

#[test]
fn intro_exit_confirmed() {
    let mut screen = IntroScreen::default();
    screen.on_exit(true);
    assert_eq!(screen.operation, IntroOperation::Exit);
    assert!(screen.closed);
}

#[test]
fn intro_exit_cancelled() {
    let mut screen = IntroScreen::default();
    screen.on_exit(false);
    assert_eq!(screen.operation, IntroOperation::Unknown);
    assert!(!screen.closed);
}

#[test]
fn intro_load_result() {
    let mut screen = IntroScreen::default();
    screen.set_load_result(true, GameCode::LevelLoad);
    assert_eq!(screen.operation, IntroOperation::Load);
    assert!(screen.closed);
}

// -- IngameScreen ------------------------------------------------------

#[test]
fn ingame_continue() {
    let mut screen = IngameScreen::default();
    screen.on_continue();
    assert_eq!(screen.game_code, GameCode::LevelInProgress);
    assert!(screen.closed);
}

#[test]
fn ingame_restart_confirmed() {
    let mut screen = IngameScreen::default();
    screen.on_restart(true);
    assert_eq!(screen.game_code, GameCode::LevelRestart);
    assert!(screen.closed);
}

#[test]
fn ingame_restart_cancelled() {
    let mut screen = IngameScreen::default();
    screen.on_restart(false);
    assert!(!screen.closed);
}

#[test]
fn ingame_quit() {
    let mut screen = IngameScreen::default();
    screen.on_quit(true);
    assert_eq!(screen.game_code, GameCode::Quit);
    assert!(screen.closed);
}

// -- OptionsScreen -----------------------------------------------------

#[test]
fn options_tracks_changes() {
    let mut screen = OptionsScreen::default();
    screen.set_graphics_result(true, false);
    assert!(screen.options_changed);
    assert!(!screen.redisplay);

    screen.set_graphics_result(false, true);
    assert!(screen.redisplay);
}

// -- GraphicsScreen ----------------------------------------------------

#[test]
fn graphics_resolution_presets() {
    assert_eq!(ResolutionPreset::Low.dimensions(), (640.0, 480.0));
    assert_eq!(ResolutionPreset::Medium.dimensions(), (800.0, 600.0));
    assert_eq!(ResolutionPreset::High.dimensions(), (1024.0, 768.0));
}

#[test]
fn graphics_resolution_from_dimensions() {
    assert_eq!(
        ResolutionPreset::from_dimensions(640.0, 480.0),
        ResolutionPreset::Low
    );
    assert_eq!(
        ResolutionPreset::from_dimensions(1024.0, 768.0),
        ResolutionPreset::High
    );
    assert_eq!(
        ResolutionPreset::from_dimensions(999.0, 999.0),
        ResolutionPreset::Medium
    );
}

#[test]
fn graphics_screen_ok_cancel() {
    let config = GraphicConfig::default();
    let mut screen = GraphicsScreen::new(config);

    screen.on_resolution(ResolutionPreset::Medium);
    assert!(screen.changed);
    assert!(screen.resolution_changed());

    screen.on_cancel();
    assert!(screen.closed);
    assert!(!screen.accepted);
    assert_eq!(screen.config.resolution_x, 1024.0); // reverted to default
}

#[test]
fn graphics_screen_toggle() {
    let config = GraphicConfig::default();
    let mut screen = GraphicsScreen::new(config);
    let original_shadow = screen.config.display_shadow;

    screen.on_toggle(GraphicsOption::TransparentShadows);
    assert_ne!(screen.config.display_shadow, original_shadow);
    assert!(screen.changed);

    assert!(screen.config.apply_fog_to_all_sprites);
    screen.on_toggle(GraphicsOption::FogTintAllSprites);
    assert!(!screen.config.apply_fog_to_all_sprites);
}

// -- SoundsScreen -------------------------------------------------------

#[test]
fn sounds_screen_ok_cancel() {
    let config = SoundConfig::default();
    let mut screen = SoundsScreen::new(config);

    screen.config.set_music_volume(3);
    screen.on_change();
    assert!(screen.changed);

    screen.on_cancel();
    assert_eq!(screen.config.music_volume, 9); // reverted
}

// -- LoadSaveScreen ----------------------------------------------------

#[test]
fn load_save_load_mode() {
    let entries = vec![
        SaveGameEntry {
            name: "Save 1".into(),
            index: 0,
            thumbnail_id: 0,
        },
        SaveGameEntry {
            name: "Save 2".into(),
            index: 1,
            thumbnail_id: 0,
        },
    ];
    let mut screen = LoadSaveScreen::new(true, entries);
    assert!(!screen.can_load_save());

    screen.on_selection_change(Some(0));
    assert!(screen.can_load_save());

    screen.on_load_save();
    assert_eq!(screen.action, LoadSaveAction::Load);
    assert!(screen.closed);
}

#[test]
fn load_save_save_mode() {
    let mut screen = LoadSaveScreen::new(false, vec![]);
    assert!(!screen.can_load_save());

    screen.on_text_change("My Save".into());
    assert!(screen.can_load_save());

    screen.on_load_save();
    assert_eq!(screen.action, LoadSaveAction::Save);
}

#[test]
fn load_save_delete() {
    let entries = vec![SaveGameEntry {
        name: "test".into(),
        index: 0,
        thumbnail_id: 0,
    }];
    let mut screen = LoadSaveScreen::new(true, entries);
    screen.on_selection_change(Some(0));
    assert!(screen.can_delete());

    screen.on_delete(true);
    assert!(screen.entries.is_empty());
    assert!(screen.selected_index.is_none());
}

// -- NewPlayerScreen ---------------------------------------------------

#[test]
fn new_player_defaults() {
    let screen = NewPlayerScreen::new();
    assert_eq!(screen.difficulty_index, 1);
    assert_eq!(screen.validated_name(), "Anonymous");
}

#[test]
fn new_player_name_validation() {
    let mut screen = NewPlayerScreen::new();
    screen.set_name("Robin Hood".into());
    assert_eq!(screen.validated_name(), "Robin Hood");

    // `validated_name` only substitutes "Anonymous" on fully empty
    // raw text; a whitespace-only name is preserved verbatim.
    screen.set_name("  ".into());
    assert_eq!(screen.validated_name(), "  ");

    screen.set_name(String::new());
    assert_eq!(screen.validated_name(), "Anonymous");
}

#[test]
fn new_player_name_length_limit() {
    let mut screen = NewPlayerScreen::new();
    let long_name = "A".repeat(100);
    screen.set_name(long_name);
    assert_eq!(screen.name.len(), MAX_PLAYER_NAME_LENGTH);
    assert_eq!(MAX_PLAYER_NAME_LENGTH, 30);
}

// -- SelectPlayerScreen ------------------------------------------------

#[test]
fn select_player_flow() {
    let names = vec!["Alice".into(), "Bob".into()];
    let mut screen = SelectPlayerScreen::new(names);
    assert!(!screen.can_select());

    screen.selected_index = Some(0);
    assert!(screen.can_select());

    let idx = screen.on_select();
    assert_eq!(idx, Some(0));
    assert!(screen.closed);
}

#[test]
fn select_player_add_delete() {
    let mut screen = SelectPlayerScreen::new(vec!["Alice".into()]);
    assert_eq!(screen.profile_names.len(), 1);

    screen.add_profile("Bob".into());
    assert_eq!(screen.profile_names.len(), 2);
    assert_eq!(screen.selected_index, Some(1));

    screen.on_delete(true);
    assert_eq!(screen.profile_names.len(), 1);
}

// -- BuyBlazonsScreen --------------------------------------------------

#[test]
fn buy_blazons_can_afford() {
    let mut screen = BuyBlazonsScreen::new(0, 100, 200);
    assert!(screen.can_buy());
    screen.on_buy();
    assert!(screen.purchased);
    assert_eq!(screen.available_funds, 100);
}

#[test]
fn buy_blazons_cannot_afford() {
    let screen = BuyBlazonsScreen::new(0, 300, 100);
    assert!(!screen.can_buy());
}

// -- MoviesScreen ------------------------------------------------------

#[test]
fn movies_outro_availability() {
    let screen = MoviesScreen::new(false);
    assert!(screen.on_outro().is_none());

    let screen = MoviesScreen::new(true);
    assert!(screen.on_outro().is_some());
}

// -- MissionWonPopup ---------------------------------------------------

#[test]
fn mission_won_opening_transition() {
    let mut popup = MissionWonPopup::new("Mission Started".into(), true);
    popup.open();
    assert!(popup.visible);
    assert_eq!(popup.phase, TransitionPhase::Opening);

    // Tick until transition completes
    let mut ticks = 0;
    while popup.tick() {
        ticks += 1;
        if ticks > 100 {
            panic!("Opening transition did not complete");
        }
    }
    assert!(popup.is_fully_open());
}

#[test]
fn mission_won_closing_transition() {
    let mut popup = MissionWonPopup {
        visible: true,
        ..Default::default()
    };
    popup.close();
    assert_eq!(popup.phase, TransitionPhase::Closing);

    let mut ticks = 0;
    while popup.tick() {
        ticks += 1;
        if ticks > 100 {
            panic!("Closing transition did not complete");
        }
    }
    assert!(!popup.visible);
}

#[test]
fn mission_won_confirm_yes() {
    let mut popup = MissionWonPopup {
        visible: true,
        ..Default::default()
    };
    popup.on_confirm(true);
    assert!(popup.confirmed);
    assert!(!popup.visible);
}

#[test]
fn mission_won_confirm_no_starts_close() {
    let mut popup = MissionWonPopup {
        visible: true,
        ..Default::default()
    };
    popup.on_confirm(false);
    assert!(!popup.confirmed);
    assert_eq!(popup.phase, TransitionPhase::Closing);
}

// -- Serde round-trip ---------------------------------------------------

#[test]
fn info_popup_serde_roundtrip() {
    let mut popup = InfoPopup::default();
    popup.show(99, 60);
    popup.tick();

    let json = serde_json::to_string(&popup).unwrap();
    let restored: InfoPopup = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.text_id, popup.text_id);
    assert_eq!(restored.visible, popup.visible);
    assert_eq!(restored.timeout_frames, popup.timeout_frames);
    assert_eq!(restored.current_frame, popup.current_frame);
}

#[test]
fn dialogue_screen_serde_roundtrip() {
    let sentences = vec![DialogueSentence {
        text: "Hello".into(),
        sound_id: "snd".into(),
        portrait_index: 3,
    }];
    let screen = DialogueScreen::new(42, sentences);
    let json = serde_json::to_string(&screen).unwrap();
    let restored: DialogueScreen = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.dialogue_id, 42);
    assert_eq!(restored.sentences.len(), 1);
    assert_eq!(restored.sentences[0].portrait_index, 3);
}

#[test]
fn mission_won_popup_serde_roundtrip() {
    let mut popup = MissionWonPopup::new("test".into(), true);
    popup.open();
    let json = serde_json::to_string(&popup).unwrap();
    let restored: MissionWonPopup = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.phase, TransitionPhase::Opening);
    assert!(restored.visible);
}
