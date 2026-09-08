import type { BoardFilters } from './state.js';
import type { BoardMetadata, Competition } from './types.js';
import { runContentDigest } from './subject-contract.js';
import { selectionFromSubject } from './view-model.js';

export function normalizeFilters(input: BoardFilters, metadata: BoardMetadata): BoardFilters {
    const selectedCompetition = input.competitionManifestSha256 === null
        ? null
        : requireCompetition(input.competitionManifestSha256, metadata.competitions);
    let subject = selectedCompetition === null
        ? input.subject
        : selectionFromSubject(selectedCompetition.subject);
    if (subject !== 'full_campaign' && metadata.missions.length === 0 && metadata.fullCampaign !== null) {
        subject = 'full_campaign';
    }
    const metric = selectedCompetition?.metric ?? input.metric;
    if (subject === 'full_campaign' && metadata.fullCampaign === null) {
        throw new Error('This server has not provisioned a full-campaign board.');
    }
    const competitionMissionId = selectedCompetition?.subject.kind === 'mission'
        ? selectedCompetition.subject.missionId
        : null;
    const missionId = subject === 'full_campaign'
        ? null
        : competitionMissionId ?? input.missionId ?? metadata.missions[0]?.id ?? null;
    const mission = missionId === null ? null : metadata.missions.find(item => item.id === missionId);
    if (subject !== 'full_campaign' && mission === undefined) {
        throw new Error('The selected mission is not published by this server.');
    }

    const compatible = metadata.rulesets.filter(ruleset => ruleset.metrics.includes(metric)
        && (subject === 'full_campaign'
            ? ruleset.supportsFullCampaign && ruleset.content.kind === 'full_campaign'
            : mission !== null && mission !== undefined
                && ruleset.content.kind === 'mission'
                && ruleset.content.contentManifestSha256 === mission.contentManifestSha256
                && ruleset.categories.includes(subject)));
    if (compatible.length === 0) throw new Error('No published ruleset supports this subject and metric.');

    const competitionRuleset = selectedCompetition === null
        ? null
        : compatible.find(ruleset => ruleset.id === selectedCompetition.rulesetId);
    if (selectedCompetition !== null) {
        if (competitionRuleset === undefined || competitionRuleset === null) {
            throw new Error('The selected challenge references a ruleset that is not published for its subject.');
        }
        if (runContentDigest(competitionRuleset.content) !== runContentDigest(selectedCompetition.content)
            || competitionRuleset.content.kind !== selectedCompetition.content.kind
            || competitionRuleset.rulesConfigSha256 !== selectedCompetition.rulesConfigSha256) {
            throw new Error('The selected challenge does not match its published content and rules configuration.');
        }
    }

    let presetId = competitionRuleset?.presetId ?? input.presetId;
    if (presetId !== null && !compatible.some(ruleset => ruleset.presetId === presetId)) {
        throw new Error('The selected preset is not available for this board.');
    }
    presetId ??= compatible.find(ruleset => ruleset.presetId === 'standard')?.presetId
        ?? compatible[0]?.presetId
        ?? null;
    const presetRules = compatible.filter(ruleset => ruleset.presetId === presetId);

    let difficultyId = competitionRuleset?.difficultyId ?? input.difficultyId;
    if (difficultyId !== null && !presetRules.some(ruleset => ruleset.difficultyId === difficultyId)) {
        throw new Error('The selected difficulty is not available for this preset.');
    }
    difficultyId ??= presetRules[0]?.difficultyId ?? null;
    const exactRules = presetRules.filter(ruleset => ruleset.difficultyId === difficultyId);

    let rulesetId = selectedCompetition?.rulesetId ?? input.rulesetId;
    if (rulesetId !== null && !exactRules.some(ruleset => ruleset.id === rulesetId)) {
        throw new Error('The selected ruleset is not available for these filters.');
    }
    rulesetId ??= exactRules[0]?.id ?? null;
    if (rulesetId === null) throw new Error('No immutable ruleset matches these filters.');
    const selectedRuleset = exactRules.find(ruleset => ruleset.id === rulesetId);
    if (selectedRuleset === undefined) throw new Error('The selected immutable ruleset is unavailable.');

    return {
        ...input,
        subject,
        metric,
        missionId,
        presetId,
        difficultyId,
        rulesetId,
        contentIdentitySha256: runContentDigest(selectedRuleset.content),
        rulesConfigSha256: selectedRuleset.rulesConfigSha256,
        competitionManifestSha256: selectedCompetition?.manifestSha256 ?? null,
        maxConcurrentPlayers: selectedCompetition === null
            ? input.maxConcurrentPlayers
            : competitionPlayerCount(selectedCompetition),
    };
}


export function requireMission(id: string | null, metadata: BoardMetadata): BoardMetadata['missions'][number] {
    const mission = metadata.missions.find(item => item.id === id);
    if (mission === undefined) throw new Error('The selected mission is not published by this server.');
    return mission;
}


export function requireCompetition(manifestSha256: string, competitions: readonly Competition[]): Competition {
    const competition = competitions.find(item => item.manifestSha256 === manifestSha256);
    if (competition === undefined) throw new Error('The selected challenge is not published by this server.');
    return competition;
}


export function competitionPlayerCount(competition: Competition): number {
    return competition.participantComposition.kind === 'single_player'
        ? 1
        : competition.participantComposition.maxConcurrentPlayers;
}


export function requireFilterIdentity(value: string | null, label: string): string {
    if (value === null) throw new Error(`The normalized board is missing its ${label} identity.`);
    return value;
}
