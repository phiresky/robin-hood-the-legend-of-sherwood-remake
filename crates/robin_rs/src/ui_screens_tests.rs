use super::*;

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
