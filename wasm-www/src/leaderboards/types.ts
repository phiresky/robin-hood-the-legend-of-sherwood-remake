// Shared normalized protocol values. Wire input is validated by the contract modules.

export type BoardCategory = 'campaign' | 'individual_level';

export type BoardMetric = 'original_score' | 'fastest_success';

export type LeaderboardSubject =
    | { readonly kind: 'mission'; readonly missionId: string; readonly category: BoardCategory }
    | { readonly kind: 'full_campaign' };

export type RunContentIdentity =
    | { readonly kind: 'mission'; readonly contentManifestSha256: string }
    | { readonly kind: 'full_campaign'; readonly campaignContentManifestSha256: string };

export type VerifiedRunComposition =
    | { readonly kind: 'mission'; readonly replaySha256: string }
    | { readonly kind: 'full_campaign'; readonly orderedSessionRunIds: readonly string[] };

export type MissionFacet = { readonly id: string; readonly label: string; readonly contentManifestSha256: string };

export type RulesetFacet = {
    readonly id: string;
    readonly rulesConfigSha256: string;
    readonly label: string;
    readonly presetId: string;
    readonly presetName: string;
    readonly difficultyId: string;
    readonly difficultyName: string;
    readonly content: RunContentIdentity;
    readonly categories: readonly BoardCategory[];
    readonly metrics: readonly BoardMetric[];
    readonly supportsFullCampaign: boolean;
};

export type Competition = {
    readonly id: string;
    readonly version: number;
    readonly manifestSha256: string;
    readonly label: string;
    readonly description: string;
    readonly subject: LeaderboardSubject;
    readonly metric: BoardMetric;
    readonly rulesetId: string;
    readonly rulesConfigSha256: string;
    readonly content: RunContentIdentity;
    readonly seedPolicy: { readonly kind: 'open' } | {
        readonly kind: 'pinned';
        /** Canonical decimal u64. Kept as text because JavaScript numbers cannot represent every seed. */
        readonly simulationSeed: string;
    };
    readonly participantComposition:
        | { readonly kind: 'single_player'; readonly maxConcurrentPlayers: 1 }
        | { readonly kind: 'multiplayer'; readonly maxConcurrentPlayers: number };
    readonly startsAtUnixMs: number;
    readonly endsAtUnixMs: number;
    readonly state: 'upcoming' | 'active' | 'ended';
};

export type FullCampaignFacet = { readonly label: string; readonly description: string };

export type BoardMetadata = {
    readonly missions: readonly MissionFacet[];
    readonly rulesets: readonly RulesetFacet[];
    readonly competitions: readonly Competition[];
    readonly fullCampaign: FullCampaignFacet | null;
};

export type PublicParticipant = {
    readonly seat: number;
    readonly username: string;
    readonly publicKey: string;
    readonly publicKeyFingerprint: string;
};

export type AggregatePublicParticipant = {
    readonly currentDisplayName: string;
    readonly publicKey: string;
    readonly publicKeyFingerprint: string;
};

export type BoardMetricValue =
    | { readonly metric: 'original_score'; readonly points: number }
    | {
        readonly metric: 'fastest_success';
        readonly activeSimulationTicks: number;
        readonly tickDuration: TickDuration;
        readonly tickDurationMicros: number;
    };

export type TickDuration = { readonly numeratorMicros: number; readonly denominator: number };

export type RunMetrics = {
    readonly originalScoreDelta: number;
    readonly activeSimulationTicks: number;
    readonly ransomCollected: number;
};

export type LeaderboardEntry = {
    readonly position: number;
    readonly rank: number;
    readonly runId: string;
    readonly composition: VerifiedRunComposition;
    readonly metricValue: BoardMetricValue;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly namedParticipants: readonly PublicParticipant[];
    readonly aggregateNamedParticipants: readonly AggregatePublicParticipant[];
    readonly anonymousParticipantInstanceCount: number;
    readonly acceptedSequence: number;
    readonly verifiedAtUnixMs: number;
};

export type BoardPage = {
    readonly filter: {
        readonly subject: LeaderboardSubject;
        readonly metric: BoardMetric;
        readonly content: RunContentIdentity;
        readonly rulesConfigSha256: string;
        readonly rulesetManifestSha256: string;
        readonly competitionManifestSha256: string | null;
        readonly maxConcurrentPlayers: number | null;
    };
    readonly entries: readonly LeaderboardEntry[];
    readonly acceptedSequenceWatermark: number;
    readonly previousCursor: LeaderboardCursor | null;
    readonly nextCursorDocument: LeaderboardCursor | null;
    readonly nextCursor: string | null;
};

export type RunSummary = {
    readonly runId: string;
    readonly subject: LeaderboardSubject;
    readonly composition: VerifiedRunComposition;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly outcome: 'won';
    readonly metrics: RunMetrics;
    readonly content: RunContentIdentity;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
};

export type PlayerRunHistoryEntry = {
    readonly playerPublicKey: string;
    readonly run: RunSummary;
    readonly verifiedAtUnixMs: number;
};

export type PlayerRunFilter = {
    readonly subject: LeaderboardSubject;
    readonly metric: BoardMetric;
    readonly content: RunContentIdentity;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly maxConcurrentPlayers: number | null;
    readonly playerPublicKey: string;
};

export type PlayerPersonalBest = {
    readonly filter: PlayerRunFilter;
    readonly runId: string;
    readonly metricValue: BoardMetricValue;
};

export type PlayerRunHistoryPage = {
    readonly player: PlayerProfile;
    readonly acceptedSequenceWatermark: number;
    readonly runs: readonly PlayerRunHistoryEntry[];
    readonly personalBests: readonly PlayerPersonalBest[];
    readonly nextCursor: string | null;
};

export type LeaderboardOrderAnchor = {
    readonly position: number;
    readonly rank: number;
    readonly metricValue: BoardMetricValue;
    readonly acceptedSequence: number;
    readonly verifiedAtUnixMs: number;
    readonly runId: string;
};

export type LeaderboardCursor = {
    readonly querySha256: string;
    readonly acceptedSequenceWatermark: number;
    readonly last: LeaderboardOrderAnchor;
    readonly opaqueToken: string;
};

export type Achievement = {
    readonly id: string;
    readonly label: string;
    readonly evaluation: 'unverifiable' | 'not_earned' | 'earned';
};

export type InputTaintKind =
    | 'http_player_command' | 'http_simulation_step' | 'http_state_mutation'
    | 'console_command' | 'cheat_command' | 'headless_automation' | 'replay_playback'
    | 'state_load' | 'mission_restart' | 'debug_input_injection';

export type InputProvenance =
    | { readonly status: 'rankable' }
    | { readonly status: 'tainted'; readonly taints: readonly { readonly kind: InputTaintKind; readonly firstFrame: number }[] };

export type PublicBuild = { readonly manifestSha256: string; readonly sourceCommit: string; readonly displayName: string };

export type ViewerContentRequirement =
    | { readonly kind: 'bundled_demo'; readonly contentManifestSha256: string }
    | { readonly kind: 'user_local_retail'; readonly contentManifestSha256: string };

export type ViewerLaunch = {
    readonly buildManifestSha256: string;
    readonly availability:
        | { readonly status: 'available'; readonly contentRequirement: ViewerContentRequirement }
        | { readonly status: 'unavailable'; readonly safeReason: string };
};

export type PublicPlaybackProof = {
    /** Canonical verified replay artifact exposed by the public API. */
    readonly replay: ReplayArtifact;
    readonly scopeKind: 'individual_level' | 'campaign';
    readonly campaignSessionKind:
        | { readonly kind: 'field_mission'; readonly missionId: string }
        | { readonly kind: 'headquarters'; readonly hqSequence: number }
        | null;
    readonly campaignSessionOrdinal: number | null;
    /** Exact fully redacted campaign state at canonical replay genesis. */
    readonly startingCampaign: ArtifactRef;
    /** Exact fully redacted campaign reached by canonical replay resimulation. */
    readonly finalCampaign: ArtifactRef;
    readonly finalStateSha256: string;
    readonly replayFrameCount: number;
    readonly outcome: 'won';
};

export type FullCampaignSession = {
    readonly ordinal: number;
    readonly runId: string;
    readonly kind: 'field_mission' | 'headquarters';
    readonly missionId: string | null;
    readonly headquartersSequence: number | null;
    readonly contentSubject:
        | { readonly kind: 'field_mission'; readonly missionId: string }
        | { readonly kind: 'headquarters'; readonly missionId: string };
    readonly label: string;
    readonly mission: MissionFacet | null;
    readonly replay: ReplayArtifact;
    readonly replayFrameCount: number;
    readonly playbackProof: PublicPlaybackProof;
    readonly publicVerificationRequestSha256: string;
    readonly publicVerificationResultSha256: string;
    readonly startingCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly publicCampaignCompleteEvidenceSha256: string | null;
    readonly contentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly anonymousParticipantInstanceCount: number;
    /** Only participants who explicitly selected NamedProfile disclosure. */
    readonly namedParticipants: readonly PublicParticipant[];
    readonly inputProvenance: InputProvenance;
    readonly metrics: RunMetrics;
    readonly achievements: readonly Achievement[];
    readonly build: PublicBuild;
    readonly viewer: ViewerLaunch;
};

export type CampaignSessionDetail = {
    readonly aggregateRunId: string;
    readonly publicAggregateResultSha256: string;
    readonly ordinal: number;
    readonly session: FullCampaignSession;
};

export type RunDetail = {
    readonly runId: string;
    readonly rank: number | null;
    readonly subject: LeaderboardSubject;
    readonly mission: MissionFacet | null;
    readonly composition: VerifiedRunComposition;
    readonly outcome: 'won';
    readonly metrics: RunMetrics;
    readonly metricValue: BoardMetricValue;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly namedParticipants: readonly PublicParticipant[];
    readonly aggregateNamedParticipants: readonly AggregatePublicParticipant[];
    readonly anonymousParticipantInstanceCount: number;
    readonly verifiedAtUnixMs: number;
    readonly publicRequestSha256: string;
    readonly publicResultSha256: string;
    readonly replay: ReplayArtifact | null;
    readonly replayFrameCount: number | null;
    readonly playbackProof: PublicPlaybackProof | null;
    readonly content: RunContentIdentity;
    readonly campaignContentManifestSha256: string | null;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly startingCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly inputProvenance: InputProvenance;
    readonly build: PublicBuild | null;
    readonly achievements: readonly Achievement[];
    readonly viewer: ViewerLaunch | null;
    readonly fullCampaignSessions: readonly FullCampaignSession[];
    readonly trustStatement: string;
};

export type PlayerProfile = { readonly username: string; readonly publicKey: string; readonly publicKeyFingerprint: string };

export type UsernameChallenge = { readonly id: string; readonly nonce: string; readonly expiresAtUnixMs: number };

export type UsernameUpdateEnvelope = {
    readonly schema_version: 1;
    readonly username_challenge_id: string;
    readonly username_challenge_nonce: string;
    readonly public_key: string;
    readonly username: string;
    readonly signature: string;
};

export type DeletionTarget =
    | { readonly kind: 'run'; readonly run_id: string }
    | { readonly kind: 'submission'; readonly submission_id: string };

export type DeletionChallenge = {
    readonly schema_version: 1;
    readonly deletion_challenge_id: string;
    readonly deletion_challenge_nonce: string;
    readonly expires_at_unix_ms: number;
    readonly public_key: string;
    readonly target: DeletionTarget;
};

export type DeletionRequestEnvelope = { readonly schema_version: 1; readonly challenge: DeletionChallenge; readonly signature: string };

export type DeletionReceipt = {
    readonly requestId: string;
    readonly target: DeletionTarget;
    readonly tombstonedAtUnixMs: number;
    readonly purgeEligibleAtUnixMs: number | null;
};

export type AbuseReportCategory =
    | 'suspected_cheating' | 'offensive_identity' | 'privacy' | 'copyright' | 'other';

export type AbuseReportAccepted = { readonly reportId: string; readonly receivedAtUnixMs: number };

export type ArtifactRef = { readonly sha256: string; readonly byteLength: number; readonly mediaType: string };

export type ReplayArtifact = {
    readonly artifact: ArtifactRef;
    readonly replaySchemaVersion: number;
};

export type ViewerArtifactRole =
    | { readonly kind: 'entry_java_script' }
    | { readonly kind: 'java_script_module'; readonly name: string }
    | { readonly kind: 'web_assembly' }
    | { readonly kind: 'auxiliary'; readonly name: string };

export type NamedArtifact = { readonly path: string; readonly role: ViewerArtifactRole; readonly artifact: ArtifactRef };

export type SharedBuildManifest = {
    readonly sourceCommit: string;
    readonly cargoLockSha256: string;
    readonly replaySchemaVersion: number;
    readonly saveSchemaVersion: number;
    readonly networkProtocolVersion: number;
};

export type HistoricalBuildManifestV1 = SharedBuildManifest & {
    readonly schemaVersion: 1;
    readonly targetTriple: string;
    readonly cargoProfile: string;
    readonly cargoFeatures: readonly string[];
    readonly verifier: ArtifactRef;
    readonly viewerArtifacts: readonly NamedArtifact[];
};

export type RustToolchainAuthority = {
    readonly schemaVersion: 1;
    readonly channel: 'nightly-2026-08-25';
    readonly components: readonly ['rust-src', 'rustc-codegen-cranelift-preview'];
    readonly targets: readonly ['wasm32-unknown-unknown'];
};

export type BuildToolAuthority = {
    readonly version: string;
    readonly authoritySha256: string;
};

export type BrowserPagesArtifact = {
    readonly path: string;
    readonly artifact: ArtifactRef;
};

export type VerifierBuildIdentityV2 = {
    readonly platform: 'x86_64_unknown_linux_musl';
    readonly targetTriple: 'x86_64-unknown-linux-musl';
    readonly cargoProfile: 'release';
    readonly cargoFeatures: readonly [];
    readonly cargoPackage: 'robin_replay_verifier';
    readonly cargoBinary: 'robin-replay-verifier';
    readonly linkage: 'fully_static_no_interpreter_or_needed_libraries';
    readonly artifact: ArtifactRef;
};

export type BrowserViewerEngineBuildIdentityV2 = {
    readonly targetTriple: 'wasm32-unknown-unknown';
    readonly cargoProfile: 'wasm-release';
    readonly cargoFeatures: readonly ['audio'];
    readonly cargoPackage: 'robin_rs';
    readonly cargoBinary: 'robin';
    readonly recipe: 'wasm_bindgen_web_binaryen_oz_strip_debug_dwarf_wabt_strip_v1';
    readonly rustToolchain: RustToolchainAuthority;
    readonly rustToolchainSha256: string;
    readonly wasmBindgenCli: BuildToolAuthority;
    readonly binaryenWasmOpt: BuildToolAuthority;
    readonly wabtWasmStrip: BuildToolAuthority;
    readonly artifacts: readonly NamedArtifact[];
};

export type BrowserPagesShellBuildIdentityV2 = {
    readonly recipe: 'pnpm_frozen_lockfile_vite_static_shell_v1';
    readonly node: BuildToolAuthority;
    readonly pnpm: BuildToolAuthority;
    readonly packageJsonSha256: string;
    readonly pnpmLockSha256: string;
    readonly publicOriginArtifacts: readonly BrowserPagesArtifact[];
};

export type BrowserIdentitySignerBuildIdentityV2 = {
    readonly targetTriple: 'wasm32-unknown-unknown';
    readonly cargoProfile: 'wasm-release';
    readonly cargoFeatures: readonly ['identity-signer-bridge'];
    readonly cargoPackage: 'robin_rs' | 'robin_identity_signer';
    readonly cargoBinary: 'leaderboard_identity_bridge';
    readonly recipe: 'wasm_bindgen_web_separate_origin_bridge_v1';
    readonly deploymentPolicy: 'separate_allowlisted_origin_csp_frame_ancestors_and_bridge_sha_v1';
    readonly rustToolchain: RustToolchainAuthority;
    readonly rustToolchainSha256: string;
    readonly wasmBindgenCli: BuildToolAuthority;
    readonly identitySignerOriginArtifacts: readonly BrowserPagesArtifact[];
};

export type PublicBuildManifestV2 = SharedBuildManifest & {
    readonly schemaVersion: 2;
    readonly verifier: VerifierBuildIdentityV2;
    readonly viewer: {
        readonly engine: BrowserViewerEngineBuildIdentityV2;
        readonly pagesShell: BrowserPagesShellBuildIdentityV2;
        readonly identitySigner: BrowserIdentitySignerBuildIdentityV2;
    };
};

export type BuildManifest = HistoricalBuildManifestV1 | PublicBuildManifestV2;

export type BuildManifestViewerEngine = {
    readonly targetTriple: string;
    readonly cargoProfile: string;
    readonly cargoFeatures: readonly string[];
    readonly artifacts: readonly NamedArtifact[];
};

export type OfficialContentSubject =
    | { readonly kind: 'field_mission'; readonly missionId: string }
    | { readonly kind: 'headquarters'; readonly missionId: string };

export type SimulationContentComponentKind =
    | 'profiles' | 'loaded_level' | 'mission_scripts' | 'sprite_simulation_metadata'
    | 'map_geometry_metadata' | 'localized_deterministic_text' | 'sound_duration_tables'
    | 'interface_simulation_metadata';

export type ContentManifest = {
    readonly name: string;
    readonly edition: 'demo' | 'full';
    readonly subject: OfficialContentSubject;
    readonly closure: 'static_prepared_mission_content_projection';
    readonly projectionSchemaVersion: number;
    /** One canonical numeric locale directory component, e.g. `1033`. */
    readonly resourceLocaleRoot: string;
    readonly speechTiming:
        | { readonly kind: 'base_installation' }
        | { readonly kind: 'language_pack'; readonly canonicalLocale: string };
    readonly components: readonly {
        readonly kind: SimulationContentComponentKind;
        readonly componentSchemaVersion: number;
        readonly artifact: ArtifactRef;
    }[];
};

export type CampaignContentManifest = {
    readonly edition: 'demo' | 'full';
    readonly entries: readonly {
        readonly subject: OfficialContentSubject;
        readonly contentManifestSha256: string;
    }[];
};

export type CanonicalValue = null | boolean | number | string | readonly CanonicalValue[] | {
    readonly [key: string]: CanonicalValue;
};

export type RankedSimulationPolicy = {
    readonly version: 1;
    readonly preset: 'standard' | 'original_parity';
    readonly difficulty: 'easy' | 'medium' | 'hard';
};

export type RulesConfigIdentity = {
    readonly replaySchemaVersion: number;
    readonly rankedSimulationPolicy: RankedSimulationPolicy;
    readonly simConfig: Readonly<Record<string, CanonicalValue>>;
    readonly rules: Readonly<Record<string, CanonicalValue>>;
};

export type RulesetBoardScope = 'individual_level' | 'campaign_mission' | 'full_campaign';

export type CampaignCompletionPolicyRequirement =
    | { readonly mode: 'not_offered' }
    | {
        readonly mode: 'required';
        readonly policy: {
            readonly terminalSubject: OfficialContentSubject;
            readonly requiredProgressionPercent: number;
        };
    };

export type CanonicalCampaignStateRequirement = {
    readonly edition: 'demo' | 'full';
    readonly kind: 'individual_template' | 'full_campaign_genesis';
    readonly rulesConfigSha256: string;
};

export type ImmutablePolicyKind = 'input_provenance' | 'command_admission' | 'submission_admission' | 'verification';

export type ImmutablePolicyIdentity = { readonly kind: ImmutablePolicyKind; readonly version: number; readonly manifestSha256: string };

export type ParticipantEligibility = {
    readonly allowSinglePlayer: boolean;
    readonly allowMultiplayer: boolean;
    readonly namedPolicy: 'host_genesis_guest_transport_join_attestation_and_final_cosign';
    readonly anonymousPolicy: 'allowed_authenticated_but_publicly_redacted' | 'forbidden';
    readonly minimumMaxConcurrentPlayers: number;
    readonly maximumMaxConcurrentPlayers: number;
    readonly maximumParticipantInstances: number;
};

export type RulesetManifest = {
    readonly displayName: string;
    readonly presetId: string;
    readonly presetName: string;
    readonly difficultyId: string;
    readonly difficultyName: string;
    readonly rulesConfigSha256: string;
    readonly rulesConfigConstraint: 'exact_canonical_digest_only';
    readonly allowedBuildManifestSha256: readonly string[];
    readonly allowedContentManifestSha256: readonly string[];
    readonly allowedCampaignContentManifestSha256: readonly string[];
    readonly boardScopes: readonly RulesetBoardScope[];
    readonly campaignCompletionPolicy: CampaignCompletionPolicyRequirement;
    readonly metrics: readonly BoardMetric[];
    readonly metricRanking: readonly ('original_score_descending' | 'fastest_success_ascending')[];
    readonly achievementPolicies: readonly {
        readonly achievementId: string;
        readonly mode: 'required' | 'reported';
    }[];
    readonly canonicalStartPolicy: 'rules_config_bound_operator_state_and_verified_predecessor';
    readonly canonicalCampaignState: CanonicalCampaignStateRequirement;
    readonly runPreflightGrantPublicKey: string;
    readonly fullCampaignChainPolicy: 'canonical_genesis_every_field_and_headquarters_session_independent_completion';
    readonly campaignRosterContinuity: 'union_of_verified_session_subsets' | 'exact_same_authenticated_keys_every_session';
    readonly campaignAggregationConsentPolicy: 'every_authenticated_key_final_cosigns_each_session';
    readonly participantEligibility: ParticipantEligibility;
    readonly replaySchemaVersions: readonly number[];
    readonly networkProtocolVersions: readonly number[];
    readonly inputProvenancePolicy: ImmutablePolicyIdentity;
    readonly commandAdmissionPolicy: ImmutablePolicyIdentity;
    readonly submissionAdmissionPolicy: ImmutablePolicyIdentity;
    readonly verifierPolicy: ImmutablePolicyIdentity;
    readonly inputProvenanceEligibility: 'current_schema_canonical_replay_only';
    readonly terminalResultPolicy: 'independently_reached_won_only';
    readonly scoreAlgorithm: 'original_mission_attempt_wrapping_subtotal_campaign_delta_v1';
    readonly scoreOverflowPolicy: 'reject_campaign_or_aggregate_overflow';
    readonly visibleTiePolicy: 'equal_primary_metric_shares_rank';
    readonly paginationTieBreak: 'accepted_sequence_then_verification_time_then_run_id_only';
    readonly tickDuration: TickDuration;
    readonly activeTimeDefinition: 'successful_simulation_ticks';
    readonly frameCountingPolicy: 'zero_based_events_before_exclusive_replay_frame_count';
    readonly fullCampaignTimeAggregation: 'checked_sum_every_verified_field_and_headquarters_session';
    readonly runCompositionPolicy: 'mission_single_replay_full_campaign_ordered_sessions_no_synthetic_replay';
    readonly mainBoardSeedPolicy: 'open' | 'server_pinned';
    readonly competitionSeedPolicy: 'open' | 'server_pinned';
    readonly allowSaveCreation: boolean;
    readonly allowAutosave: boolean;
    readonly allowStateLoad: boolean;
    readonly allowMissionRestart: boolean;
};

export type PublishedRuleset = {
    readonly rulesetManifestSha256: string;
    readonly manifest: RulesetManifest;
    readonly operationalStatus:
        | { readonly status: 'active' }
        | { readonly status: 'quarantined'; readonly auditId: string; readonly reasonCode: string; readonly sinceUnixMs: number };
};

export type JsonObject = Readonly<Record<string, unknown>>;

export type ParsedVerificationProof = {
    readonly publicResultSha256: string;
    readonly publicRequestSha256: string;
    readonly replay: ReplayArtifact;
    readonly contentEdition: 'demo' | 'full';
    readonly buildManifestSha256: string;
    readonly contentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly inputProvenance: InputProvenance;
    readonly scopeKind: 'individual_level' | 'campaign';
    readonly campaignAggregationConsent: 'not_authorized' | 'authorize_signed_session_in_server_recognized_chain_v1';
    readonly campaignSessionKind: ParsedCampaignSessionKind | null;
    readonly campaignSessionOrdinal: number | null;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly anonymousParticipantInstanceCount: number;
    readonly replayFrameCount: number;
    readonly outcome: 'won';
    readonly startingCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly finalStateSha256: string;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly publicCampaignCompleteEvidenceSha256: string | null;
    readonly namedParticipants: readonly {
        readonly seat: number;
        readonly publicKey: string;
    }[];
    readonly achievements: readonly ParsedAchievementDecision[];
    readonly metrics: RunMetrics;
};

export type ParsedCampaignAggregate = {
    readonly publicResultSha256: string;
    readonly publicRequestSha256: string;
    readonly fullCampaignRunId: string;
    readonly campaignCompleteTerminalRunId: string;
    readonly publicCampaignCompleteEvidenceSha256: string;
    readonly sessions: readonly ParsedAggregateSession[];
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly anonymousParticipantInstanceCount: number;
    readonly campaignContentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly canonicalGenesisCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly metrics: RunMetrics;
};

export type ParsedAggregateSession = {
    readonly ordinal: number;
    readonly runId: string;
    readonly publicVerificationRequestSha256: string;
    readonly publicVerificationResultSha256: string;
};

export type ParsedCampaignSessionKind =
    | { readonly kind: 'field_mission'; readonly missionId: string }
    | { readonly kind: 'headquarters'; readonly hqSequence: number };

export type ParsedAchievementDecision = {
    readonly id: string;
    readonly evaluation: Achievement['evaluation'];
};

export type ParsedPublicVerificationRequest = {
    readonly replay: ReplayArtifact;
    readonly contentEdition: 'demo' | 'full';
    readonly contentSubject: FullCampaignSession['contentSubject'];
    readonly scopeKind: 'individual_level' | 'campaign';
    readonly campaignAggregationConsent: 'not_authorized' | 'authorize_signed_session_in_server_recognized_chain_v1';
    readonly buildManifestSha256: string;
    readonly contentManifestSha256: string;
    readonly campaignContentManifestSha256: string | null;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly anonymousParticipantInstanceCount: number;
    readonly namedParticipants: readonly {
        readonly seat: number;
        readonly publicKey: string;
    }[];
};

export type ParsedFullSession = FullCampaignSession & {
    readonly inputProvenance: InputProvenance;
    readonly contentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly verificationProof: ParsedVerificationProof;
};

export type CommonRunProof = {
    readonly runId: string;
    readonly subject: LeaderboardSubject;
    readonly composition: VerifiedRunComposition;
    readonly mission: MissionFacet | null;
    readonly metrics: RunMetrics;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly namedParticipants: readonly PublicParticipant[];
    readonly aggregateNamedParticipants: readonly AggregatePublicParticipant[];
    readonly anonymousParticipantInstanceCount: number;
    readonly publicRequestSha256: string;
    readonly publicResultSha256: string;
    readonly content: RunContentIdentity;
    readonly contentManifestSha256: string;
    readonly campaignContentManifestSha256: string | null;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly startingCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly inputProvenance: InputProvenance;
    readonly achievements: readonly Achievement[];
};

export type RunArtifacts = {
    readonly mission: MissionFacet | null;
    readonly composition: VerifiedRunComposition;
    readonly build: PublicBuild | null;
    readonly viewer: ViewerLaunch | null;
    readonly verificationProof: ParsedVerificationProof | null;
    readonly campaignAggregate: ParsedCampaignAggregate | null;
    readonly replay: ReplayArtifact | null;
    readonly fullCampaignSessions: readonly ParsedFullSession[];
};

export type ExpectedVerificationProof = {
    readonly publicResultSha256: string;
    readonly publicRequestSha256: string;
    readonly replay: ReplayArtifact;
    readonly buildManifestSha256: string;
    readonly contentManifestSha256: string;
    readonly rulesConfigSha256: string;
    readonly rulesetManifestSha256: string;
    readonly competitionManifestSha256: string | null;
    readonly inputProvenance: InputProvenance;
    readonly scopeKind: 'individual_level' | 'campaign';
    readonly missionId: string | null;
    readonly campaignSession?: ParsedCampaignSessionKind;
    readonly campaignSessionOrdinal?: number;
    readonly maxConcurrentPlayers: number;
    readonly participantInstanceCount: number;
    readonly namedParticipantInstanceCount: number;
    readonly anonymousParticipantInstanceCount: number;
    readonly namedParticipants: readonly PublicParticipant[];
    readonly startingCampaign: ArtifactRef;
    readonly finalCampaign: ArtifactRef;
    readonly startingCampaignScore: number;
    readonly finalCampaignScore: number;
    readonly publicCampaignCompleteEvidenceSha256?: string | null;
    readonly achievements: readonly Achievement[];
    readonly metrics: RunMetrics;
};
