import assert from 'node:assert/strict';
import test from 'node:test';
import { compatibleRulesets, normalizeFilters } from './board-filters.js';
import { routeFromUrl } from './state.js';
import type { BoardMetadata, Competition } from './types.js';

const digest = (n: string) => n.repeat(64);
const route = routeFromUrl('https://robinhood.phiresky.xyz/leaderboards/');
if (route.kind !== 'leaderboard') throw new Error('fixture route must select a leaderboard');
const defaults = route.filters;
const metadata: BoardMetadata = {
    missions: [{ id: 'm01', label: 'Mission', contentManifestSha256: digest('a') }], fullCampaign: null, competitions: [],
    rulesets: [{ id: digest('b'), label: 'Standard', rulesConfigSha256: digest('c'), presetId: 'standard', presetName: 'Standard',
        difficultyId: 'normal', difficultyName: 'Normal', content: { kind: 'mission', contentManifestSha256: digest('a') },
        categories: ['individual_level'], metrics: ['original_score'], supportsFullCampaign: false }],
};

test('controls and normalization share subject, metric and content compatibility', () => {
    const standard = metadata.rulesets[0]!;
    const full = { ...standard, id: digest('f'), content: { kind: 'full_campaign' as const, campaignContentManifestSha256: digest('a') }, supportsFullCampaign: true };
    const candidates: BoardMetadata = { ...metadata, fullCampaign: { label: 'Full campaign', description: '' }, rulesets: [
        standard,
        { ...standard, id: digest('d'), content: { kind: 'mission', contentManifestSha256: digest('d') } },
        { ...standard, id: digest('e'), categories: ['campaign'], metrics: ['fastest_success'] },
        full,
        { ...full, id: digest('9'), supportsFullCampaign: false },
    ] };
    const mission = metadata.missions[0]!;
    assert.deepEqual(compatibleRulesets(candidates, 'individual_level', 'original_score', mission).map(item => item.id), [standard.id]);
    assert.deepEqual(compatibleRulesets(candidates, 'campaign', 'fastest_success', mission).map(item => item.id), [digest('e')]);
    assert.deepEqual(compatibleRulesets(candidates, 'full_campaign', 'original_score', null).map(item => item.id), [full.id]);
    assert.deepEqual(compatibleRulesets(candidates, 'individual_level', 'fastest_success', mission), []);
    assert.deepEqual(compatibleRulesets(candidates, 'individual_level', 'original_score', undefined), []);
    for (const subject of ['individual_level', 'campaign', 'full_campaign'] as const) {
        for (const metric of ['original_score', 'fastest_success'] as const) {
            for (const ruleset of compatibleRulesets(candidates, subject, metric, mission)) {
                const normalized = normalizeFilters({ ...defaults, subject, metric, missionId: mission.id,
                    presetId: ruleset.presetId, difficultyId: ruleset.difficultyId, rulesetId: ruleset.id }, candidates);
                assert.equal(normalized.rulesetId, ruleset.id);
            }
        }
    }
});

test('board normalization derives exact published content/configuration instead of trusting URL identity fields', () => {
    const selected = normalizeFilters({ ...defaults, presetId: 'standard', contentIdentitySha256: digest('d'), rulesConfigSha256: digest('e') }, metadata);
    assert.equal(selected.missionId, 'm01'); assert.equal(selected.presetId, 'standard');
    assert.equal(selected.rulesetId, digest('b')); assert.equal(selected.contentIdentitySha256, digest('a'));
    assert.equal(selected.rulesConfigSha256, digest('c'));
});

test('unpublished mission, preset, difficulty, ruleset and full-campaign choices fail clearly', () => {
    for (const field of ['missionId', 'presetId', 'difficultyId', 'rulesetId'] as const) {
        assert.throws(() => normalizeFilters({ ...defaults, [field]: 'missing' }, metadata), /selected|published/u);
    }
    assert.throws(() => normalizeFilters({ ...defaults, subject: 'full_campaign' }, metadata), /not provisioned/u);
    assert.throws(() => normalizeFilters(defaults, { ...metadata, rulesets: [] }), /No published ruleset/u);
});

test('published competition fixes subject, ruleset and player count and rejects substituted content/configuration', () => {
    const competition: Competition = {
        id: 'challenge', version: 1, manifestSha256: digest('f'), label: 'Challenge', description: '',
        subject: { kind: 'mission', missionId: 'm01', category: 'individual_level' }, metric: 'original_score',
        rulesetId: digest('b'), rulesConfigSha256: digest('c'), content: { kind: 'mission', contentManifestSha256: digest('a') },
        seedPolicy: { kind: 'open' }, participantComposition: { kind: 'multiplayer', maxConcurrentPlayers: 3 },
        startsAtUnixMs: 1, endsAtUnixMs: 2, state: 'active',
    };
    const input = { ...defaults, subject: 'campaign' as const, missionId: 'ignored', competitionManifestSha256: digest('f'), maxConcurrentPlayers: 9 };
    const selected = normalizeFilters(input, { ...metadata, competitions: [competition] });
    assert.equal(selected.subject, 'individual_level'); assert.equal(selected.missionId, 'm01'); assert.equal(selected.maxConcurrentPlayers, 3);
    assert.throws(() => normalizeFilters(input, { ...metadata, competitions: [{ ...competition, rulesConfigSha256: digest('d') }] }), /does not match/u);
});

 test('boards default to all rulesets while deriving the published mission identity', () => {
    const selected = normalizeFilters({ ...defaults, contentIdentitySha256: digest('d'), rulesConfigSha256: digest('e') }, metadata);
    assert.equal(selected.missionId, 'm01');
    assert.equal(selected.contentIdentitySha256, digest('a'));
    assert.equal(selected.presetId, null);
    assert.equal(selected.rulesetId, null);
    assert.equal(selected.rulesConfigSha256, null);
});
