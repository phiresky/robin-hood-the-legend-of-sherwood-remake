//! AI parameters — constants controlling NPC/AI behavior.
//!
//! All time values are in game ticks (1 tick ≈ 1/25s unless noted).
//! Distance values are in game units.

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

pub const MAX_NPC_ARROWS: i32 = 20;

// ---------------------------------------------------------------------------
// (1) Reaction times (E3 version)
// ---------------------------------------------------------------------------

pub const AI_MAX_STEPS_REACTIONTIME: i32 = 60;
pub const AI_MAX_STANDARD_REACTIONTIME: i32 = 50;
pub const AI_MAX_DEADBODY_REACTIONTIME: i32 = 80;
pub const AI_MAX_FRIENDINTROUBLE_REACTIONTIME: i32 = 10;
pub const AI_MAX_ENEMY_REACTIONTIME: i32 = 30;
pub const AI_MAX_LOSTENEMY_REACTIONTIME: i32 = 10;
pub const AI_MAX_MENACE_REACTIONTIME: i32 = 10;
pub const AI_MAX_PRISONER_REACTIONTIME: i32 = 10;
pub const AI_MAX_GOT_HIT_REACTIONTIME: i32 = 10;
pub const AI_MAX_ALERT_REACTIONTIME: i32 = 30;
pub const AI_RUNNING_ENEMY_REACTIONTIME: i32 = 10;
pub const AI_QUICK_ENEMY_REACTIONTIME: i32 = 10;
pub const AI_STANDARD_NOISE_REACTIONTIME: i32 = 10;

// ---------------------------------------------------------------------------
// (2) Other time constants
// ---------------------------------------------------------------------------

pub const AI_WAKEUP_IDLING_TIME: i32 = 100;
pub const AI_HIDING_TIME: i32 = 3000;
pub const AI_LOOK_TIME: i32 = 10;
pub const AI_SEEKPOINT_LOOK_TIME: i32 = 20;
pub const AI_ANIMALNOISE_LOOK_TIME: i32 = 20;
pub const AI_FIRST_LOOK_TIME: i32 = 60;
pub const AI_FIRST_NOISE_LOOK_TIME: i32 = 10;
pub const AI_LOSTENEMY_REDCONETIME: i32 = 10;
pub const AI_MAX_YELLOWCONETIME: i32 = 20;
pub const AI_GLANCE_TIME: i32 = 30;
pub const AI_WHISTLE_GLANCE_TIME: i32 = 30;
pub const AI_AMBUSH_POINT_GLANCE_TIME: i32 = 20;
pub const AI_QUICK_GLANCE_TIME: i32 = 20;
pub const AI_MAX_BETWEEN_SHOTS_TIME: i32 = 50;
pub const AI_RELOAD_TIME: i32 = 20;
pub const AI_MIN_PAUSE_TIME: i32 = 40;
pub const AI_MAX_PAUSE_TIME: i32 = 400;
pub const AI_LOOK_PRISONER_TIME: i32 = 50;
pub const AI_LOOK_PRISONERS_MENACER_TIME: i32 = 50;
pub const AI_UNCONSCIOUS_RECOVER_TIME: i32 = 20;
pub const AI_PUNCH_GOBACK_TIME: i32 = 30;
pub const AI_BATTLE_OVERVIEW_TIME: i32 = 18;
pub const AI_END_OVERVIEW_TIME: i32 = 6;
pub const AI_QUICK_OVERVIEW_TIME: i32 = 6;
pub const AI_SURRENDER_RECONSIDER_TIME: i32 = 200;
pub const AI_SEES_FRIEND_STANDUP_TIME: i32 = 20;
pub const AI_WATCH_DEADBODY_AGAIN_TIME: i32 = 50;
pub const AI_MENACE_FROM_FAR_TIME: i32 = 50;
pub const AI_MENACE_APROACH_STOP_TIME: i32 = 30;
pub const AI_EXPLOSION_WAIT_TIME: i32 = 150;
pub const AI_AFTER_EXPLOSION_WAIT_TIME: i32 = 30;
pub const AI_STANDARD_TALK_TIME: i32 = 30;
pub const AI_POINT_TIME: i32 = 30;
pub const AI_SHORT_LOOK_TIME: i32 = 5;
pub const AI_REGROUP_TIME: i32 = 15;
pub const AI_FOLLOW_DELAY: i32 = 30;
pub const AI_MIN_COVER_TIME: i32 = 3;
pub const AI_MAX_COVER_TIME: i32 = 20;
pub const AI_DAZZLE_TIME: i32 = 20;
pub const AI_DAZZLE_LUMINA_TIME: i32 = 100;
pub const AI_GET_CRAZY_TIME: i32 = 20;
pub const AI_GLANCE_ARROW_TIME: i32 = 20;
pub const AB_GLANCE_NOISE_TIME: i32 = 20;
pub const AB_NERVOUS_GLANCE_PC_TIME: i32 = 20;
pub const AB_FRIENDLY_GLANCE_PC_TIME: i32 = 20;
pub const AB_NEUTRAL_GLANCE_PC_TIME: i32 = 10;
pub const AI_LEAVE_HOUSE_INTERVAL: i32 = 10;
pub const AI_GIVE_HINT_TIME_1: i32 = 15;
pub const AI_GIVE_HINT_TIME_2: i32 = 20;
/// Must be > `AI_GIVE_HINT_TIME_1`.
pub const AI_GET_HINT_TIME: i32 = 30;
pub const AI_STANDARD_CONTROL_INTERVAL: i32 = 10;
pub const AI_LOOK_AFTER_HINT_TIME: i32 = 50;
pub const AI_FIRING_SECURITY_TIME: i32 = 20;
pub const AI_SOD_WATCH_TIME_1: i32 = 60;
pub const AI_SOD_WATCH_TIME_2: i32 = 40;

/// ~15 minutes of game time.
pub const AI_HINT_EXPIRANCY_TIME: i32 = 30_000;

pub const AI_CHECKFOR_TIME_INTERVAL: i32 = 10;
pub const AI_CHARLY_LOOK_TIME: i32 = 50;

/// Cooldown after an enemy detection during which `InitializeFriendCheck`
/// skips the visibility/wait sequence and just resumes the macro — the
/// soldier has more important things to do than verify a partner.
pub const NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME: u32 = 3000;

/// In units of 16 frames.
pub const AI_INHOUSE_IGNORE_NOISE_TIME: i32 = 20;
/// ~5 minutes of game time.
pub const AI_SEEKED_POINT_SAFE_TIME: i32 = 10_000;
pub const AI_MENACING_PATIENCE: i32 = 100;

pub const AI_SHORT_REMARK_FORBIDDEN_TIME: i32 = 10;
pub const AI_REMARK_FORBIDDEN_TIME: i32 = 200;
pub const AI_DRUNKEN_REMARK_FORBIDDEN_TIME: i32 = 1000;
pub const AI_LOOK_DOWN_TIME: i32 = 30;

// ---------------------------------------------------------------------------
// (3) Civilian time constants
// ---------------------------------------------------------------------------

pub const AB_MIN_DEFAULT_LOOK_TIME: i32 = 20;
pub const AB_DELTA_DEFAULT_LOOK_TIME: i32 = 40;
pub const AB_MIN_DEFAULT_TALK_TIME: i32 = 20;
pub const AB_DELTA_DEFAULT_TALK_TIME: i32 = 40;
pub const AB_CONTINUE_TALK_OR_LOOK_TIME: i32 = 30;
pub const AB_MIN_PANIC_HIDING_TIME: i32 = 500;
pub const AB_DELTA_PANIC_HIDING_TIME: i32 = 500;
pub const AB_MIN_HIT_LIEDOWN_TIME: i32 = 50;
pub const AB_DELTA_HIT_LIEDOWN_TIME: i32 = 50;
pub const AI_UNCONSCIOUS_KICKED_WAIT_TIME: i32 = 10;
pub const AI_INBATTLE_LOOK_BOOOM_TIME: i32 = 20;
pub const AI_MENACING_GLANCE_NOISE_TIME: i32 = 10;
pub const AI_PRISONER_EXECUTED_LOOK_ENEMY_TIME: i32 = 50;

// ---------------------------------------------------------------------------
// AI decision parameters (minimum attribute values to trigger behaviours)
// ---------------------------------------------------------------------------

pub const MINVALUE_SLEEPING_HELP: i32 = 50;
pub const MINVALUE_WAKEUP_DONT_IDLE: i32 = 50;
pub const MINVALUE_FOLLOW_ENEMY: i32 = 30;
pub const MINVALUE_RUN: i32 = 50;
pub const MINVALUE_HELP: i32 = 20;
pub const MINVALUE_WATCH: i32 = 10;
pub const MINVALUE_ATTACK: i32 = 20;
pub const MINVALUE_NOISE_1: i32 = 10;
pub const MINVALUE_NOISE_2: i32 = 10;
pub const MINVALUE_NOISE_3: i32 = 10;
pub const MINVALUE_STEPS_1: i32 = 10;
pub const MINVALUE_STEPS_2: i32 = 10;
pub const MINVALUE_STEPS_3: i32 = 10;
pub const MINVALUE_WHISTLE_1: i32 = 10;
pub const MINVALUE_WHISTLE_2: i32 = 10;
pub const MINVALUE_PRISONER: i32 = 60;
pub const MINVALUE_STAY_ON_POST: i32 = 50;

// ---------------------------------------------------------------------------
// Noise constants
// ---------------------------------------------------------------------------

pub const AI_MINIMAL_WAKEUP_NOISE_VOLUME: i32 = 100;
pub const AI_MINIMAL_ENDIDLING_NOISE_VOLUME: i32 = 30;
pub const NOISE_VOLUME_PLOUF: i32 = 300;
pub const NOISE_VOLUME_BONK: i32 = 70;
pub const NOISE_VOLUME_ZONK: i32 = 50;
pub const NOISE_VOLUME_OUCH: i32 = 100;
pub const NOISE_VOLUME_TIRILI: i32 = 200;
pub const NOISE_VOLUME_ARFARF: i32 = 500;
pub const NOISE_VOLUME_PUTPUT: i32 = 200;
pub const NOISE_VOLUME_AAARGH: i32 = 70;
pub const NOISE_VOLUME_HEEELP: i32 = 200;
pub const NOISE_VOLUME_PLING: i32 = 200;
pub const NOISE_VOLUME_PFIIIT: i32 = 400;
pub const NOISE_VOLUME_LOGS: i32 = 400;
pub const NOISE_VOLUME_DRAWBRIDGE: i32 = 500;
/// Audible radius of a deliberately ground-thrown stone. It is broad enough
/// to pull guards off a nearby patrol without behaving like the map-wide
/// drawbridge/thunder events.
pub const NOISE_VOLUME_DISTRACTION: i32 = 250;

pub const MIN_SUBJECTIVE_NOISE_VOLUME_TO_GO: i32 = 0;
pub const CALL_VOLUME_ALERTING_CIVILIST: i32 = 650;
pub const NOISE_VOLUME_THUNDER: i32 = 1000;
pub const CALL_VOLUME_LOOKTHERE: i32 = 100;

// ---------------------------------------------------------------------------
// Number of looks / watches
// ---------------------------------------------------------------------------

pub const AI_SLEEPING_HEARD_NOISE_LOOKS: i32 = 5;
pub const AI_IDLING_HEARD_NOISE_LOOKS: i32 = 3;
pub const AI_HEARD_NOISE_LOOKS: i32 = 7;
pub const AI_SCENE_OF_THE_CRIME_LOOKS: i32 = 7;
pub const AI_DEFAULT_WATCHING_LOOKS: i32 = 7;
pub const AI_LAZY_WATCHING_LOOKS: i32 = 3;
pub const AI_SEEKING_REACHED_POINT_LOOKS: i32 = 8;
pub const AI_UNCONSCIOUS_RECOVER_LOOKS: i32 = 3;
pub const AI_MIN_SEEKING_ENEMY_WALKS: i32 = 2;
pub const AI_MAX_SEEKING_ENEMY_WALKS: i32 = 10;
pub const AI_SEEKING_ENEMY_LOOKS: i32 = 5;
pub const AI_DETECTED_WATCH_LOOKS: i32 = 3;
pub const AI_DETECTED_WATCH_SOD_LOOKS: i32 = 2;
pub const AI_HEARED_WHISTLING_LOOKS: i32 = 2;
pub const AI_LUMINA_LOOKS: i32 = 5;
pub const AI_HINT_WATCHING_LOOKS: i32 = 5;
pub const AI_SOD_WATCHES: i32 = 6;
pub const AI_ARE_THERE_MORE_DOLLARS_LOOKS: i32 = 3;

// ---------------------------------------------------------------------------
// Distance constants (in game units)
// ---------------------------------------------------------------------------

pub const AI_IDENTIFY_GOFORWARD_STEPS: i32 = 200;
pub const AI_MAX_GOBACK_STEPS: i32 = 50;
pub const AI_MAX_GOFORWARD_STEPS: i32 = 50;
pub const AI_MAX_RUNFORWARD_STEPS: i32 = 100;
pub const AI_MIN_ENEMYDISTANCE: i32 = 60;
pub const AI_MIN_FRIENDDISTANCE: i32 = 80;
pub const AI_PUNCH_GOBACK_STEPS: i32 = 50;
pub const AI_PUNCH_MAX_HITBACK_DISTANCE: i32 = 50;
pub const AI_SHOT_MISSED_GO_SIDEWARDS_DISTANCE: i32 = 30;
pub const AI_IN_BATTLE_GO_SIDEWARDS_DISTANCE: i32 = 10;
pub const AI_STOP_BEFORE_BODY_STEPS: i32 = 30;
pub const AI_GO_SIDEWARDS_INSTEAD_TRESHOLD: i32 = 10;
pub const AI_MAX_MENACE_DISTANCE: i32 = 100;
pub const AI_MIN_MENACE_GOFORWARD_STEPS: i32 = 30;
pub const AI_DELTA_MENACE_GOFORWARD_STEPS: i32 = 20;
pub const AI_VISIT_MENACING_FRIEND_DISTANCE: i32 = 20;
pub const AB_MAX_TALKTO_DISTANCE: i32 = 80;
pub const AB_MIN_WALK_THEN_TALK_DISTANCE: i32 = 30;
pub const AB_VISIT_TO_TALK_DISTANCE: i32 = 20;
pub const AB_MIN_PANIC_RUN_SEGMENT_DISTANCE: i32 = 30;
pub const AB_DELTA_PANIC_RUN_SEGMENT_DISTANCE: i32 = 100;
pub const AI_MAX_NOISE_FAILURE_FACTOR: f32 = 0.1;
pub const AI_MIN_SEARCHNOISE_DISTANCE: i32 = 50;
pub const AI_DELTA_SEARCHNOISE_DISTANCE: i32 = 50;
pub const AI_HELP_FRIEND_IN_TROUBLE_DISTANCE: i32 = 50;
pub const AI_BATTLE_FLEE_DISTANCE: i32 = 100;
pub const AI_STANDARD_WATCHING_STEPS: i32 = 10;
pub const AI_DEADBODY_WATCHING_STEPS: i32 = 50;
pub const AI_SEEKENEMY_STEPS: i32 = 60;
pub const AI_DONT_FACE_TO_WALL_DISTANCE: i32 = 80;
pub const AI_STAND_UP_DISTANCE: i32 = 10;
pub const AI_STOP_BEFORE_MONEY_DISTANCE: i32 = 20;
pub const AI_TALK_DISTANCE: i32 = 70;
pub const AI_MIN_SURROUND_DISTANCE: i32 = 35;
pub const AI_MAX_SURROUND_DISTANCE: i32 = 50;
pub const AI_BRANCH_BEFORE_PATH_DISTANCE: i32 = 50;
pub const AI_STOP_BEFORE_SEEKPOINT_DISTANCE: i32 = 80;
pub const AI_MIN_RUN_CRAZY_STEPS: i32 = 10;
pub const AI_MAX_RUN_CRAZY_STEPS: i32 = 70;
pub const AI_SHOOT_CRAZY_DISTANCE: i32 = 200;
pub const AI_HIT_DISTANCE: i32 = 30;

pub const AI_MAX_HIDE_POINT_DISTANCE: i32 = 200;
pub const HIDE_POINT_TO_HIDE_POINT_DISTANCE: i32 = 10;
pub const AI_COVER_SHOOT_GABARIT: i32 = 10;
pub const AB_MAX_FALSE_SEEKPOINT_DISTANCE: i32 = 800;

pub const AI_BODY_CONTACT_DISTANCE: i32 = 50;
pub const AI_GROUP_DISTANCE: i32 = 150;
pub const AI_CHECK_AMBUSH_POINT_WHEN_SEARCH_DIST: i32 = 300;

pub const AI_IGNORE_NOISE_SEEKPOINT_DISTANCE: i32 = 150;

pub const AI_MIN_RIDE_DISTANCE: i32 = 300;
pub const AI_HORSE_ACCELERATION_FACTOR: f32 = 0.5;

pub const AI_NEAR_ALERT_RADIUS: i32 = 150;

pub const AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS: i32 = 700;

// ---------------------------------------------------------------------------
// Seek point constants
// ---------------------------------------------------------------------------

pub const SEEK_POINT_UNIFY_TOLERANCE: i32 = 10;
pub const SEEK_POINT_MAX_RADIUS: i32 = 1000;
pub const SEEK_POINT_MAX_SQR_RADIUS: i32 = 1_000_000;
pub const DIRECTION_SEEK_POINT_MAX_SQR_RADIUS: i32 = 250_000;
pub const SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST: i32 = 10_000;
pub const SEEK_POINT_TIME_TO_REGAIN_1_PERCENT_OF_INTEREST: i32 = 100;
pub const SEEK_POINT_EXAMINE_DELTA_INTEREST: i32 = 50;
pub const SEEK_POINT_NUMBER_FACTOR: f32 = 1.2;
pub const AI_LOST_ENEMY_SEEK_RADIUS: i32 = 300;
pub const AI_DEAD_BODY_SEEK_RADIUS: i32 = 300;
pub const AI_SOD_DEAD_BODY_SEEK_RADIUS: i32 = 150;
pub const AI_NOISE_SEEK_RADIUS: i32 = 300;
pub const AI_STEPS_SEEK_RADIUS: i32 = 150;
pub const AI_SCRIPT_SEEK_RADIUS: i32 = 300;
pub const AI_WHISTLE_SEEK_RADIUS: i32 = 100;
pub const AI_HINT_SEEK_RADIUS: i32 = 300;
pub const AI_FIX_CHARLY_SEEK_RADIUS: i32 = 400;
pub const AI_PATROL_CHARLY_SEEK_RADIUS: i32 = 200;
pub const AI_WATCH_SEEK_RADIUS: i32 = 200;
pub const AI_MIN_LOOKFORHELPFLAG_SEEK_POINT_FACTOR: f32 = 0.1;
pub const AI_LOOK_HELP_AFTER_SEEK_ADR_LIMIT: i32 = 80;
pub const AI_LOOK_HELP_AFTER_SEEK_LAZ_LIMIT: i32 = 39;
pub const AI_MAX_SEEKED_HIT_DISTANCE: i32 = 50;
pub const AI_MAX_REFLEXSHOT_DISTANCE: i32 = 80;
/// Penalty added to seek-point distance when crossing layers.
pub const LAYER_CHANGE_PENALTY: f32 = 100.0;
/// Max distance to search for a building door the enemy could have fled through.
pub const MAX_SEARCH_ENEMY_BEHIND_DOOR_DISTANCE: u16 = 300;

// ---------------------------------------------------------------------------
// Miscellaneous AI constants
// ---------------------------------------------------------------------------

pub const AI_HELP_BOX_HALF_WIDTH: i32 = 250;
/// ≈ 250 * aspect ratio
pub const AI_HELP_BOX_HALF_HEIGHT: i32 = 140;
/// 3 vs 1
pub const AI_BEST_BATTLE_RELATION: i32 = 300;
pub const AI_BEST_BATTLE_RELATION_MINUS_100: i32 = 200;
/// 1 vs 5
pub const AI_WORST_BATTLE_RELATION: i32 = 20;
pub const AI_100_MINUS_WORST_BATTLE_RELATION: i32 = 80;
pub const AI_COVER_BONUS: i32 = 30;

pub const AI_HIT_PC_EFFECT: i32 = 5;
pub const AI_GO_SIDEWARDS_PROBABILITY: i32 = 30;
pub const AB_DEFAULT_TALK_PROBABILITY: i32 = 50;
pub const AI_STANDARD_PANIC_RUNS: i32 = 8;
pub const AI_MIN_PANIC_HIDING_TIME: i32 = 500;
pub const AI_DELTA_PANIC_HIDING_TIME: i32 = 500;
pub const AI_MIN_PANIC_RUN_SEGMENT_DISTANCE: i32 = 50;
pub const AI_DELTA_PANIC_RUN_SEGMENT_DISTANCE: i32 = 120;
pub const AI_MIN_PANIC_RUNAWAY_RADIUS: i32 = 1000;
pub const AB_MENACER_MOVES_PANIC_RUNS: i32 = 8;
pub const AB_DEAD_BODY_PANIC_RUNS: i32 = 5;
pub const AI_NEUTRAL_FRIENDINTROUBLE_PANIC_RUNS: i32 = 5;
pub const AB_HIDE_PANIC_RUNS: i32 = 5;
pub const AB_BODY_ADDITIONAL_PANIC_RUNS: i32 = 1;
pub const AM_BATTLE_PANIC_RUNS: i32 = 10;
pub const AI_MACRO_PANIC_RUNS: i32 = 7;
pub const AI_END_MENACING_PANIC_RUNS: i32 = 10;

pub const AI_MAX_HIDEANDFIGHT_ODDS: i32 = 85;

pub const INVERSE_AVERAGE_RUN_SPEED: f32 = 0.167;
pub const AI_REFLEX_SHOT_TRESHOLD: i32 = 85;

pub const MAX_BODY_VISITORS: i32 = 4;

// ---------------------------------------------------------------------------
// Tiredness constants
// ---------------------------------------------------------------------------

pub const AI_WALKING_TIREDNESS: i32 = 1;
pub const AI_RUNNING_TIREDNESS: i32 = 5;
pub const AI_SHOOTING_TIREDNESS: i32 = 5;

pub const AI_MIN_IDLING_EFFECT: i32 = 4;
pub const AI_MAX_IDLING_EFFECT: i32 = 10;
pub const AI_MIN_WAITING_EFFECT: i32 = 2;
pub const AI_MAX_WAITING_EFFECT: i32 = 5;
pub const AI_SLEEPING_EFFECT: i32 = 1;
pub const AI_STOP_SEEKING_TIREDNESS: i32 = 90;

// ---------------------------------------------------------------------------
// Stress constants
// ---------------------------------------------------------------------------

pub const AI_NOISE_BANG_BASE_STRESS: i32 = 10;
pub const AI_NOISE_BANG_INBATTLE_STRESS: i32 = 1;
pub const AI_NOISE_BANG_STRESS_FACTOR: f32 = 0.2;
pub const AI_NOISE_BOOOM_BASE_STRESS: i32 = 30;
pub const AI_NOISE_BOOOM_STRESS_FACTOR: f32 = 0.1;
pub const AI_NOISE_TAPTAPTAPTAPTAP_STRESS: i32 = 15;
pub const AI_DAZZLED_STRESS: i32 = 70;
pub const AI_IDENTIFY_PC_BASE_STRESS: i32 = 20;
pub const AI_BEEING_SEEKED_STRESS_PLUS: i32 = 40;
pub const AI_BODY_STRESS: i32 = 25;
pub const AI_MUSICAL_WATCH_STRESS: i32 = 25;
pub const AI_STONE_STRESS: i32 = 10;
pub const AI_WHISTLE_STRESS: i32 = 10;

pub const AI_SHOOTING_UNSTRESS: i32 = 5;
pub const AI_IDLING_UNSTRESS_EFFECT: i32 = 10;
pub const AI_SLEEPING_UNSTRESS_EFFECT: i32 = 10;
pub const AI_DEFAULT_UNSTRESS_EFFECT: i32 = 3;

// ---------------------------------------------------------------------------
// Interest + max curiosity thresholds
// ---------------------------------------------------------------------------

pub const AI_SHOT_INTEREST: i32 = 10;
pub const AI_SHOT_CURIOSITY_TRESHOLD: i32 = 41;
pub const AI_MAX_SHOT_CURIOSITY: i32 = 40;
pub const AI_STEPS_INTEREST: i32 = 20;
pub const AI_MAX_STEPS_CURIOSITY: i32 = 40;
pub const AI_WATCH_INTEREST: i32 = 20;
pub const AI_MAX_WATCH_CURIOSITY: i32 = 20;
pub const AI_WHISTLE_INTEREST: i32 = 20;
pub const AI_MAX_WHISTLE_CURIOSITY: i32 = 70;
pub const AI_STEPS_CURIOSITY_TRESHOLD: i32 = 30;
pub const AI_ANIMALNOISE_INTEREST: i32 = 20;
pub const AI_ANIMALNOISE_CURIOSITY_TRESHOLD: i32 = 30;
pub const AI_MAX_ANIMALNOISE_CURIOSITY: i32 = 40;

pub const MAX_SWORDFIGHT_CONSIDERATION_RADIUS: i32 = 500;

// ---------------------------------------------------------------------------
// View polygon — half aperture
// ---------------------------------------------------------------------------

pub const WIDE_HALF_APERTURE: f32 = 0.8;
pub const NORMAL_HALF_APERTURE: f32 = 0.5;
pub const LOOKTO_HALF_APERTURE: f32 = 0.3;
pub const FOCUS_HALF_APERTURE: f32 = 0.15;
pub const HALF_APERTURE_STEP: f32 = 0.10;
pub const QUICK_HALF_APERTURE_STEP: f32 = 0.3;

pub const COS_DETECTION_ZERO_ANGLE: f32 = 0.5;
/// 1 / (1 - COS_DETECTION_ZERO_ANGLE)
pub const INVERSE_ONE_MINUS_COS_DETECTION_ZERO_ANGLE: f32 = 2.0;

// ---------------------------------------------------------------------------
// View polygon — half angle range
// ---------------------------------------------------------------------------

pub const NORMAL_HALF_ANGLE_RANGE: f32 = 0.8;
pub const SEARCH_HALF_ANGLE_RANGE: f32 = 1.0;
pub const OVERVIEW_HALF_ANGLE_RANGE: f32 = 1.5;
pub const PATROL_HALF_ANGLE_RANGE: f32 = 0.8;
pub const IDLE_HALF_ANGLE_RANGE: f32 = 0.5;
pub const VERY_SMALL_HALF_ANGLE_RANGE: f32 = 0.1;

// ---------------------------------------------------------------------------
// View polygon — angle iterator steps
// ---------------------------------------------------------------------------

/// PI/80
pub const NORMAL_ANGLE_ITERATOR_STEP: f32 = 0.03927;
/// PI/500
pub const LONGRANGE_SCAN_ANGLE_ITERATOR_STEP: f32 = 0.00628;
/// PI/500
pub const SLOW_ANGLE_ITERATOR_STEP: f32 = 0.00628;
/// PI/8
pub const OVERVIEW_ANGLE_ITERATOR_STEP: f32 = std::f32::consts::FRAC_PI_8;
/// PI/20
pub const SLOW_OVERVIEW_ANGLE_ITERATOR_STEP: f32 = 0.15708;
/// PI/80
pub const PATROL_ANGLE_ITERATOR_STEP: f32 = 0.03927;
/// PI/200
pub const IDLE_ANGLE_ITERATOR_STEP: f32 = 0.01570;
/// PI/30
pub const SEARCH_ANGLE_ITERATOR_STEP: f32 = 0.10472;
/// PI/16
pub const QUICKSEARCH_ANGLE_ITERATOR_STEP: f32 = 0.19635;
/// PI/8
pub const NORMAL_ANGLE_STEP: f32 = std::f32::consts::FRAC_PI_8;
/// PI/4
pub const QUICK_ANGLE_STEP: f32 = std::f32::consts::FRAC_PI_4;
/// PI/4
pub const GLANCE_ANGLE_STEP: f32 = std::f32::consts::FRAC_PI_4;

// ---------------------------------------------------------------------------
// View polygon — misc
// ---------------------------------------------------------------------------

pub const RHANGLE_VALUE: f32 = 22.5;
pub const DAZZLE_RADIUS_STEP: i32 = 60;
pub const RHOFFSET_MAX: i32 = 16;
pub const ALPHA_START: i32 = 154;
pub const SURRENDER_ORANGE: i32 = 20;
pub const CRAZY_TREMBLE_RANGE: f32 = 0.15;
pub const CRAZY_TREMBLE_ITERATOR_STEP: f32 = 1.4;
pub const CRAZY_COLOR_ITERATOR_STEP: i32 = 1;
pub const RELATIVE_TIRED_ANGLE_DIFFERENCE: f32 = 0.7;
pub const RELATIVE_NERVOUS_SPEED_DIFFERENCE: f32 = 2.0;

// ---------------------------------------------------------------------------
// Posture view factors (multiplied with base view speed)
// ---------------------------------------------------------------------------

pub const WALK_VIEW_FACTOR: f32 = 2.0;
pub const RUN_VIEW_FACTOR: f32 = 4.0;
pub const LIE_VIEW_FACTOR: f32 = 0.25;
pub const CRAWL_VIEW_FACTOR: f32 = 0.4;
pub const SIT_VIEW_FACTOR: f32 = 0.6;
pub const GALLOP_VIEW_FACTOR: f32 = 5.0;
pub const CARRY_VIEW_FACTOR: f32 = 3.5;
pub const MISC_ACTION_VIEW_FACTOR: f32 = 2.0;

// NOTE: The DESPERADOS-specific constants (seduction, colour speeds, blinded
// behaviour, etc.) from the legacy header are intentionally omitted — they
// were already disabled in the legacy implementation.

// ---------------------------------------------------------------------------
// Character heights (Z coordinates)
// ---------------------------------------------------------------------------

pub const Z_HEAD: i32 = 30;
pub const Z_TUMMY: i32 = 17;
pub const Z_FEET: i32 = 5;
pub const Z_HEAD_ON_HORSE: i32 = 55;
pub const Z_FEET_ON_HORSE: i32 = 30;

// ---------------------------------------------------------------------------
// Effect of hits / concussion
// ---------------------------------------------------------------------------

pub const STUNNING_THRESHOLD: i32 = 40;
pub const CONCUSSION_TRESHOLD: i32 = 70;
pub const CONCUSSION_WAKEUP_TRESHOLD: i32 = 30;
pub const CONCUSSION_MAX: i32 = 300;
pub const GAS_CONCUSSION: i32 = 300;
pub const START_BLOWED_CONCUSSION: i32 = 300;
pub const START_PUNCHED_CONCUSSION: i32 = 150;
pub const START_FAINTED_CONCUSSION: i32 = 200;

pub const CIVILIAN_LIFE_POINTS: i32 = 100;

// ---------------------------------------------------------------------------
// Miscellaneous gameplay constants
// ---------------------------------------------------------------------------

pub const STANDING_AROUND_LIMIT: i32 = 10;

pub const DODGED_SHOOTING_ELEVATION: i32 = 32;
pub const DODGED_HIDING_ELEVATION: i32 = 5;

pub const MIRROR_SPOT_RADIUS: i32 = 16;
// Note: the original comment says "16 * 16" but the value is 156, not 256.
// Legacy bug preserved as-is.
pub const SQUARE_MIRROR_SPOT_RADIUS: i32 = 156;

pub const AI_DEAFNESS_MINUS: i32 = 10;
pub const AI_QUICK_DEAFNESS_MINUS: i32 = 50;
pub const AI_QUICK_DEAFNESS_RADIUS: i32 = 300;
pub const AI_NOISE_DEAFNESS_FACTOR: f32 = 0.25;

pub const AI_MIN_IQ_TO_LOOK_AROUND_INSTEAD_OF_STARE: i32 = 30;
pub const AI_MIN_IQ_TO_CONTROL_AMBUSH_POINTS: i32 = 30;

pub const AI_DOOR_RALLY_POINT_DISTANCE: i32 = 100;
pub const AI_RALLY_POINT_MIN_VILLAINS: i32 = 2;

pub const AI_BODY_IN_WATER_DETECTION_FACTOR: f32 = 0.1;

pub const AI_REQUIRED_NEARER_GUYS_TO_GO_AROUND: i32 = 2;

pub const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

pub const AI_DEBILITY_ALCOHOL_LIMIT: i32 = 1;
pub const AI_DRUNKEN_TITBIT_ALCOHOL_LIMIT: i32 = 0;

pub const AI_ONE_POINT_DEFAULT_TIME: i32 = 100;

pub const CHECK_BEGGAR_MIN_IQ: i32 = 30;
pub const CHECK_FOLLOW_INTO_HOUSE_MIN_IQ: i32 = 30;

// ---------------------------------------------------------------------------
// Hit probability tables
//
// Indexed by range bracket: 0%, 12%, 25%, 37%, 50%, 62%, 75%, 87%, 100%
// of the shooter's view range.
// ---------------------------------------------------------------------------

/// Hit probabilities for bad shooters (head+stomach visible).
pub const GREENHORN_HIT_PROB: [i32; 9] = [100, 95, 90, 70, 40, 20, 10, 0, 0];

/// Hit probabilities for good shooters (head+stomach visible).
pub const MARKSMAN_HIT_PROB: [i32; 9] = [100, 100, 100, 100, 100, 100, 100, 90, 0];

/// Hit probabilities for bad shooters (target partially in cover).
pub const GREENHORN_COVER_HIT_PROB: [i32; 9] = [100, 90, 80, 50, 20, 10, 5, 0, 0];

/// Hit probabilities for good shooters (target partially in cover).
pub const MARKSMAN_COVER_HIT_PROB: [i32; 9] = [100, 100, 100, 100, 100, 90, 80, 50, 0];

// ---------------------------------------------------------------------------
// Swordfight observation / retreat constants
// ---------------------------------------------------------------------------

/// Min distance an observer tries to maintain from a swordfight.
pub const OBSERVE_SWORDFIGHT_MIN_DISTANCE: u16 = 100;
/// Max distance an observer tries to maintain from a swordfight.
pub const OBSERVE_SWORDFIGHT_MAX_DISTANCE: u16 = 200;
/// Sideways step distance while observing a swordfight.
pub const OBSERVE_SWORDFIGHT_SIDE_STEP: f32 = 50.0;

/// Distance archers try to maintain from enemies.
pub const ARCHER_GOOD_DISTANCE: u16 = 250;
/// Minimum acceptable retreat distance for archers.
pub const ARCHER_MIN_DISTANCE: u16 = 50;

/// Minimum distance for a proud observer backing away.
pub const PROUD_OBSERVER_MIN_DISTANCE: u16 = 100;
/// Desired distance for a proud observer.
pub const PROUD_OBSERVER_GOOD_DISTANCE: u16 = 150;
/// Maximum distance before a proud observer approaches.
pub const PROUD_OBSERVER_MAX_DISTANCE: u16 = 200;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_point_sqr_radius_consistent() {
        assert_eq!(
            SEEK_POINT_MAX_SQR_RADIUS,
            SEEK_POINT_MAX_RADIUS * SEEK_POINT_MAX_RADIUS
        );
    }

    #[test]
    fn hit_probability_tables_length() {
        assert_eq!(GREENHORN_HIT_PROB.len(), 9);
        assert_eq!(MARKSMAN_HIT_PROB.len(), 9);
        assert_eq!(GREENHORN_COVER_HIT_PROB.len(), 9);
        assert_eq!(MARKSMAN_COVER_HIT_PROB.len(), 9);
    }

    #[test]
    fn hit_probability_close_range_is_100() {
        assert_eq!(GREENHORN_HIT_PROB[0], 100);
        assert_eq!(MARKSMAN_HIT_PROB[0], 100);
        assert_eq!(GREENHORN_COVER_HIT_PROB[0], 100);
        assert_eq!(MARKSMAN_COVER_HIT_PROB[0], 100);
    }

    #[test]
    fn hit_probability_max_range_is_zero() {
        assert_eq!(GREENHORN_HIT_PROB[8], 0);
        assert_eq!(MARKSMAN_HIT_PROB[8], 0);
        assert_eq!(GREENHORN_COVER_HIT_PROB[8], 0);
        assert_eq!(MARKSMAN_COVER_HIT_PROB[8], 0);
    }

    #[test]
    fn marksman_better_than_greenhorn() {
        for i in 0..9 {
            assert!(MARKSMAN_HIT_PROB[i] >= GREENHORN_HIT_PROB[i]);
            assert!(MARKSMAN_COVER_HIT_PROB[i] >= GREENHORN_COVER_HIT_PROB[i]);
        }
    }

    #[test]
    fn cover_reduces_hit_chance() {
        for i in 0..9 {
            assert!(GREENHORN_HIT_PROB[i] >= GREENHORN_COVER_HIT_PROB[i]);
            assert!(MARKSMAN_HIT_PROB[i] >= MARKSMAN_COVER_HIT_PROB[i]);
        }
    }

    #[test]
    fn key_constant_values() {
        assert_eq!(MAX_NPC_ARROWS, 20);
        assert_eq!(AI_HIDING_TIME, 3000);
        assert_eq!(AI_HINT_EXPIRANCY_TIME, 30_000);
        assert_eq!(CIVILIAN_LIFE_POINTS, 100);
        assert_eq!(CONCUSSION_MAX, 300);
        assert_eq!(NOISE_VOLUME_THUNDER, 1000);
        assert_eq!(Z_HEAD, 30);
    }

    #[test]
    fn view_factor_ordering() {
        const { assert!(LIE_VIEW_FACTOR < CRAWL_VIEW_FACTOR) };
        const { assert!(CRAWL_VIEW_FACTOR < SIT_VIEW_FACTOR) };
        const { assert!(SIT_VIEW_FACTOR < WALK_VIEW_FACTOR) };
        const { assert!(WALK_VIEW_FACTOR < RUN_VIEW_FACTOR) };
        const { assert!(RUN_VIEW_FACTOR < GALLOP_VIEW_FACTOR) };
    }

    #[test]
    fn aperture_ordering() {
        const { assert!(FOCUS_HALF_APERTURE < LOOKTO_HALF_APERTURE) };
        const { assert!(LOOKTO_HALF_APERTURE < NORMAL_HALF_APERTURE) };
        const { assert!(NORMAL_HALF_APERTURE < WIDE_HALF_APERTURE) };
    }
}
