//! Byte-identity guard for the strum-derived overlay/log names.
//!
//! The `legacy_*` functions below are verbatim copies of the hand-written
//! tables that `Substate::log_string`, `StimulusType::log_string` and
//! `Decision::log_string` replaced. Every `u16` discriminant (valid or not)
//! must produce the same bytes, including the `???` fallbacks.

use super::*;

fn legacy_pascal_debug_name_to_hyphen_upper<T: std::fmt::Debug>(value: T) -> String {
    let name = format!("{value:?}");
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev: Option<char> = None;
    let mut chars = name.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_uppercase() {
            let split_before = prev.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || chars.peek().is_some_and(|next| next.is_lowercase()) && p.is_uppercase()
            });
            if split_before {
                out.push('-');
            }
        } else if ch.is_ascii_digit() && prev.is_some_and(|p| !p.is_ascii_digit()) {
            out.push('-');
        }

        for upper in ch.to_uppercase() {
            out.push(upper);
        }
        prev = Some(ch);
    }

    out
}

fn legacy_substate_log_string_from_u16(raw: u16) -> String {
    Substate::try_from(u32::from(raw))
        .ok()
        .and_then(legacy_substate_log_string)
        .unwrap_or_else(|| "SUBSTATE-???".to_string())
}

fn legacy_substate_log_string(substate: Substate) -> Option<String> {
    use Substate::*;

    let text = match substate {
        StartSleepingSubstates
        | EndSleepingSubstates
        | StartDefaultSubstates
        | EndDefaultSubstates
        | StartWonderingSubstates
        | EndWonderingSubstates
        | StartSeekingSubstates
        | EndSeekingSubstates
        | StartAttackingSubstates
        | EndAttackingSubstates
        | StartMenacingSubstates
        | EndMenacingSubstates
        | StartFleeingSubstates
        | EndFleeingSubstates
        | BeginAdditionalSubstates
        | AttackingRunToAvengerOnRoof
        | AttackingWaitForAvengerOnRoof
        | NumberOfSubstates
        | None => return std::option::Option::None,

        DefaultGotoPost => "SUBSTATE-DEFAULT-GOTOPOST".to_string(),
        DefaultGotoPostTurn => "SUBSTATE-DEFAULT-GOTOPOST-TURN".to_string(),
        DefaultGotoRoute => "SUBSTATE-DEFAULT-GOTOROUTE".to_string(),
        DefaultGotoRouteTurn => "SUBSTATE-DEFAULT-GOTOROUTE-TURN".to_string(),
        DefaultGotoChief => "SUBSTATE-DEFAULT-GOTOCHIEF".to_string(),
        DefaultOnPost => "SUBSTATE-DEFAULT-ONPOST".to_string(),
        DefaultOnPostLookingSidewards => "SUBSTATE-DEFAULT-ONPOST-LOOKING-SIDEWARDS".to_string(),
        DefaultInMacro => "SUBSTATE-DEFAULT-INMACRO".to_string(),
        DefaultInMacroWaitingForDone => "SUBSTATE-DEFAULT-INMACRO-WAITING-FOR-DONE".to_string(),
        WonderingBrawlGotHit => "SUBSTATE-WONDERING-BRAWL-GOTHIT".to_string(),
        SeekingBodyAwakeningSleeperr => "SUBSTATE-SEEKING-BODY-AWAKENING-SLEEPER".to_string(),
        AttackingSwordfight => "SUBSTATE-ATTACKING-SWORDFIGHT".to_string(),
        AttackingSwordfightSpecialStrike => {
            "SUBSTATE-ATTACKING-SWORDFIGHT-SPECIAL-STRIKE".to_string()
        }
        AttackingSwordfightParade => "SUBSTATE-ATTACKING-SWORDFIGHT-PARADE".to_string(),
        AttackingQuittingSwordfight => "SUBSTATE-ATTACKING-QUITTING-SWORDFIGHT".to_string(),
        AttackingSwordfightStepBack => "SUBSTATE-ATTACKING-SWORDFIGHT-STEP-BACK".to_string(),
        AttackingArcherWaitOnArcheryPath => {
            "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH".to_string()
        }
        AttackingArcherWaitOnArcheryPathBending => {
            "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH-BENDING".to_string()
        }
        other => format!(
            "SUBSTATE-{}",
            legacy_pascal_debug_name_to_hyphen_upper(other)
        ),
    };

    Some(text)
}

fn legacy_stimulus_log_string_from_u16(raw: u16) -> &'static str {
    StimulusType::try_from(u32::from(raw))
        .ok()
        .and_then(legacy_stimulus_log_string)
        .unwrap_or("EVENT-???")
}

fn legacy_stimulus_log_string(stimulus: StimulusType) -> Option<&'static str> {
    Some(match stimulus {
        StimulusType::EventView => "EVENT-VIEW",
        StimulusType::EventOutOfView => "EVENT-OUTOFVIEW",
        StimulusType::EventHear => "EVENT-HEAR",
        StimulusType::EventReachPoint => "EVENT-REACHPOINT",
        StimulusType::EventCouldntReachPoint => "EVENT-COULDNT-REACHPOINT",
        StimulusType::EventDone => "EVENT-DONE",
        StimulusType::EventImpossible => "EVENT-IMPOSSIBLE",
        StimulusType::EventTimer => "EVENT-TIMER",
        StimulusType::EventPcShotAtMe => "EVENT-PC-SHOT-AT-ME",
        StimulusType::EventSeesBody => "EVENT-SEESBODY",
        StimulusType::EventSeesObject => "EVENT-SEESOBJECT",
        StimulusType::EventSeesSoldier => "EVENT-SEES-SOLDIER",
        StimulusType::EventSeesFriendInTrouble => "EVENT-SEESFRIENDINTROUBLE",
        StimulusType::EventFitAgain => "EVENT-FITAGAIN",
        StimulusType::EventGotHit => "EVENT-GOTHIT",
        StimulusType::EventLoseConsciousness => "EVENT-LOSE-CONSCIOUSNESS",
        StimulusType::EventMissesCharly => "EVENT-MISSES-CHARLY",
        StimulusType::EventObjectAway => "EVENT-OBJECT-AWAY",
        StimulusType::EventSeesCharly => "EVENT-SEES-CHARLY",
        StimulusType::EventSyncCharly => "EVENT-SYNC-CHARLY",
        StimulusType::EventAfterScriptGoOn => "EVENT-AFTER-SCRIPT-GO-ON",
        StimulusType::EventReturnToDuty => "EVENT-RETURN-TO-DUTY",
        StimulusType::EventPanic => "EVENT-PANIC",
        StimulusType::EventEnterSwordfight => "EVENT-ENTER-SWORDFIGHT",
        StimulusType::EventQuitSwordfight => "EVENT-QUIT-SWORDFIGHT",
        StimulusType::EventSwordStrike => "EVENT-SWORDSTRIKE",
        StimulusType::EventWasp => "EVENT-WASP",
        StimulusType::EventWaspAway => "EVENT-WASP-AWAY",
        StimulusType::EventApple => "EVENT-APPLE",
        StimulusType::EventNet => "EVENT-NET",
        StimulusType::EventNetAway => "EVENT-NET-AWAY",
        StimulusType::EventSeesBeggar => "EVENT-SEES-BEGGAR",
        StimulusType::EventGetArrow => "EVENT-GET-ARROW",
        StimulusType::EventSeesBrawl => "EVENT-SEES-BRAWL",
        StimulusType::CallAlert => "CALL-ALERT",
        StimulusType::CallCombatAlert => "CALL-COMBAT-ALERT",
        StimulusType::CallHey => "CALL-HEY",
        StimulusType::CallHint => "CALL-HINT",
        StimulusType::CallInstruction => "CALL-INSTRUCTION",
        StimulusType::CallLookThere => "CALL-LOOKTHERE",
        StimulusType::CallCoordinate => "CALL-COORDINATE",
        StimulusType::CallReport => "CALL-REPORT",
        StimulusType::CallGoToOfficer => "CALL-GO-TO-OFFICER",
        StimulusType::CallMrOfficerIAmBack => "CALL-MR-OFFICER-I-AM-BACK",
        StimulusType::CallCharlyIsBack => "CALL-CHARLY-IS-BACK",
        StimulusType::CallPatrolCoordinate => "CALL-PATROL-COORDINATE",
        StimulusType::CallTowerGuardAlert => "CALL-TOWER-GUARD-ALERT",
        StimulusType::CallTowerGuardCallsMe => "CALL-TOWER-GUARD-CALLS-ME",
        StimulusType::CallFinishBrawl => "CALL-FINISH-BRAWL",
        StimulusType::CallYouJustWait => "CALL-YOU-JUST-WAIT",
        StimulusType::EventAppleChaseNear => "EVENT-APPLE-CHASE-NEAR",
        StimulusType::EventDoorCombat => "EVENT-DOOR-COMBAT",
        StimulusType::EventGaloppLoopEnd => "EVENT-GALOPP-LOOP-END",
        StimulusType::EventSeesShadow => "EVENT-SEES-SHADOW",
        StimulusType::EventArrowLaunched => "EVENT-ARROW-LAUNCHED",
        StimulusType::EventStone => "EVENT-STONE",
        StimulusType::EventAdversaryWeak => "EVENT-ADVERSARY-WEAK",
        StimulusType::EventAfterCombatInjury => "EVENT-AFTER-COMBAT-INJURY",
        StimulusType::CallCleanUpAfterBrawl => "CALL-CLEAN-UP-AFTER-BRAWL",
        StimulusType::EventMyTalk1 => "EVENT-MYTALK-1",
        StimulusType::EventMyTalk2 => "EVENT-MYTALK-2",
        StimulusType::EventMyTalk3 => "EVENT-MYTALK-3",
        StimulusType::CallYourTalk1 => "CALL-YOURTALK-1",
        StimulusType::CallYourTalk2 => "CALL-YOURTALK-2",
        StimulusType::CallYourTalk3 => "CALL-YOURTALK-3",
        StimulusType::EventGoodStrike => "EVENT-GOOD-STRIKE",
        StimulusType::EventLethalStrike => "EVENT-LETHAL-STRIKE",
        StimulusType::EventEnemyNear => "EVENT-ENEMY-NEAR",
        StimulusType::EventMyTalk0 => "EVENT-MYTALK-0",
        StimulusType::CallYourTalk0 => "CALL-YOURTALK-0",
        StimulusType::EventStop => "EVENT-STOP",
        StimulusType::NoEvent | StimulusType::ForceBattleDecision => return None,
    })
}

fn legacy_decision_log_string_from_u16(raw: u16) -> &'static str {
    Decision::try_from(u32::from(raw))
        .ok()
        .and_then(legacy_decision_log_string)
        .unwrap_or("DECISION-???")
}

fn legacy_decision_log_string(decision: Decision) -> Option<&'static str> {
    Some(match decision {
        Decision::None | Decision::PredecisionOffensive | Decision::PredecisionDefensive => {
            return None;
        }
        Decision::Cassos => "DECISION-CASSOS",
        Decision::Fight => "DECISION-FIGHT",
        Decision::Observe => "DECISION-OBSERVE",
        Decision::Reserve => "DECISION-RESERVE",
        Decision::AlertSoldiers => "DECISION-ALERT-SOLDIERS",
        Decision::RunAndAlertSoldiers => "DECISION-RUN-AND-ALERT-SOLDIERS",
        Decision::Menace => "DECISION-MENACE",
        Decision::Shoot => "DECISION-SHOOT",
        Decision::ArcherStepBack => "DECISION-ARCHER-STEP-BACK",
        Decision::LookForHelp => "DECISION-LOOK-4-HELP",
        Decision::LookForHelpIfNobodyElseDoes => "DECISION-LOOK-4-HELP-IF-NOBODY-ELSE-DOES",
        Decision::CoverBehindShieldBearer => "DECISION-COVER-BEHIND-SHIELD-BEARER",
        Decision::TooProudToAttack => "DECISION-TOO-PROUD-TO-ATTACK",
        Decision::TowerGuardAlert => "DECISION-TOWER-GUARD-ALERT",
        Decision::TowerGuardObserve => "DECISION-TOWER-GUARD-OBSERVE",
        Decision::ArcherObserve => "DECISION-ARCHER-OBSERVE",
        Decision::RunToArcheryPoint => "DECISION-RUN-TO-ARCHERY-POINT",
        Decision::RunForNewArrows => "DECISION-RUN-FOR-NEW-ARROWS",
        Decision::LastReserve => "DECISION-LAST-RESERVE",
    })
}

#[test]
fn substate_log_strings_match_legacy_table_for_every_discriminant() {
    let mut named = 0usize;
    for raw in 0..=u16::MAX {
        assert_eq!(
            Substate::log_string_from_u16(raw),
            legacy_substate_log_string_from_u16(raw),
            "substate discriminant {raw}"
        );
        if let Ok(substate) = Substate::try_from(u32::from(raw)) {
            let derived = substate.log_string().map(str::to_owned);
            assert_eq!(
                derived,
                legacy_substate_log_string(substate),
                "{substate:?}"
            );
            named += usize::from(derived.is_some());
        }
    }
    // Every substate except the 17 group markers / roof-avenger / sentinel
    // variants has a name; pinning the count keeps the table from being
    // trivially empty.
    assert_eq!(named, 237, "named substate count");
}

#[test]
fn stimulus_log_strings_match_legacy_table_for_every_discriminant() {
    for raw in 0..=u16::MAX {
        assert_eq!(
            StimulusType::log_string_from_u16(raw),
            legacy_stimulus_log_string_from_u16(raw),
            "stimulus discriminant {raw}"
        );
        if let Ok(stimulus) = StimulusType::try_from(u32::from(raw)) {
            assert_eq!(
                stimulus.log_string(),
                legacy_stimulus_log_string(stimulus),
                "{stimulus:?}"
            );
        }
    }
}

#[test]
fn decision_log_strings_match_legacy_table_for_every_discriminant() {
    for raw in 0..=u16::MAX {
        assert_eq!(
            Decision::log_string_from_u16(raw),
            legacy_decision_log_string_from_u16(raw),
            "decision discriminant {raw}"
        );
        if let Ok(decision) = Decision::try_from(u32::from(raw)) {
            assert_eq!(
                decision.log_string(),
                legacy_decision_log_string(decision),
                "{decision:?}"
            );
        }
    }
}
