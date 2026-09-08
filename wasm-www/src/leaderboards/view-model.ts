import type { BoardMetadata, BoardPage, RunDetail, RulesConfigIdentity, RulesetManifest, Competition } from './types.js';
import type { BoardFilters, SubjectSelection } from './state.js';
import { runContentDigest } from './subject-contract.js';
import { validateRulesetFacetScopeBinding, validateRulesetRulesConfigBinding } from './ruleset-contract.js';

export function assertRulesetBinding(
    ruleset: RulesetManifest,
    filters: BoardFilters,
    metadata: BoardMetadata,
): void {
    const facet = metadata.rulesets.find(item => item.id === filters.rulesetId);
    if (facet === undefined) throw new Error('The selected ruleset facet is no longer published.');
    const expectedScopes = [
        ...(facet.categories.includes('individual_level') ? ['individual_level' as const] : []),
        ...(facet.categories.includes('campaign') ? ['campaign_mission' as const] : []),
        ...(facet.supportsFullCampaign ? ['full_campaign' as const] : []),
    ];
    if (ruleset.displayName !== facet.label
        || ruleset.presetId !== facet.presetId
        || ruleset.presetName !== facet.presetName
        || ruleset.difficultyId !== facet.difficultyId
        || ruleset.difficultyName !== facet.difficultyName
        || ruleset.rulesConfigSha256 !== facet.rulesConfigSha256
        || facet.content.kind !== (filters.subject === 'full_campaign' ? 'full_campaign' : 'mission')
        || runContentDigest(facet.content) !== filters.contentIdentitySha256
        || !rulesetAllowsContent(ruleset, facet.content)
        || !sameStrings(ruleset.metrics, facet.metrics)
        || !ruleset.metrics.includes(filters.metric)
        || !ruleset.boardScopes.includes(subjectRulesetScope(filters.subject))) {
        throw new Error('The immutable ruleset manifest does not match its published board facet.');
    }
    validateRulesetFacetScopeBinding(ruleset, expectedScopes);
}

export function assertRunRulesetBinding(
    run: RunDetail,
    ruleset: RulesetManifest,
    rulesConfig: RulesConfigIdentity,
): void {
    if (ruleset.rulesConfigSha256 !== run.rulesConfigSha256
        || !rulesetAllowsContent(ruleset, run.content)
        || !ruleset.metrics.includes(run.metricValue.metric)
        || !ruleset.boardScopes.includes(subjectRulesetScope(
            run.subject.kind === 'full_campaign' ? 'full_campaign' : run.subject.category,
        ))) {
        throw new Error('The run proof does not match its immutable ruleset manifest.');
    }
    validateRulesetRulesConfigBinding(ruleset, rulesConfig);
    assertRankedCompositionBinding(
        ruleset,
        run.maxConcurrentPlayers,
        run.participantInstanceCount,
        run.anonymousParticipantInstanceCount,
        run.metricValue,
    );
    if (run.subject.kind === 'mission') {
        assertAchievementPolicyBinding(ruleset, run.achievements, 'run');
        if (run.build === null || !ruleset.allowedBuildManifestSha256.includes(run.build.manifestSha256)) {
            throw new Error('The mission run uses a build outside its immutable ruleset allowlist.');
        }
    } else {
        for (const session of run.fullCampaignSessions) {
            if (!ruleset.allowedBuildManifestSha256.includes(session.build.manifestSha256)
                || !ruleset.replaySchemaVersions.includes(session.replay.replaySchemaVersion)) {
                throw new Error('A campaign session uses a build or replay schema outside its immutable ruleset.');
            }
            assertAchievementPolicyBinding(ruleset, session.achievements, 'campaign session');
            assertRankedCompositionBinding(
                ruleset,
                session.maxConcurrentPlayers,
                session.participantInstanceCount,
                session.anonymousParticipantInstanceCount,
                run.metricValue,
            );
        }
        // Exact authenticated-roster continuity is verified server-side. The public projection
        // deliberately exposes only participants who opted into named disclosure.
    }
}

export function assertAchievementPolicyBinding(
    ruleset: RulesetManifest,
    achievements: readonly RunDetail['achievements'][number][],
    label: string,
): void {
    if (achievements.length !== ruleset.achievementPolicies.length
        || achievements.some((achievement, index) => {
            const policy = ruleset.achievementPolicies[index];
            return policy === undefined || achievement.id !== policy.achievementId
                || (policy.mode === 'required' && achievement.evaluation === 'unverifiable');
        })) {
        throw new Error(`The ${label} achievement decisions do not match the immutable ruleset catalog.`);
    }
}

export function rulesetAllowsContent(ruleset: RulesetManifest, content: RunDetail['content']): boolean {
    return content.kind === 'mission'
        ? ruleset.allowedContentManifestSha256.includes(content.contentManifestSha256)
        : ruleset.allowedCampaignContentManifestSha256.includes(content.campaignContentManifestSha256);
}

export function assertRankedCompositionBinding(
    ruleset: RulesetManifest,
    maxConcurrentPlayers: number,
    participantInstanceCount: number,
    anonymousParticipantInstanceCount: number,
    metricValue: RunDetail['metricValue'],
): void {
    const eligibility = ruleset.participantEligibility;
    if (maxConcurrentPlayers < eligibility.minimumMaxConcurrentPlayers
        || maxConcurrentPlayers > eligibility.maximumMaxConcurrentPlayers
        || participantInstanceCount > eligibility.maximumParticipantInstances
        || (!eligibility.allowSinglePlayer && maxConcurrentPlayers === 1)
        || (!eligibility.allowMultiplayer && maxConcurrentPlayers > 1)
        || (eligibility.anonymousPolicy === 'forbidden' && anonymousParticipantInstanceCount !== 0)) {
        throw new Error('The ranked participant composition is outside its immutable ruleset policy.');
    }
    if (metricValue.metric === 'fastest_success'
        && (metricValue.tickDuration.numeratorMicros !== ruleset.tickDuration.numeratorMicros
            || metricValue.tickDuration.denominator !== ruleset.tickDuration.denominator)) {
        throw new Error('The ranked time metric uses a different immutable tick duration.');
    }
}

export function subjectRulesetScope(subject: SubjectSelection): RulesetManifest['boardScopes'][number] {
    if (subject === 'campaign') return 'campaign_mission';
    return subject;
}

export function sameStrings(left: readonly string[], right: readonly string[]): boolean {
    return left.length === right.length && left.every((value, index) => value === right[index]);
}

export function selectionFromSubject(subject: Competition['subject']): SubjectSelection {
    return subject.kind === 'full_campaign' ? 'full_campaign' : subject.category;
}

export function subjectMatchesFilters(subject: Competition['subject'], filters: BoardFilters): boolean {
    return selectionFromSubject(subject) === filters.subject
        && (subject.kind === 'full_campaign' || subject.missionId === filters.missionId);
}

export function validateBoardView(page: BoardPage, filters: BoardFilters, metadata: BoardMetadata,
    ruleset: RulesetManifest, rulesConfig: RulesConfigIdentity): void {
    assertRulesetBinding(ruleset, filters, metadata);
    validateRulesetRulesConfigBinding(ruleset, rulesConfig);
    if (!subjectMatchesFilters(page.filter.subject, filters)
        || page.filter.metric !== filters.metric
        || page.filter.content.kind !== (filters.subject === 'full_campaign' ? 'full_campaign' : 'mission')
        || runContentDigest(page.filter.content) !== filters.contentIdentitySha256
        || page.filter.rulesConfigSha256 !== filters.rulesConfigSha256
        || page.filter.rulesetManifestSha256 !== filters.rulesetId
        || page.filter.competitionManifestSha256 !== filters.competitionManifestSha256
        || page.filter.maxConcurrentPlayers !== filters.maxConcurrentPlayers) {
        throw new Error('The server returned a leaderboard for different filters.');
    }
    for (const entry of page.entries) {
        if (entry.metricValue.metric !== filters.metric) {
            throw new Error('The server returned an entry for a different leaderboard metric.');
        }
        assertRankedCompositionBinding(
            ruleset,
            entry.maxConcurrentPlayers,
            entry.participantInstanceCount,
            entry.anonymousParticipantInstanceCount,
            entry.metricValue,
        );
    }

}
