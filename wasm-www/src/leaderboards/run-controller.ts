import type { HighscoreApi } from './api.js';
import type { DigestVerified } from './canonical.js';
import type { RunDetail, RulesetManifest, RulesConfigIdentity, CampaignSessionDetail } from './types.js';
import {
    campaignContentCatalogDigest,
    validateRunCampaignContentBinding,
    validateCampaignSessionDetailBinding,
} from './public-response.js';
import { assertRunRulesetBinding } from './view-model.js';

type RunApi = Pick<HighscoreApi, 'run' | 'rulesetManifest' | 'rulesConfig' | 'campaignContentManifest'>;
declare const boundRun: unique symbol;
export type VerifiedRunView = {
    readonly run: RunDetail;
    readonly ruleset: DigestVerified<RulesetManifest>;
    readonly rulesConfig: DigestVerified<RulesConfigIdentity>;
    readonly [boundRun]: true;
};

/** Fetch and bind the exact immutable contracts before exposing a run to rendering. */
export async function loadVerifiedRunView(api: RunApi, id: string, signal: AbortSignal): Promise<VerifiedRunView> {
    const run = await api.run(id, signal);
    signal.throwIfAborted();
    if (run.runId !== id) throw new Error('The server returned a different run identity.');
    const catalogDigest = campaignContentCatalogDigest(run);
    const [ruleset, rulesConfig, campaignContent] = await Promise.all([
        api.rulesetManifest(run.rulesetManifestSha256, signal),
        api.rulesConfig(run.rulesConfigSha256, signal),
        catalogDigest === null ? Promise.resolve(null) : api.campaignContentManifest(catalogDigest, signal),
    ]);
    signal.throwIfAborted();
    assertRunRulesetBinding(run, ruleset, rulesConfig);
    validateRunCampaignContentBinding(run, campaignContent);
    return { run, ruleset, rulesConfig } as VerifiedRunView;
}

export async function loadVerifiedCampaignSession(
    api: RunApi & Pick<HighscoreApi, 'campaignSession'>,
    aggregateRunId: string,
    ordinal: number,
    signal: AbortSignal,
): Promise<{ readonly aggregate: VerifiedRunView; readonly detail: CampaignSessionDetail }> {
    const [aggregate, detail] = await Promise.all([
        loadVerifiedRunView(api, aggregateRunId, signal),
        api.campaignSession(aggregateRunId, ordinal, signal),
    ]);
    signal.throwIfAborted();
    validateCampaignSessionDetailBinding(aggregate.run, detail);
    return { aggregate, detail };
}
