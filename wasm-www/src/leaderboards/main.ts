import { renderUsernameForm, renderDeletionForm, renderReportForm } from './account-forms.js';
import { compatibleRulesets, normalizeFilters, requireMission, requireCompetition } from './board-filters.js';
import { participantView, aggregateParticipantView, appendAchievements, playerTables, campaignCompositionLabel } from './public-components.js';
import { loadVerifiedRunView, loadVerifiedCampaignSession } from './run-controller.js';
import { runContentDigest } from './subject-contract.js';
import { RouteController } from './route-controller.js';
import { validateBoardView, selectionFromSubject } from './view-model.js';
import { HighscoreApi, PublicApiError } from './api.js';
import { apiBaseFromBrowser, ownerWritesArePinnedFromBrowser } from './config.js';
import { element, link, replace, statePanel } from './dom.js';
import { formatBytes, formatDate, formatMetricValue } from './format.js';
import {
    rankedSimulationPolicyDisplay,
} from './policy-display.js';
import {
    type BoardMetadata,
    type BoardMetric,
    type Competition,
    type LeaderboardEntry,
    type PlayerRunHistoryPage,
    type RunDetail,
    type RulesConfigIdentity,
    type RulesetManifest,
} from './types.js';
import {
    connectedSigningBridge,
    type LeaderboardSigningBridge,
} from './signing.js';
import {
    routeFromUrl,
    urlWithFilters,
    urlWithPlayerCursor,
    type BoardFilters,
    type PageRoute,
    type SubjectSelection,
} from './state.js';

const { renderPlayerPersonalBests, renderPlayerRunHistory } = playerTables(runLink, renderPlayerPagination);
const app = requireApp();

function requireApp(): HTMLDivElement {
    const value = document.querySelector<HTMLDivElement>('#app');
    if (value === null) throw new Error('leaderboards: missing #app element');
    return value;
}

const previousPageUrls = new Map<string, string>();
const playerPageWatermarks = new Map<string, number>();
const SUBJECT_STORAGE_KEY = 'robinhood.leaderboards.subject.v1';
const routeController = new RouteController();
window.addEventListener('pagehide', () => routeController.cancel());

window.addEventListener('popstate', () => { void renderCurrentRoute(); });
document.querySelector('[data-nav="boards"]')?.setAttribute('aria-current', 'page');
if (window.top !== window.self) {
    replace(app, statePanel(
        'Open this page directly',
        'Leaderboard identity and moderation actions are disabled while the site is embedded in another page.',
    ));
    app.setAttribute('aria-busy', 'false');
} else {
    void renderCurrentRoute();
}

async function renderCurrentRoute(): Promise<void> {
    await routeController.run(async signal => {
        renderLoading();
        const route = routeFromUrl(window.location.href, rememberedSubject());
        const api = new HighscoreApi(apiBaseFromBrowser());
        switch (route.kind) {
            case 'leaderboard': await renderLeaderboard(api, route, signal); break;
            case 'run': await renderRun(api, route.id, signal); break;
            case 'campaign_session': await renderCampaignSession(
                api,
                route.aggregateRunId,
                route.ordinal,
                signal,
            ); break;
            case 'player': await renderPlayer(
                api,
                route.publicKey,
                route.cursor,
                signal,
            ); break;
        }
    }, {
        busy: busy => app.setAttribute('aria-busy', String(busy)),
        error: renderError,
    });
}

async function renderLeaderboard(
    api: HighscoreApi,
    route: Extract<PageRoute, { readonly kind: 'leaderboard' }>,
    signal: AbortSignal,
): Promise<void> {
    const metadata = await api.metadata(signal);
    signal.throwIfAborted();
    if (metadata.rulesets.length === 0
        || (metadata.missions.length === 0 && metadata.fullCampaign === null)) {
        replace(app, pageHeading('Verified leaderboards', boardIntro()), statePanel(
            'No official boards are provisioned',
            'This server has no eligible official demo or full-retail mission and ruleset pair. It will not substitute custom, modded, or mismatched content.',
        ));
        return;
    }
    const filters = normalizeFilters(route.filters, metadata);
    rememberSubject(filters.subject);
    const normalizedUrl = urlWithFilters(window.location.href, filters);
    if (normalizedUrl !== window.location.href) window.history.replaceState(null, '', normalizedUrl);
    const [page, ruleset, rulesConfig] = await Promise.all([
        api.board(filters, signal),
        filters.rulesetId === null ? Promise.resolve(null) : api.rulesetManifest(filters.rulesetId, signal),
        filters.rulesConfigSha256 === null ? Promise.resolve(null) : api.rulesConfig(filters.rulesConfigSha256, signal),
    ]);
    signal.throwIfAborted();
    validateBoardView(page, filters, metadata, ruleset, rulesConfig);

    const heading = pageHeading('Verified leaderboards', boardIntro());
    const controls = renderFilters(metadata, filters);
    if (ruleset !== null && rulesConfig !== null) controls.append(renderRulesetPolicy(ruleset, rulesConfig));
    else controls.append(element('p', { className: 'notice', text: 'All rulesets combined. Runs with different gameplay settings and difficulties compete here; open a record to see its exact settings.' }));
    if (page.entries.length === 0) {
        replace(app, heading, controls, statePanel(
            'No verified runs on this board',
            'No verified runs match the selected mission, ruleset, competition, and player count.',
        ));
        return;
    }

    const tablePanel = element('section', { className: 'panel table-panel' });
    const tableScroll = element('div', { className: 'table-scroll' });
    const table = element('table');
    table.append(element('caption', {
        text: `${page.entries.length} server-verified run${page.entries.length === 1 ? '' : 's'} on this page`,
    }));
    const head = element('thead');
    const row = element('tr');
    for (const [label, className] of [
        ['Rank', 'rank'], ['Players', ''], [metricHeading(filters.metric), ''],
        ['Verified', 'hide-small'], ['Trust', 'hide-small'], ['Record', ''],
    ] as const) row.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
    head.append(row);
    const body = element('tbody');
    for (const entry of page.entries) body.append(renderBoardRow(entry));
    table.append(head, body);
    tableScroll.append(table);
    tablePanel.append(tableScroll, renderPagination(page.nextCursor, filters));
    replace(app, heading, controls, tablePanel);
}

function boardIntro(): string {
    return 'Official demo and full-retail runs only. Every listed score or time is backed by complete, rankable replay evidence reproduced by an allowlisted server verifier; loaded or restarted runs are excluded.';
}

function renderFilters(metadata: BoardMetadata, filters: BoardFilters): HTMLElement {
    const panel = element('section', { className: 'panel filter-panel', attrs: { 'aria-label': 'Board filters' } });
    const categories = element('div', { className: 'category-switch', attrs: { role: 'group', 'aria-label': 'Leaderboard subject' } });
    categories.append(
        categoryButton('Individual Level', 'individual_level', filters),
        categoryButton('Campaign', 'campaign', filters),
        categoryButton('Full Campaign', 'full_campaign', filters, metadata.fullCampaign === null),
    );
    const tabs = element('div', { className: 'tabs', attrs: { role: 'tablist', 'aria-label': 'Leaderboard board' } });
    tabs.append(
        metricButton('Original score', 'original_score', filters),
        metricButton('Fastest successful', 'fastest_success', filters),
    );
    for (const competition of metadata.competitions.filter(item => item.state === 'active')) {
        tabs.append(competitionButton(competition, filters));
    }

    const mission = filters.subject === 'full_campaign' ? null : requireMission(filters.missionId, metadata);
    const compatible = compatibleRulesets(metadata, filters.subject, filters.metric, mission);
    const presets = uniqueOptions(compatible.filter(item => item.presetId !== 'any').map(item => ({ id: item.presetId, label: item.presetName })));
    const presetRules = compatible.filter(item => item.presetId === filters.presetId);
    const difficulties = uniqueOptions(presetRules.map(item => ({ id: item.difficultyId, label: item.difficultyName })));
    const exactRules = presetRules.filter(item => item.difficultyId === filters.difficultyId);
    const competition = filters.competitionManifestSha256 === null
        ? null
        : requireCompetition(filters.competitionManifestSha256, metadata.competitions);

    const fields = element('div', { className: 'filters' });
    if (filters.subject !== 'full_campaign') fields.append(
        optionSelect('Mission', metadata.missions, filters.missionId, value => {
            navigateFilters({ ...filters, missionId: value, presetId: null, difficultyId: null, rulesetId: null, competitionManifestSha256: null, cursor: null });
        }),
    );
    fields.append(
        optionSelect('Preset', [{ id: '', label: 'Any ruleset (combined)' }, ...presets], filters.presetId ?? '', value => {
            navigateFilters({ ...filters, presetId: value === '' ? null : value, difficultyId: null, rulesetId: null, competitionManifestSha256: null, cursor: null });
        }),
    );
    if (filters.presetId !== null) fields.append(
        optionSelect('Difficulty', difficulties, filters.difficultyId, value => {
            navigateFilters({ ...filters, difficultyId: value, rulesetId: null, competitionManifestSha256: null, cursor: null });
        }),
        optionSelect('Ruleset / season', exactRules, filters.rulesetId, value => {
            navigateFilters({ ...filters, rulesetId: value, competitionManifestSha256: null, cursor: null });
        }),
    );
    fields.append(maxConcurrentPlayersInput(filters, competition !== null));
    panel.append(categories, tabs, fields);
    if (competition !== null) panel.append(element('p', { className: 'notice', text: competition.description }));
    else if (filters.subject === 'full_campaign' && metadata.fullCampaign !== null) {
        panel.append(element('p', { className: 'notice', text: metadata.fullCampaign.description }));
    }
    return panel;
}

function categoryButton(
    label: string,
    subject: SubjectSelection,
    filters: BoardFilters,
    disabled = false,
): HTMLButtonElement {
    const button = element('button', { text: label, attrs: { type: 'button', 'aria-pressed': String(filters.subject === subject) } });
    button.disabled = disabled;
    button.addEventListener('click', () => navigateFilters({
        ...filters,
        subject,
        missionId: subject === 'full_campaign' ? null : filters.missionId,
        presetId: null,
        difficultyId: null,
        rulesetId: null,
        competitionManifestSha256: null,
        cursor: null,
    }));
    return button;
}

function metricButton(label: string, metric: BoardMetric, filters: BoardFilters): HTMLButtonElement {
    const selected = filters.metric === metric && filters.competitionManifestSha256 === null;
    const button = element('button', {
        text: label,
        attrs: { type: 'button', role: 'tab', 'aria-selected': String(selected), tabindex: selected ? '0' : '-1' },
    });
    button.addEventListener('click', () => navigateFilters({
        ...filters, metric, presetId: null, difficultyId: null, rulesetId: null, competitionManifestSha256: null, cursor: null,
    }));
    return button;
}

function competitionButton(competition: Competition, filters: BoardFilters): HTMLButtonElement {
    const selected = filters.competitionManifestSha256 === competition.manifestSha256;
    const button = element('button', {
        text: competition.label,
        attrs: { type: 'button', role: 'tab', 'aria-selected': String(selected), tabindex: selected ? '0' : '-1' },
    });
    button.addEventListener('click', () => navigateFilters({
        ...filters,
        subject: selectionFromSubject(competition.subject),
        metric: competition.metric,
        missionId: competition.subject.kind === 'mission' ? competition.subject.missionId : null,
        presetId: null,
        difficultyId: null,
        rulesetId: competition.rulesetId,
        competitionManifestSha256: competition.manifestSha256,
        cursor: null,
    }));
    return button;
}

type Option = { readonly id: string; readonly label: string };

function optionSelect(
    labelText: string,
    options: readonly Option[],
    selected: string | null,
    changed: (value: string) => void,
): HTMLLabelElement {
    const label = element('label', { text: labelText });
    const select = element('select', { attrs: { 'aria-label': labelText } });
    for (const item of options) {
        const option = element('option', { text: item.label, attrs: { value: item.id } });
        option.selected = selected === item.id;
        select.append(option);
    }
    select.addEventListener('change', () => changed(select.value));
    label.append(select);
    return label;
}

function maxConcurrentPlayersInput(filters: BoardFilters, fixedByCompetition: boolean): HTMLLabelElement {
    const label = element('label', {
        text: fixedByCompetition ? 'Max concurrent players (Challenge)' : 'Max concurrent players (optional)',
    });
    const input = element('input', {
        attrs: { type: 'number', inputmode: 'numeric', min: '1', max: '4', step: '1', placeholder: 'Any', 'aria-label': 'Exact player count' },
    });
    input.disabled = fixedByCompetition;
    if (filters.maxConcurrentPlayers !== null) input.value = String(filters.maxConcurrentPlayers);
    input.addEventListener('change', () => {
        const count = input.value.length === 0 ? null : Number(input.value);
        if (count !== null && (!Number.isSafeInteger(count) || count < 1 || count > 4)) {
            input.setCustomValidity('Enter a whole number from 1 through 4.');
            input.reportValidity();
            return;
        }
        input.setCustomValidity('');
        navigateFilters({ ...filters, maxConcurrentPlayers: count, cursor: null });
    });
    label.append(input);
    return label;
}

function renderBoardRow(entry: LeaderboardEntry): HTMLTableRowElement {
    const row = element('tr');
    row.append(element('td', { className: 'rank', text: `#${entry.rank}` }));
    const people = element('td');
    if (entry.composition.kind === 'full_campaign') {
        for (const participant of entry.aggregateNamedParticipants) {
            people.append(aggregateParticipantView(participant, playerLink));
        }
        if (entry.anonymousParticipantInstanceCount > 0) people.append(element('span', {
            className: 'fingerprint', text: `+ ${entry.anonymousParticipantInstanceCount} anonymous instance${entry.anonymousParticipantInstanceCount === 1 ? '' : 's'}`,
        }));
    } else if (entry.namedParticipants.length === 0) {
        people.append(element('span', {
            text: `${entry.anonymousParticipantInstanceCount} anonymous participant instance${entry.anonymousParticipantInstanceCount === 1 ? '' : 's'}`,
        }));
    } else {
        for (const participant of entry.namedParticipants) people.append(participantView(participant, playerLink));
        if (entry.anonymousParticipantInstanceCount > 0) people.append(element('span', {
            className: 'fingerprint', text: `+ ${entry.anonymousParticipantInstanceCount} anonymous instance${entry.anonymousParticipantInstanceCount === 1 ? '' : 's'}`,
        }));
    }
    people.append(element('span', {
        className: 'fingerprint',
        text: `${entry.maxConcurrentPlayers} max concurrent · ${entry.participantInstanceCount} total instance${entry.participantInstanceCount === 1 ? '' : 's'}`,
    }));
    row.append(people);
    row.append(element('td', { className: 'primary-metric', text: formatMetricValue(entry.metricValue) }));
    row.append(element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }));
    row.append(element('td', { className: 'hide-small' }, [element('span', {
        className: 'badge verified',
        text: entry.composition.kind === 'full_campaign' ? '✓ Chain verified' : '✓ Replay verified',
    })]));
    const replay = element('td');
    replay.append(runLink(entry.runId, 'Details'));
    row.append(replay);
    return row;
}

async function renderPlayer(
    api: HighscoreApi,
    publicKey: string,
    cursor: string | null,
    signal: AbortSignal,
): Promise<void> {
    const page = await api.playerRuns(publicKey, cursor, signal);
    signal.throwIfAborted();
    const profile = page.player;
    const expectedWatermark = cursor === null ? undefined : playerPageWatermarks.get(window.location.href);
    if (expectedWatermark !== undefined && page.acceptedSequenceWatermark !== expectedWatermark) {
        throw new Error('The player history response changed snapshots while paging.');
    }
    const panel = element('section', { className: 'panel detail-section' });
    panel.append(
        element('span', { className: 'badge verified', text: 'Public-key identity' }),
        element('div', { className: 'metric-hero', text: profile.username }),
        definitionList([
            ['Fingerprint', profile.publicKeyFingerprint],
            ['Public key', profile.publicKey],
        ]),
        element('p', {
            className: 'notice',
            text: 'This display name is mutable and not unique. Only the owning browser identity can sign a rename. Its non-extractable private key remains in the dedicated signer origin; this page receives only the public key and typed signed document.',
        }),
    );
    const ownerBridge = await bridgeForKeys([profile.publicKey]);
    signal.throwIfAborted();
    panel.append(ownerBridge === null
        ? signerUnavailableNotice('Rename is available only to this key owner when the protected identity signer is connected.')
        : renderUsernameForm(api, profile, ownerBridge, signal));
    panel.append(renderReportForm(api, { kind: 'player', public_key: profile.publicKey }, signal));
    replace(
        app,
        pageHeading('Player identity', 'Durable accountless identity, disambiguated by its key fingerprint.'),
        panel,
        renderPlayerPersonalBests(page.personalBests),
        renderPlayerRunHistory(page, cursor),
    );
}

function renderPlayerPagination(page: PlayerRunHistoryPage, cursor: string | null): HTMLElement {
    const pagination = element('nav', {
        className: 'pagination',
        attrs: { 'aria-label': 'Player run-history pages' },
    });
    const previousUrl = previousPageUrls.get(window.location.href);
    const previous = element('button', { text: 'Previous', attrs: { type: 'button' } });
    previous.disabled = previousUrl === undefined;
    previous.addEventListener('click', () => {
        if (previousUrl !== undefined) navigate(previousUrl);
    });
    const next = element('button', { text: 'Next', attrs: { type: 'button' } });
    next.disabled = page.nextCursor === null;
    next.addEventListener('click', () => {
        if (page.nextCursor === null) return;
        if (page.nextCursor === cursor) throw new Error('Player history pagination did not advance.');
        const url = urlWithPlayerCursor(window.location.href, page.player.publicKey, page.nextCursor);
        previousPageUrls.set(url, window.location.href);
        playerPageWatermarks.set(url, page.acceptedSequenceWatermark);
        navigate(url);
    });
    pagination.append(previous, element('span', { text: '25 runs per page' }), next);
    return pagination;
}

function renderPagination(nextCursor: string | null, filters: BoardFilters): HTMLElement {
    const pagination = element('nav', { className: 'pagination', attrs: { 'aria-label': 'Leaderboard pages' } });
    const previousUrl = previousPageUrls.get(window.location.href);
    const previous = element('button', { text: 'Previous', attrs: { type: 'button' } });
    previous.disabled = previousUrl === undefined;
    previous.addEventListener('click', () => { if (previousUrl !== undefined) navigate(previousUrl); });
    const next = element('button', { text: 'Next', attrs: { type: 'button' } });
    next.disabled = nextCursor === null;
    next.addEventListener('click', () => {
        if (nextCursor === null) return;
        const url = urlWithFilters(window.location.href, { ...filters, cursor: nextCursor });
        previousPageUrls.set(url, window.location.href);
        navigate(url);
    });
    pagination.append(previous, element('span', { text: '25 runs per page' }), next);
    return pagination;
}

async function renderRun(api: HighscoreApi, id: string, signal: AbortSignal): Promise<void> {
    const { run, ruleset, rulesConfig } = await loadVerifiedRunView(api, id, signal);
    const ownerBridge = await bridgeForKeys(run.subject.kind === 'full_campaign'
        ? run.aggregateNamedParticipants.map(participant => participant.publicKey)
        : run.namedParticipants.map(participant => participant.publicKey));
    signal.throwIfAborted();
    const fullCampaign = run.subject.kind === 'full_campaign';
    const mainPanel = element('section', { className: 'panel detail-section' });
    mainPanel.append(
        element('span', {
            className: 'badge verified',
            text: fullCampaign ? '✓ Full campaign chain verified' : '✓ Server replay verified',
        }),
        element('div', { className: 'metric-hero', text: formatMetricValue(run.metricValue) }),
    );
    const participants = element('div');
    if (fullCampaign) {
        for (const participant of run.aggregateNamedParticipants) {
            participants.append(aggregateParticipantView(participant, playerLink));
        }
    } else {
        for (const participant of run.namedParticipants) participants.append(participantView(participant, playerLink));
    }
    if (run.anonymousParticipantInstanceCount > 0) participants.append(element('span', {
        className: 'fingerprint',
        text: `${run.anonymousParticipantInstanceCount} anonymous participant instance${run.anonymousParticipantInstanceCount === 1 ? '' : 's'}`,
    }));
    mainPanel.append(definitionList([
        ['Players', participants],
        ['Participation', `${run.maxConcurrentPlayers} max concurrent · ${run.participantInstanceCount} total instance${run.participantInstanceCount === 1 ? '' : 's'}`],
        ['Category', runSubjectLabel(run)],
        ['Shared rank', run.rank === null ? 'Not on a current board' : `#${run.rank}`],
        ['Verified', formatDate(run.verifiedAtUnixMs)],
        ['Replay evidence', fullCampaign
            ? `${run.fullCampaignSessions.length} ordered, independently verified session${run.fullCampaignSessions.length === 1 ? '' : 's'}`
            : formatBytes(requireMissionReplayLength(run))],
        ['Input provenance', provenanceLabel(run)],
        ['Score change', `${run.startingCampaignScore} → ${run.finalCampaignScore}`],
        ['Ransom collected', String(run.metrics.ransomCollected)],
    ]));
    if (!fullCampaign) mainPanel.append(renderMissionReplayActions(api, run.runId, run.viewer));
    mainPanel.append(element('p', { className: 'notice', text: run.trustStatement }));
    if (!fullCampaign && run.viewer?.availability.status === 'unavailable') {
        mainPanel.append(element('p', { className: 'notice', text: run.viewer.availability.safeReason }));
    }

    const proof = element('aside', { className: 'panel detail-section' });
    proof.append(element('h2', { text: 'Immutable verification record' }), definitionList([
        ['Build', run.build === null ? 'Each session records its exact build' : `${run.build.displayName} (${run.build.sourceCommit.slice(0, 12)})`],
        ['Build manifest', run.build?.manifestSha256 ?? 'See ordered sessions'],
        [run.content.kind === 'mission' ? 'Content manifest' : 'Campaign content catalog', runContentDigest(run.content)],
        ...(run.campaignContentManifestSha256 === null
            ? []
            : [['Campaign content catalog', run.campaignContentManifestSha256] as const]),
        ['Ruleset manifest', run.rulesetManifestSha256],
        ['Rules configuration', run.rulesConfigSha256],
        ['Starting campaign', run.startingCampaign.sha256],
        ['Final campaign', run.finalCampaign.sha256],
        ['Campaign composition', campaignCompositionLabel(run)],
        ['Replay', run.replay?.artifact.sha256 ?? 'No synthetic aggregate replay'],
        ['Verified result', run.publicResultSha256],
    ]));
    proof.append(renderRulesetPolicy(ruleset, rulesConfig));
    appendAchievements(proof, run.achievements);
    proof.append(renderReportForm(api, { kind: 'run', run_id: run.runId }, signal));
    proof.append(ownerBridge === null
        ? signerUnavailableNotice('Owner deletion is available only to an attributed participant with the protected identity signer.')
        : renderDeletionForm(api, ownerBridge, { kind: 'run', run_id: run.runId }, signal));
    const sections: Node[] = [
        pageHeading(runTitle(run), fullCampaign
            ? 'Every field mission and headquarters session is ordered, independently replay-verified, and cryptographically bound into this aggregate.'
            : 'Full replay, exact inputs, and server-derived result identities.'),
        element('div', { className: 'detail-grid' }, [mainPanel, proof]),
    ];
    if (fullCampaign) sections.push(renderFullCampaignSessions(api, run));
    replace(app, ...sections);
}

function runTitle(run: RunDetail): string {
    return run.subject.kind === 'full_campaign'
        ? 'Full Campaign — verified run'
        : `${run.mission?.label ?? run.subject.missionId} — verified run`;
}

function runSubjectLabel(run: RunDetail): string {
    if (run.subject.kind === 'full_campaign') return 'Full Campaign';
    return run.subject.category === 'campaign' ? 'Campaign mission' : 'Individual Level';
}

function requireMissionReplayLength(run: RunDetail): number {
    if (run.subject.kind !== 'mission' || run.replay === null) {
        throw new Error('A mission run is missing its authenticated replay length.');
    }
    return run.replay.artifact.byteLength;
}

function renderMissionReplayActions(
    api: HighscoreApi,
    runId: string,
    viewer: RunDetail['viewer'],
): HTMLElement {
    const wrapper = element('div');
    const actions = element('div', { className: 'actions' });
    const download = link('Download full replay', api.replayUrl(runId), 'button secondary');
    download.download = `${runId}.rhrec`;
    download.rel = 'noopener';
    actions.append(download);
    wrapper.append(actions);
    if (viewer === null) wrapper.append(element('p', {
        className: 'notice', text: 'The exact verified browser build is not published for this run.',
    }));
    else if (viewer.availability.status === 'available') {
        const viewerUrl = new URL('../', window.location.href);
        viewerUrl.searchParams.set('run', runId);
        actions.prepend(link('Watch verified replay', viewerUrl.toString(), 'button'));
        wrapper.append(element('p', {
            className: 'notice',
            text: viewer.availability.contentRequirement.kind === 'bundled_demo'
                ? 'Playback uses an exact pinned Demo shipping package and the authenticated browser engine.'
                : 'Playback requires a matching local Full retail v10 shipping export. Retail files stay in this browser and are never uploaded.',
        }));
    }
    return wrapper;
}

function renderFullCampaignSessions(api: HighscoreApi, run: RunDetail): HTMLElement {
    if (run.subject.kind !== 'full_campaign' || run.fullCampaignSessions.length === 0) {
        throw new Error('A Full Campaign result is missing its ordered verified sessions.');
    }
    const section = element('section', { className: 'panel detail-section' });
    section.append(
        element('h2', { text: 'Ordered verified sessions' }),
        element('p', {
            className: 'notice',
            text: 'The aggregate has no synthetic replay. Download each independently verified session in order.',
        }),
    );
    const list = element('ol', { className: 'session-list' });
    for (const session of run.fullCampaignSessions) {
        const item = element('li', { className: 'owner-action' });
        item.append(
            element('h3', { text: session.label }),
            definitionList([
                ['Type', session.kind === 'field_mission' ? 'Field mission' : 'Headquarters'],
                ['Score change', `${session.startingCampaignScore} → ${session.finalCampaignScore}`],
                ['Original score', String(session.metrics.originalScoreDelta)],
                ['Active simulation ticks', String(session.metrics.activeSimulationTicks)],
                ['Participation', `${session.maxConcurrentPlayers} max concurrent · ${session.participantInstanceCount} total instance${session.participantInstanceCount === 1 ? '' : 's'}`],
                ['Replay', session.replay.artifact.sha256],
                ['Build', `${session.build.displayName} (${session.build.sourceCommit.slice(0, 12)})`],
            ]),
            renderCampaignSessionReplayActions(api, run, session),
        );
        if (session.viewer.availability.status === 'unavailable') item.append(element('p', {
            className: 'notice', text: session.viewer.availability.safeReason,
        }));
        list.append(item);
    }
    section.append(list);
    return section;
}

async function renderCampaignSession(
    api: HighscoreApi,
    aggregateRunId: string,
    ordinal: number,
    signal: AbortSignal,
): Promise<void> {
    const { aggregate: { run: aggregate }, detail } = await loadVerifiedCampaignSession(api, aggregateRunId, ordinal, signal);
    const session = detail.session;
    const ownerBridge = await bridgeForKeys(session.namedParticipants.map(participant => participant.publicKey));
    signal.throwIfAborted();
    const panel = element('section', { className: 'panel detail-section' });
    panel.append(
        element('span', { className: 'badge verified', text: '✓ Aggregate-authorized session replay' }),
        element('div', { className: 'metric-hero', text: session.label }),
        definitionList([
            ['Aggregate', aggregate.runId],
            ['Ordinal', String(session.ordinal)],
            ['Type', session.kind === 'field_mission' ? 'Field mission' : `Headquarters #${session.headquartersSequence}`],
            ['Score change', `${session.startingCampaignScore} → ${session.finalCampaignScore}`],
            ['Active simulation ticks', String(session.metrics.activeSimulationTicks)],
            ['Replay', `${session.replay.artifact.sha256} · ${formatBytes(session.replay.artifact.byteLength)}`],
            ['Build manifest', session.build.manifestSha256],
            ['Verification result', session.publicVerificationResultSha256],
        ]),
        renderCampaignSessionReplayActions(api, aggregate, session),
        element('div', { className: 'actions' }, [runLink(aggregate.runId, 'Open Full Campaign aggregate')]),
    );
    if (session.viewer.availability.status === 'unavailable') panel.append(element('p', {
        className: 'notice', text: session.viewer.availability.safeReason,
    }));
    panel.append(ownerBridge === null
        ? signerUnavailableNotice('Owner actions are available only to an attributed participant with the protected identity signer.')
        : element('p', { className: 'notice', text: 'This replay is owned through its Full Campaign aggregate.' }));
    replace(app, pageHeading(
        `${session.label} — campaign session`,
        'This exact replay is available only through its verified Full Campaign aggregate and ordinal.',
    ), panel);
}

function renderCampaignSessionReplayActions(
    api: HighscoreApi,
    aggregate: RunDetail,
    session: RunDetail['fullCampaignSessions'][number],
): HTMLElement {
    const wrapper = element('div');
    const actions = element('div', { className: 'actions' });
    const download = link(
        'Download full replay',
        api.campaignSessionReplayUrl(aggregate.runId, session.ordinal),
        'button secondary',
    );
    download.download = `${aggregate.runId}-session-${session.ordinal}.rhrec`;
    download.rel = 'noopener';
    actions.append(download);
    if (session.viewer.availability.status === 'available') {
        const viewerUrl = new URL('../', window.location.href);
        viewerUrl.searchParams.set('run', aggregate.runId);
        viewerUrl.searchParams.set('session', String(session.ordinal));
        actions.prepend(link('Watch verified replay', viewerUrl.toString(), 'button'));
    }
    const detailUrl = new URL(window.location.href);
    detailUrl.search = '';
    detailUrl.searchParams.set('run', aggregate.runId);
    detailUrl.searchParams.set('session', String(session.ordinal));
    actions.append(link('Open session detail', detailUrl.toString(), 'button secondary'));
    wrapper.append(actions);
    if (session.viewer.availability.status === 'available') wrapper.append(element('p', {
        className: 'notice',
        text: session.viewer.availability.contentRequirement.kind === 'bundled_demo'
            ? 'Playback uses an exact pinned Demo shipping package and authenticated browser engine.'
            : 'Playback requires a matching local Full retail v10 shipping export; the files are never uploaded.',
    }));
    return wrapper;
}

async function bridgeForKeys(publicKeys: readonly string[]): Promise<LeaderboardSigningBridge | null> {
    const bridge = await connectedOwnerBridge();
    if (bridge === null) return null;
    const bridgeKey = await bridge.publicKey();
    return publicKeys.includes(bridgeKey) ? bridge : null;
}

async function connectedOwnerBridge(): Promise<LeaderboardSigningBridge | null> {
    if (!ownerWritesArePinnedFromBrowser()) return null;
    const connection = await connectedSigningBridge();
    if (connection === null) return null;
    return connection.bridge;
}

function signerUnavailableNotice(message: string): HTMLElement {
    return element('section', { className: 'notice' }, [element('p', {
        className: 'notice',
        text: `${message} Open this page directly from an official deployment with its dedicated identity signer, or use the owning game client.`,
    })]);
}

function definitionList(rows: readonly (readonly [string, string | Node])[]): HTMLDListElement {
    const list = element('dl', { className: 'definition-list' });
    for (const [term, description] of rows) list.append(
        element('dt', { text: term }), element('dd', {}, [description]),
    );
    return list;
}

function pageHeading(title: string, description: string): HTMLElement {
    return element('header', { className: 'page-heading' }, [element('h1', { text: title }), element('p', { text: description })]);
}

function renderLoading(): void {
    replace(app, element('section', { className: 'panel state-panel', attrs: { role: 'status' } }, [
        element('div', { className: 'spinner', attrs: { 'aria-hidden': 'true' } }),
        element('h1', { text: 'Loading verified records…' }),
    ]));
}

function renderError(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    const title = error instanceof PublicApiError && error.status === 404 ? 'Verified record not found' : 'Could not load leaderboards';
    const panel = element('section', { className: 'panel state-panel error', attrs: { role: 'alert' } }, [
        element('h1', { text: title }), element('p', { text: message }),
    ]);
    const retry = element('button', { className: 'button', text: 'Try again', attrs: { type: 'button' } });
    retry.addEventListener('click', () => { void renderCurrentRoute(); });
    panel.append(retry);
    replace(app, panel);
}

function runLink(runId: string, label: string): HTMLAnchorElement {
    const url = new URL(window.location.href);
    url.searchParams.delete('session');
    url.searchParams.delete('submission');
    url.searchParams.delete('player');
    url.searchParams.set('run', runId);
    const anchor = link(label, url.toString(), 'row-link');
    anchor.addEventListener('click', event => {
        if (!event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey) {
            event.preventDefault();
            navigate(anchor.href);
        }
    });
    return anchor;
}

function playerLink(publicKey: string, label: string): HTMLAnchorElement {
    const url = new URL(window.location.href);
    url.searchParams.delete('run');
    url.searchParams.delete('session');
    url.searchParams.delete('submission');
    url.searchParams.set('player', publicKey);
    const anchor = link(label, url.toString(), 'row-link');
    anchor.addEventListener('click', event => {
        if (!event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey) {
            event.preventDefault();
            navigate(anchor.href);
        }
    });
    return anchor;
}

function navigateFilters(filters: BoardFilters): void { navigate(urlWithFilters(window.location.href, filters)); }

function navigate(url: string): void {
    window.history.pushState(null, '', url);
    window.scrollTo({ top: 0, behavior: 'instant' });
    void renderCurrentRoute();
}

function renderRulesetPolicy(
    ruleset: RulesetManifest,
    rulesConfig: RulesConfigIdentity,
): HTMLElement {
    const simulationPolicy = rulesConfig.rankedSimulationPolicy;
    const section = element('section', { className: 'owner-action' });
    section.append(
        element('h2', { text: 'Published board contract' }),
        definitionList([
            ['Ruleset', ruleset.displayName],
            ['Preset / difficulty', `${ruleset.presetName} / ${ruleset.difficultyName}`],
            ['Ranked simulation policy', rankedSimulationPolicyDisplay(simulationPolicy)],
            ['Canonical start', ruleset.canonicalCampaignState.kind === 'individual_template'
                ? 'Official individual mission template'
                : 'Official full-campaign genesis'],
            ['Campaign completion', ruleset.campaignCompletionPolicy.mode === 'not_offered'
                ? 'Not offered'
                : `${ruleset.campaignCompletionPolicy.policy.terminalSubject.missionId} at ${ruleset.campaignCompletionPolicy.policy.requiredProgressionPercent}%`],
            ['Main-board seed policy', ruleset.mainBoardSeedPolicy === 'open' ? 'Open seed' : 'Server-pinned seed'],
            ['Challenge seed policy', ruleset.competitionSeedPolicy === 'open' ? 'Open or pinned by challenge' : 'Server-pinned seed'],
            ['Active time', 'Successful simulation ticks only'],
            ['Save creation', ruleset.allowSaveCreation ? 'Allowed' : 'Excluded'],
            ['Autosave', ruleset.allowAutosave ? 'Allowed' : 'Excluded'],
            ['State loading', ruleset.allowStateLoad ? 'Allowed' : 'Excluded'],
            ['Mission restart', ruleset.allowMissionRestart ? 'Allowed' : 'Excluded'],
            ['Replay schema', String(rulesConfig.replaySchemaVersion)],
            ['Exact config', `${Object.keys(rulesConfig.simConfig).length} simulation fields · ${Object.keys(rulesConfig.rules).length} ranking rules`],
        ]),
    );
    const settings = element('details');
    settings.append(element('summary', { text: 'Exact gameplay settings' }));
    settings.append(definitionList(Object.entries(rulesConfig.simConfig).map(([name, value]) => [
        name.replaceAll('_', ' '),
        typeof value === 'boolean' ? (value ? 'Enabled' : 'Disabled')
            : typeof value === 'object' ? JSON.stringify(value) : String(value),
    ])));
    section.append(settings);
    return section;
}

function rememberedSubject(): SubjectSelection {
    try {
        const value = window.localStorage.getItem(SUBJECT_STORAGE_KEY);
        if (value === 'individual_level' || value === 'campaign' || value === 'full_campaign') return value;
    } catch {
        // Storage may be disabled; the stable Individual Level default remains.
    }
    return 'individual_level';
}

function rememberSubject(subject: SubjectSelection): void {
    try {
        window.localStorage.setItem(SUBJECT_STORAGE_KEY, subject);
    } catch {
        // Navigation still works when storage is unavailable.
    }
}

function uniqueOptions(values: readonly Option[]): readonly Option[] {
    const seen = new Set<string>();
    return values.filter(value => !seen.has(value.id) && seen.add(value.id));
}

function metricHeading(metric: BoardMetric): string {
    return metric === 'original_score' ? 'Original score' : 'Active time';
}

function provenanceLabel(run: RunDetail): string {
    if (run.inputProvenance.status === 'rankable') return 'Rankable';
    return `Ineligible input: ${run.inputProvenance.taints.map(taint => taint.kind.replaceAll('_', ' ')).join(', ')}`;
}
