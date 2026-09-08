//! Versioned machine-readable replay evidence. Human diagnostics are not a protocol.
use serde::{Deserialize, Serialize};

pub const RESULT_PREFIX: &str = "ROBIN_PARITY_RESULT ";

/// Bundles invoke their pinned ELF loader explicitly. On Linux current_exe
/// then identifies ld-linux, while argv[0] is the runner supplied to it.
/// Audit import still checks this claimed identity against the sealed bundle.
pub fn executable_path() -> std::path::PathBuf {
    let process_executable = std::env::current_exe().expect("resolve replay executable identity");
    if process_executable.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.starts_with("ld-linux") || name.starts_with("ld-musl")
    }) {
        let runner = std::env::args_os()
            .next()
            .expect("loader omitted runner argv[0]");
        return std::path::PathBuf::from(runner)
            .canonicalize()
            .expect("resolve bundled replay runner argv[0]");
    }
    process_executable
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionException {
    pub id: String,
    pub scope: String,
    pub removal_condition: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceCapabilities {
    pub policy_version: u32,
    pub trace_schema: u32,
    pub native_version: u32,
    pub exceptions: Vec<ProjectionException>,
}

impl TraceCapabilities {
    /// This is a description of comparison policy, never permission to accept
    /// an unsupported schema or silently drop additional fields.
    pub fn new(schema: u32, native_version: u32, missing_npc_transients: bool) -> Self {
        let mut exceptions = vec![ProjectionException {
            id: "original-runtime-representation".into(),
            scope: "Original zero-based order IDs map to Rust nonzero IDs; unset all-positive-zero blocked boxes map to null; allocation identities map by persistent rank".into(),
            removal_condition: "Retain while Original and Rust use different equivalent representations".into(),
        }];
        if schema <= 16 {
            exceptions.push(ProjectionException {
                id: "unrecorded-draw-viewport".into(),
                scope: "sprite.width, sprite.height, sprite.masked are excluded where the comparator applies the missing draw-view projection".into(),
                removal_condition: "A trace schema records the Original draw viewport and its producer timing".into(),
            });
        }
        if missing_npc_transients {
            exceptions.push(ProjectionException {
                id: "absent-initial-npc-transients".into(),
                scope: "Legacy session-boundary NPC state uses reconstruction; absence is distinct from an authoritative empty overlay".into(),
                removal_condition: "The trace header contains initial_npc_transients".into(),
            });
        }
        // Entries describe eligibility and the exact per-event condition, not
        // a claim that every exception actually occurred in this recording.
        // Keep this inventory beside the result contract: adding a projection
        // must also declare its evidence boundary and removal condition here.
        let mut register = |id: &str, scope: &str, removal: &str| {
            exceptions.push(ProjectionException {
                id: id.into(),
                scope: scope.into(),
                removal_condition: removal.into(),
            });
        };
        register(
            "background-fx",
            "compare_frame excludes entire Fx entities from logical gameplay comparison; their recorded animation is retained for diagnostics, not renderer equivalence",
            "A separate renderer-parity comparator validates retained background FX against the Original draw lifecycle",
        );
        register(
            "presentation-allocation-metadata",
            "class_id is redundant with the checked concrete entity kind; surface_id is an Original DrawManager allocation handle and is not compared",
            "A renderer/RTTI identity mapping with meaningful cross-engine semantics exists",
        );
        register(
            "persistent-rank-identity",
            "EntityMap validates persistent creation order by rank when absolute allocation counters differ; numeric gaps may differ, but persistent reordering, entity kind and bijection may not; equivalent building-sector numbers map through retained topology",
            "Both engines expose the same allocation-counter and sector-number namespace",
        );
        register(
            "runtime-recorded-subset",
            "collect_json_subset_differences checks every recorded JSON key and array length after explicit projections; extra Rust-only keys are not evidence, null/absent legacy runtime payloads are not compared, and reporting stops after 64 differences without accepting a mismatched frame",
            "A versioned common complete runtime schema replaces the recorded-subset contract",
        );
        register(
            "motion-line-identity",
            "MotionLineParity maps Original static line indices to Rust indices by layer and endpoint float-bit signature, then compares initial and per-frame active state; an unmapped line is a divergence rather than invented inactive state",
            "Both engines use the same static line indexing, making the signature mapping unnecessary",
        );
        register(
            "conversion-roundtrip-normalization",
            "The JSON/native conversion audit removes redundant float display values while preserving bits, null/empty containers, and named zero/false additive defaults before roundtrip comparison; raw source fingerprints and full native readback remain independently verified",
            "A canonical versioned JSON representation makes all these equivalent spellings identical without normalization",
        );
        if native_version == 67 && missing_npc_transients {
            register(
                "native-v67-overlay-presence",
                "The original native-v67 Vec overlay layout collapsed absent and present-empty initial_npc_transients; its empty vector decodes as missing. The later accidental v67 Option layout and v68 preserve presence",
                "Only artifacts whose native layout preserves overlay presence remain in the corpus",
            );
        }
        register(
            "optional-increment-validity",
            "compare_frame checks increment_map_valid only when the trace contains Some(bool); false remains authoritative and is distinct from a missing value",
            "Every retained element records increment_map_valid",
        );
        register(
            "scalar-float-tolerance",
            "Scalar/point summaries use abs(error) <= 1e-5 * max(abs(original),abs(rust),1); two NaNs compare equal. Runtime JSON bit fields, visibility endpoints and path requests/waypoints remain bit-exact",
            "Cross-platform scalar summaries are proven bit-exact and the comparison contract is deliberately tightened",
        );
        register(
            "position-goal-tolerance",
            "position_goal_map summary coordinates additionally allow absolute error 0.011, combined with the scalar relative tolerance; this does not relax path-event goal bits",
            "Original/Rust goal quantization is unified and original corpus proves exact coordinates",
        );
        register(
            "runtime-bonus-old-position",
            "original_runtime_bonus_has_undefined_old_position excludes old_position_map, old_elevation, movement_map, moving and moving_map summaries only for Bonus entities created at/after the runtime creation boundary; initial/save bonuses remain checked",
            "Original initializes runtime bonus/ale old-position storage or records whether each value is initialized",
        );
        register(
            "runtime-projectile-constructor-storage",
            "project_runtime_projectile_constructor_storage applies only to runtime-created Projectile entities: removes position.door_direction/goal_world/radius and sprite.flight_countdown; removes behind_display_order_reference only when display_order_reference is null; removes integer material only outside 0..=10; normalizes a point move_box to null",
            "Original initializes these constructor slots and records bounding-box validity; in-domain material and referenced display ordering must remain checked meanwhile",
        );
        register(
            "blocked-box-validity-transitions",
            "canonicalize_legacy_blocked_box reconstructs omitted bounds-set state from initial save validity and proven motion-order resets, tuple updates and anti-collision deviation; direct recorded null ends inference. Runtime actors without a saved bit remain unknown until a proving transition",
            "Every runtime snapshot directly emits null for unset blocked boxes and preserves validity independently of stale coordinate words",
        );
        register(
            "dangling-actor-animation",
            "original_actor_animation_is_logical excludes actor.animation only for entities present in this frame's late_movement_retranslations; replacement movement/path state and later actor snapshots are still compared",
            "All surviving captures use the recorder fix that avoids dereferencing the invalidated/retranslated actor order (Original 7243bed9 or newer)",
        );
        register(
            "completed-during-translation-telemetry",
            "original_actor_execution_telemetry_is_logical excludes actor.animation and actor.motion_state only when sequence_lifecycle_events contains actor_instruct_result/completed_during_translation for that creation_order",
            "The recorder exposes execution telemetry provenance or refreshes the stale fields on that exact early-return path",
        );
        register(
            "undefined-actor-motion-state",
            "original_motion_state_is_defined checks all six declared values 0..=5 (including ERROR); larger actor.motion_state values are uninitialized strangling/turning stack telemetry and are excluded",
            "Original initializes the local motionState on every strangling/turning execution path",
        );
        register(
            "pass-door-direction",
            "active_pass_door compares gate_id and direct=(direction!=0), asserting the Original flag agrees; raw nonzero direction magnitude is not a separate semantic fact",
            "A shared signed direction domain makes magnitude semantically meaningful in both implementations",
        );
        register(
            "macro-cursor-pointer-projection",
            "ai.macro_cursor is an offset only while the retained macro stream belongs to the current waypoint, is nonempty, and offset<=length; identical bytes from another waypoint do not prove pointer identity",
            "Trace and engine expose the same persistent macro-stream identity rather than pointer-range membership",
        );
        register(
            "actor-sequence-diagnostics",
            "actor.sequence_element directly compares command_name only; topology/current-order/movement fields support specific reconstruction rules, and actor.position_interface remains diagnostic. These payloads are not a complete independent sequence-manager equality check",
            "A side-effect-free Rust current-sequence/position diagnostic snapshot with Original identities is available and compared",
        );
        register(
            "visibility-cache-diagnostics",
            "compare_visibility_queries checks ordered query count, bit-exact origin/destination and boolean result; cache_hit/key/offset, candidate_count, reason and blocking_obstacle are retained diagnostics rather than cache-algorithm equivalence",
            "A separate visibility-cache/obstacle diagnostic comparator has a cross-engine identity model",
        );
        register(
            "audio-rng-domain",
            "TraceRngBatch retains all draws but gameplay replay consumes only the Simulation domain; off-main-thread simulation draws are rejected. Audio-domain draw order is not a deterministic gameplay parity claim",
            "A dedicated audio execution parity contract is defined independently of simulation RNG",
        );
        register(
            "resolved-exclamation-input",
            "resolved_exclamations supplies recorded sound completion facts/duration; selected_variant and selected_entry are diagnostics, so replay does not prove that the Rust audio mixer independently selected the same sample",
            "An audio comparator independently reproduces selection and completion timing",
        );
        register(
            "nested-selection-command-deduplication",
            "TraceCommand::into_player_command retains BoxSelect/BoxUnselect gesture metadata but executes their already-recorded nested resolved selection commands only; SelectActionIndex is a no-op unless exactly one PC is selected",
            "The recorder separates root gestures from resolved commands and the replay input contract selects one authoritative layer",
        );
        register(
            "unrecorded-selection-restitution",
            "Parity frame admission retains independently recorded command boundaries because Original records raw-mouse depth-2 messages but omits SelectPc depth-3 restitution; replay must not synthesize that omitted nested boundary twice",
            "The recorder includes explicit complete messenger nesting/restitution provenance",
        );
        register(
            "refresh-owned-orientation-order",
            "split_refresh_owned_orientations places orientation after Hourglass when preceded by matching SelectAction or proven final popup nested refresh; random-input singleton ambiguity additionally requires the exact consecutive orientation provenance state",
            "Each command records its input/ordinary-refresh/nested-refresh producer phase (including messenger no-mouse state)",
        );
        register(
            "group-move-route-constructor",
            "resolve_current_group_move_route recovers FindPathGates versus FindPathIntoDoor from retained route ordinal, actor/destination and Original sparse topology; a path ending across an overlay door alone does not select the door branch",
            "GroupMove records the exact selected route constructor and authoritative goal identity",
        );
        register(
            "drop-ale-goal-recovery",
            "resolve_current_drop_ale and delayed route recovery bind route ordinal plus actor/point and selected sector/layer; same-sector fallback requires exact geometry. A legacy missing cross-sector stream falls back to live resolution, never a fabricated same-sector identity; QA ambiguity without authoritative evidence is rejected",
            "DropAle and delayed joins record their pre-authorization goal sector/layer, route result and unique ordinal directly",
        );
        register(
            "legacy-sword-seek-distance",
            "trace_sword_seek_distance maps absent legacy seek_distance (NaN sentinel) to None and ignores distance when with_seek=false; it does not claim reconstruction of an unrecorded seek distance",
            "Every surviving seeking SwordStrike records a valid seek_distance",
        );
        if missing_npc_transients {
            register(
                "legacy-additive-fields",
                "Header absence of initial_npc_transients identifies the legacy additive generation: sprite_frame_count, actor pass-door fields, human opponents/jump-lines and AI lock/busy/macro/list/jump-line additions are not compared; modern present-empty headers remain strict. Legacy empty opponent_jump_lines permits its historical shape omission only",
                "Every surviving trace header records the additive generation explicitly and supplies all associated fields",
            );
            register(
                "legacy-route-result-and-ordinal",
                "restore_legacy_route_construction_diagnostics fills only absent ordinals from vector append position and absent results with success for the legacy generation which emitted successful constructions only; conflicting explicit ordinals remain errors",
                "All surviving captures contain explicit ordinal and success/failure outcome for each construction attempt",
            );
            register(
                "legacy-maximal-visibility",
                "apply_legacy_segment_visibility_fallback runs only on loaded-save session_index>1 with zero setup RNG prefix and missing NPC overlay; only dead/unconscious NPCs reconstruct the retained maximum from detectable visibility buckets and leaning-out speed",
                "The session-boundary header records initial_npc_transients, including the authoritative maximum",
            );
            if schema == 16 {
                register(
                    "legacy-dormant-macro-cursor",
                    "apply_legacy_interactive_chain_macro_fallback requires schema16 loaded-save, missing NPC overlay, zero setup RNG prefix and an adjacent fully terminated same-mission/proto/seed session; only uniquely identified stationary waypoint macros are restored by creation_order",
                    "The save/header records the process-local dormant macro stream identity and cursor",
                );
            }
        }
        if schema == 16 {
            register(
                "retained-terminal-success-input",
                "append_legacy_retained_terminal_success_repair applies once only when schema16 retains frame_before==frame_after, simulation_body_ran=false and LevelSucceeded: QuitMissionRequested precedes the boundary and campaign updates follow it",
                "All surviving schema16 captures explicitly record mission quitting",
            );
        }
        if (12..=16).contains(&schema) {
            register(
                "presentation-rng-homogeneous-burst",
                "legacy_presentation_sprite_rng_burst requires an ordinary in-progress simulated frame, equal callsite/value lengths, no commands, retained scrolls, a homogeneous terminal burst longer than the scroll set, previously unseen callsite and distinct suffix values; consume only when the entire exact suffix remains after Rust tick",
                "The trace records the omitted ForceRandomSpriteFrame host lifecycle/presentation boundary directly",
            );
            register(
                "presentation-rng-mobile-vibration",
                "The same burst detector admits at least six alternating X/Y suffix draws from two callsites absent in the ordinary prefix, only in an in-progress simulated frame and only when that exact suffix is unconsumed after tick",
                "The trace records the mobile-element presentation refresh and its RNG boundary directly",
            );
            register(
                "presentation-rng-teleport-stars",
                "A homogeneous ten-draw suffix with no commands is admitted only when consecutive snapshots preserve entity identity/order and show exactly one moved PC becoming active and one Target becoming inactive; callsite/value and exact-unconsumed-suffix checks still apply",
                "The trace records the ten transient teleport-star objects and their draw lifecycle directly",
            );
            register(
                "additional-falling-arrow-refreshes",
                "legacy_additional_arrow_refresh_draws requires a nonzero pending-pass draw count and a leading callsite already correlated with ArrowFallingFrame on an earlier exact frame; only leading_draws minus the pending pass is replayed",
                "The trace records each additional falling-arrow presentation pass and its producer timing",
            );
        }
        register(
            "campaign-run-identity",
            "Original has no Rust campaign-attempt nonce; replay_campaign_run_id derives a stable domain-separated identity from the recording family shared by chained sessions, not from the machine/path or current time",
            "Original traces carry an explicit durable campaign-attempt identity",
        );
        for (id, scope, removal) in [
            (
                "route-construction-event-stream",
                "route_construction_events reconstruct group-move/DropAle inputs and are printed on divergence, but the complete event stream is not independently compared",
                "Rust sequence builders publish matching ordered source/goal/gate construction events",
            ),
            (
                "popup-event-stream",
                "popup_events supplies nested-refresh evidence and diagnostics, not a complete GUI event comparison",
                "A deterministic host popup event comparator is available",
            ),
            (
                "ai-forecast-event-stream",
                "ai_forecast_events is retained for divergence inspection; forecast event ordering/content is not independently compared",
                "Rust exposes the corresponding side-effect-free AI forecast event stream",
            ),
            (
                "alert-formation-event-stream",
                "alert_formation_events is retained for divergence inspection; complete formation event equality is not independently checked",
                "Rust exposes a corresponding ordered formation event stream",
            ),
            (
                "goto-authorization-event-stream",
                "goto_authorization_events is retained diagnostic/route evidence, not an independent full authorization-event equality proof",
                "Rust exposes matching authorization decisions with stable identities",
            ),
            (
                "strike-proposal-event-stream",
                "strike_proposal_events supplies recorded opponent inputs as external facts; the runner does not independently reproduce and compare the complete proposal event stream",
                "Rust proposal capture independently emits all matching operands and decisions",
            ),
            (
                "sequence-lifecycle-event-stream",
                "sequence_lifecycle_events is shape/order-validated and supports the completed-during-translation exception, but full lifecycle stream equality is not independently checked",
                "Rust exposes a matching side-effect-free ordered sequence lifecycle stream",
            ),
            (
                "target-lifecycle-event-stream",
                "target_lifecycle_events is retained and printed on divergence, not a full independent lifecycle comparator",
                "Rust exposes matching target lifecycle events with stable identities",
            ),
            (
                "movement-step-event-stream",
                "movement_steps may be absent from early/disabled recordings; Original and Rust movement steps are captured in diagnostic dumps, not independently compared by the EOF gate",
                "Both recorders require and compare the ordered motion-position commit stream",
            ),
            (
                "flight-step-event-stream",
                "flight_steps may be absent from early/disabled recordings; flight and move-box extraction diagnostics are retained in dumps, not independently compared by the EOF gate",
                "Both recorders require and compare the ordered flight execution and move-box streams",
            ),
        ] {
            register(id, scope, removal);
        }
        Self {
            policy_version: 1,
            trace_schema: schema,
            native_version,
            exceptions,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDivergence {
    pub field: String,
    pub frame: u64,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayResult {
    pub result_version: u32,
    pub trace_path: String,
    pub native_trace_sha256: String,
    pub executable_path: String,
    pub executable_sha256: String,
    pub expected_frames: u64,
    pub processed_frames: u64,
    pub expected_final_frame: u64,
    pub final_frame: u64,
    pub terminator_validated: bool,
    pub divergent_frames: u64,
    pub first_divergences: Vec<FieldDivergence>,
    pub outcome: String,
    pub capabilities: TraceCapabilities,
}

impl ReplayResult {
    pub fn publish(&self) {
        let json = serde_json::to_string(self).expect("serialize parity result");
        // The existing audit log is already atomically published and sealed by
        // orchestration. Put the result inside that artifact, preserving those
        // integrity and restart guarantees instead of adding a detached sidecar.
        println!("{RESULT_PREFIX}{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_exception_is_scoped_to_schemas_without_draw_view() {
        for schema in [12, 16] {
            assert!(
                TraceCapabilities::new(schema, 68, false)
                    .exceptions
                    .iter()
                    .any(|exception| exception.id == "unrecorded-draw-viewport")
            );
        }
        assert!(
            !TraceCapabilities::new(17, 68, false)
                .exceptions
                .iter()
                .any(|exception| exception.id == "unrecorded-draw-viewport")
        );
    }

    #[test]
    fn authoritative_empty_overlay_does_not_admit_legacy_reconstruction() {
        assert!(
            !TraceCapabilities::new(16, 68, false)
                .exceptions
                .iter()
                .any(|exception| exception.id == "absent-initial-npc-transients")
        );
        assert!(
            TraceCapabilities::new(16, 68, true)
                .exceptions
                .iter()
                .any(|exception| exception.id == "absent-initial-npc-transients")
        );
    }

    #[test]
    fn event_policy_eligibility_tracks_schema_and_header_generation() {
        let has = |schema, missing, id: &str| {
            TraceCapabilities::new(schema, 68, missing)
                .exceptions
                .iter()
                .any(|rule| rule.id == id)
        };
        for schema in [12, 13, 14, 15, 16] {
            assert!(has(schema, true, "legacy-additive-fields"));
            assert!(has(schema, true, "legacy-route-result-and-ordinal"));
            assert!(!has(schema, false, "legacy-additive-fields"));
            assert!(!has(schema, false, "legacy-maximal-visibility"));
            assert!(has(schema, false, "presentation-rng-teleport-stars"));
            assert_eq!(
                has(schema, true, "legacy-dormant-macro-cursor"),
                schema == 16
            );
            assert_eq!(
                has(schema, false, "retained-terminal-success-input"),
                schema == 16
            );
        }
        for id in [
            "presentation-rng-homogeneous-burst",
            "presentation-rng-mobile-vibration",
            "presentation-rng-teleport-stars",
            "additional-falling-arrow-refreshes",
        ] {
            assert!(!has(17, false, id), "future schema inherited {id}");
        }
    }

    #[test]
    fn every_diagnostic_event_stream_has_an_independent_limit_and_removal_contract() {
        let capabilities = TraceCapabilities::new(16, 68, false);
        for id in [
            "route-construction-event-stream",
            "popup-event-stream",
            "ai-forecast-event-stream",
            "alert-formation-event-stream",
            "goto-authorization-event-stream",
            "strike-proposal-event-stream",
            "sequence-lifecycle-event-stream",
            "target-lifecycle-event-stream",
            "movement-step-event-stream",
            "flight-step-event-stream",
        ] {
            let rule = capabilities
                .exceptions
                .iter()
                .find(|rule| rule.id == id)
                .unwrap_or_else(|| panic!("unreported evidence boundary: {id}"));
            assert!(!rule.scope.is_empty());
            assert!(!rule.removal_condition.is_empty());
        }
    }

    #[test]
    fn policy_ids_are_unique_and_survive_result_json_roundtrip() {
        for missing in [false, true] {
            let capabilities = TraceCapabilities::new(16, 68, missing);
            let encoded = serde_json::to_string(&capabilities).unwrap();
            let decoded: TraceCapabilities = serde_json::from_str(&encoded).unwrap();
            let mut ids = std::collections::BTreeSet::new();
            for rule in decoded.exceptions {
                assert!(ids.insert(rule.id.clone()), "duplicate policy {}", rule.id);
                assert!(!rule.scope.is_empty());
                assert!(!rule.removal_condition.is_empty());
            }
            assert_eq!(ids.len(), capabilities.exceptions.len());
        }
    }
}
