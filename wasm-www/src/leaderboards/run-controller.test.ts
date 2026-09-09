import assert from 'node:assert/strict';
import test from 'node:test';
import { canonicalDocumentSha256 } from './canonical.js';
import { parseRunDetail } from './public-response.js';
import { parseAndVerifyRulesConfigIdentity, parseAndVerifyRulesetManifest } from './ruleset-contract.js';
import { loadVerifiedRunView } from './run-controller.js';
import { assertRunRulesetBinding, assertAchievementPolicyBinding, assertRankedCompositionBinding } from './view-model.js';
import { missionRun, rulesConfigDocument, rulesetDocument } from './model-fixtures.js';

async function fixture() {
    const configWire = rulesConfigDocument();
    const configDigest = await canonicalDocumentSha256(configWire);
    const rulesConfig = await parseAndVerifyRulesConfigIdentity(configWire, configDigest);
    const parsed = parseRunDetail(missionRun());
    assert.equal(parsed.content.kind, 'mission');
    const policyWire = { ...rulesetDocument(configDigest), allowed_content_manifest_sha256: [
        parsed.content.kind === 'mission' ? parsed.content.contentManifestSha256 : '',
    ] };
    const policyDigest = await canonicalDocumentSha256(policyWire);
    const ruleset = await parseAndVerifyRulesetManifest(policyWire, policyDigest);
    const run = { ...parsed, rulesConfigSha256: configDigest, rulesetManifestSha256: policyDigest };
    const api = {
        run: async () => run,
        rulesConfig: async (digest: string) => { assert.equal(digest, configDigest); return rulesConfig; },
        rulesetManifest: async (digest: string) => { assert.equal(digest, policyDigest); return ruleset; },
        campaignContentManifest: async () => { throw new Error('unexpected catalog'); },
    };
    return { run, ruleset, rulesConfig, api };
}

test('run controller loads exact contracts and exposes a view only after semantic binding', async () => {
    const { api, run, ruleset } = await fixture();
    const view = await loadVerifiedRunView(api, run.runId, new AbortController().signal);
    assert.equal(view.run, run);
    assert.equal(view.ruleset, ruleset);
    await assert.rejects(loadVerifiedRunView(api, 'substituted', new AbortController().signal), /different run identity/u);
    await assert.rejects(loadVerifiedRunView({ ...api, rulesetManifest: async () => ({ ...ruleset, allowedBuildManifestSha256: [] }) }, run.runId, new AbortController().signal), /build outside/u);
});

test('superseded run loading cannot return a verified view even when the API adapter ignores cancellation', async () => {
    const { api, run } = await fixture();
    const controller = new AbortController();
    await assert.rejects(loadVerifiedRunView({ ...api, rulesConfig: async digest => {
        controller.abort(); return api.rulesConfig(digest);
    } }, run.runId, controller.signal), { name: 'AbortError' });
});

test('view contracts preserve content, metric, build, achievement and participant rejection behavior', async () => {
    const { run, ruleset, rulesConfig } = await fixture();
    assert.doesNotThrow(() => assertRunRulesetBinding(run, ruleset, rulesConfig));
    for (const policy of [
        { ...ruleset, allowedContentManifestSha256: [] },
        { ...ruleset, metrics: [] },
        { ...ruleset, boardScopes: [] },
        { ...ruleset, allowedBuildManifestSha256: [] },
        { ...ruleset, replaySchemaVersions: [] },
    ]) assert.throws(() => assertRunRulesetBinding(run, policy, rulesConfig));
    assert.throws(() => assertAchievementPolicyBinding(ruleset, [{ id: 'clean_hands', label: 'Clean Hands', evaluation: 'unverifiable' }], 'run'), /achievement decisions/u);
    assert.throws(() => assertAchievementPolicyBinding(ruleset, [], 'run'), /achievement decisions/u);
    assert.throws(() => assertRankedCompositionBinding(ruleset, 5, 5, 0, run.metricValue), /participant composition/u);
    assert.throws(() => assertRankedCompositionBinding(ruleset, 1, 1, 0, {
        metric: 'fastest_success', activeSimulationTicks: 1, tickDuration: { numeratorMicros: 1, denominator: 1 }, tickDurationMicros: 1,
    }), /tick duration/u);
});

test('only a matching canonical wire digest can produce a digest-verified contract', async () => {
    const config = rulesConfigDocument();
    const digest = await canonicalDocumentSha256(config);
    assert.equal((await parseAndVerifyRulesConfigIdentity(config, digest)).rankedSimulationPolicy.preset, 'standard');
    await assert.rejects(parseAndVerifyRulesConfigIdentity({ ...config, rules: { ...config.rules, state_load: true } }, digest), /content address/u);
});

test('custom run settings bind to an open ruleset without entering exact preset boards', async () => {
    const { run, ruleset, rulesConfig } = await fixture();
    const customRun = { ...run, rulesConfigSha256: 'f'.repeat(64) };
    const customConfig = { ...rulesConfig, rankedSimulationPolicy: { version: 1 as const, preset: 'custom' as const, difficulty: 'legendary' as const }, simConfig: { ...rulesConfig.simConfig, difficulty: 'Legendary' } };
    const open = { ...ruleset, rulesConfigConstraint: 'any_canonical_sim_config' as const, presetId: 'any', difficultyId: 'any' };
    assert.doesNotThrow(() => assertRunRulesetBinding(customRun, open, customConfig));
    assert.throws(() => assertRunRulesetBinding(customRun, ruleset, customConfig));
    assert.throws(() => assertRunRulesetBinding(customRun, { ...open, allowedContentManifestSha256: [] }, customConfig));
});
