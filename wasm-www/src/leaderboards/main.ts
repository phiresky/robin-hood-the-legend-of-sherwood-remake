import { renderUsernameForm, renderDeletionForm, renderReportForm } from './account-forms.js';
import { boardFacets, boardForFacet, filtersForBoard, normalizeFilters, type NormalizedBoardFilters } from './board-filters.js';
import {
    appendAchievements,
    participationLabel,
    playerTables,
    uploaderView,
} from './public-components.js';
import { RouteController } from './route-controller.js';
import { submissionPresentation } from './submission-status.js';
import { boardPolicyLabel, runLabels, validateBoardView } from './view-model.js';
import { HighscoreApi, PublicApiError } from './api.js';
import { apiBaseFromBrowser, ownerWritesArePinnedFromBrowser } from './config.js';
import { element, link, replace, statePanel } from './dom.js';
import {
    editionLabel,
    formatActiveTime,
    formatBytes,
    formatDate,
    formatInteger,
    formatMetricValue,
    metricLabel,
} from './format.js';
import {
    type Board,
    type BoardMetadata,
    type BoardMetric,
    type LeaderboardEntry,
    type PlayerRunHistoryPage,
    type RunDetail,
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
} from './state.js';

const app = requireApp();

function requireApp(): HTMLDivElement {
    const value = document.querySelector<HTMLDivElement>('#app');
    if (value === null) throw new Error('leaderboards: missing #app element');
    return value;
}

const previousPageUrls = new Map<string, string>();
const playerPageWatermarks = new Map<string, number>();
const BOARD_STORAGE_KEY = 'robinhood.leaderboards.board.v2';
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
        const route = routeFromUrl(window.location.href);
        const api = new HighscoreApi(apiBaseFromBrowser());
        switch (route.kind) {
            case 'submission': await renderSubmission(api, route.id, signal); break;
            case 'leaderboard': await renderLeaderboard(api, route, signal); break;
            case 'run': await renderRun(api, route.id, signal); break;
            case 'player': await renderPlayer(api, route.publicKey, route.cursor, signal); break;
        }
    }, {
        busy: busy => app.setAttribute('aria-busy', String(busy)),
        error: renderError,
    });
}

async function renderSubmission(api: HighscoreApi, id: string, signal: AbortSignal): Promise<void> {
    const status = await api.submissionStatus(id, signal);
    signal.throwIfAborted();
    const view = submissionPresentation(status);
    const panel = statePanel(view.title, view.message);
    panel.append(element('p', { className: 'fingerprint', text: `Submission ${id}` }));
    if (status.runId !== null) panel.append(runLink(status.runId, 'View verified result'));
    replace(app, pageHeading('Replay submission', 'A received replay stays unverified until the server confirms it.'), panel);
    if (view.pending) scheduleSubmissionRefresh(api, id, signal);
}

function scheduleSubmissionRefresh(api: HighscoreApi, id: string, signal: AbortSignal): void {
    const cancel = (): void => window.clearTimeout(timer);
    const timer = window.setTimeout(() => {
        signal.removeEventListener('abort', cancel);
        if (!signal.aborted) void renderSubmission(api, id, signal).catch(error => {
            if (signal.aborted) return;
            renderError(error);
            // A temporary network failure must not leave a pending result frozen.
            scheduleSubmissionRefresh(api, id, signal);
        });
    }, 3000);
    signal.addEventListener('abort', cancel, { once: true });
}

async function renderLeaderboard(
    api: HighscoreApi,
    route: Extract<PageRoute, { readonly kind: 'leaderboard' }>,
    signal: AbortSignal,
): Promise<void> {
    const metadata = await api.metadata(signal);
    signal.throwIfAborted();
    if (metadata.boards.length === 0) {
        replace(app, pageHeading('Verified leaderboards', boardIntro()), statePanel(
            'No official boards are provisioned',
            'This server publishes no ranked Demo or Full boards yet.',
        ));
        return;
    }
    const filters = normalizeFilters(route.filters, metadata, rememberedBoard());
    rememberBoard(filters.boardId);
    const normalizedUrl = urlWithFilters(window.location.href, filters);
    if (normalizedUrl !== window.location.href) window.history.replaceState(null, '', normalizedUrl);
    const page = await api.board(filters, signal);
    signal.throwIfAborted();
    validateBoardView(page, filters);

    const heading = pageHeading('Verified leaderboards', boardIntro());
    const controls = renderFilters(metadata, filters);
    if (page.entries.length === 0) {
        replace(app, heading, controls, statePanel(
            'No verified runs on this board',
            'No verified runs match the selected board, mission, metric and player count.',
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
        ['Rank', 'rank'], ['Uploader', ''], [metricHeading(filters.metric), ''],
        ['Players', 'hide-small'], ['Verified', 'hide-small'], ['Record', ''],
    ] as const) row.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
    head.append(row);
    const body = element('tbody');
    for (const entry of page.entries) body.append(renderBoardRow(entry, metadata));
    table.append(head, body);
    tableScroll.append(table);
    tablePanel.append(tableScroll, renderPagination(page.nextCursor, filters));
    replace(app, heading, controls, tablePanel);
}

function boardIntro(): string {
    return 'Official Demo and Full runs only. Every listed score or time comes from a complete replay that the server re-simulated and checked against its recorded state.';
}

function renderFilters(metadata: BoardMetadata, filters: NormalizedBoardFilters): HTMLElement {
    const board = filters.board;
    const panel = element('section', { className: 'panel filter-panel', attrs: { 'aria-label': 'Board filters' } });
    const tabs = element('div', { className: 'tabs', attrs: { role: 'tablist', 'aria-label': 'Leaderboard metric' } });
    for (const metric of board.metrics) tabs.append(metricButton(metric, filters));

    const facets = boardFacets(metadata, board);
    const switchBoard = (next: Board): void => navigateFilters(filtersForBoard(next, filters));
    const fields = element('div', { className: 'filters' });
    if (facets.editions.length > 1) fields.append(optionSelect('Edition', facets.editions, board.edition, value => {
        switchBoard(boardForFacet(metadata, board, { edition: value === 'full' ? 'full' : 'demo' }));
    }));
    fields.append(
        optionSelect('Preset', facets.presets, board.presetId, value => {
            switchBoard(boardForFacet(metadata, board, { presetId: value }));
        }),
        optionSelect('Difficulty', facets.difficulties, board.difficultyId, value => {
            switchBoard(boardForFacet(metadata, board, { difficultyId: value }));
        }),
    );
    if (facets.variants.length > 1) fields.append(optionSelect(
        'Board',
        facets.variants.map(item => ({ id: item.boardId, label: item.displayName })),
        board.boardId,
        value => {
            const next = facets.variants.find(item => item.boardId === value);
            if (next !== undefined) switchBoard(next);
        },
    ));
    fields.append(
        optionSelect(
            'Mission',
            board.missions.map(mission => ({ id: mission.missionId, label: mission.displayName })),
            filters.missionId,
            value => navigateFilters({ ...filters, missionId: value, cursor: null }),
        ),
        maxConcurrentPlayersInput(filters),
    );
    panel.append(tabs, fields, element('p', {
        className: 'notice',
        text: `${board.displayName}: ${boardPolicyLabel(board)}.`,
    }));
    return panel;
}

function metricButton(metric: BoardMetric, filters: NormalizedBoardFilters): HTMLButtonElement {
    const selected = filters.metric === metric;
    const button = element('button', {
        text: metricLabel(metric),
        attrs: { type: 'button', role: 'tab', 'aria-selected': String(selected), tabindex: selected ? '0' : '-1' },
    });
    button.addEventListener('click', () => navigateFilters({ ...filters, metric, cursor: null }));
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

function maxConcurrentPlayersInput(filters: BoardFilters): HTMLLabelElement {
    const label = element('label', { text: 'Max concurrent players (optional)' });
    const input = element('input', {
        attrs: { type: 'number', inputmode: 'numeric', min: '1', max: '4', step: '1', placeholder: 'Any', 'aria-label': 'Exact player count' },
    });
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

function renderBoardRow(entry: LeaderboardEntry, metadata: BoardMetadata): HTMLTableRowElement {
    const row = element('tr');
    row.append(
        element('td', { className: 'rank', text: `#${entry.rank}` }),
        element('td', {}, [uploaderView(entry.uploader, playerLink)]),
        element('td', { className: 'primary-metric', text: formatMetricValue(entry.metricValue, metadata.tickDuration) }),
        element('td', { className: 'hide-small', text: participationLabel(entry.maxConcurrentPlayers, entry.participantInstanceCount) }),
        element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }),
        element('td', {}, [runLink(entry.runId, 'Details')]),
    );
    return row;
}

async function renderPlayer(
    api: HighscoreApi,
    publicKey: string,
    cursor: string | null,
    signal: AbortSignal,
): Promise<void> {
    const [page, metadata] = await Promise.all([api.playerRuns(publicKey, cursor, signal), api.metadata(signal)]);
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
    const { renderPlayerPersonalBests, renderPlayerRunHistory } = playerTables(runLink, renderPlayerPagination, metadata);
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
    const [run, metadata] = await Promise.all([api.run(id, signal), api.metadata(signal)]);
    signal.throwIfAborted();
    const ownerBridge = await bridgeForKeys(run.uploader === null ? [] : [run.uploader.publicKey]);
    signal.throwIfAborted();
    const labels = runLabels(metadata, run.boardId, run.missionId);
    const mainPanel = element('section', { className: 'panel detail-section' });
    mainPanel.append(
        element('span', { className: 'badge verified', text: '✓ Server replay verified' }),
        element('div', { className: 'metric-hero', text: `${formatInteger(run.metrics.originalScoreDelta)} points` }),
        definitionList([
            ['Uploader', uploaderView(run.uploader, playerLink)],
            ['Participation', participationLabel(run.maxConcurrentPlayers, run.participantInstanceCount)],
            ['Board', labels.board === null ? labels.boardLabel : `${labels.boardLabel} (${boardPolicyLabel(labels.board)})`],
            ['Mission', labels.missionLabel],
            ['Edition', editionLabel(run.edition)],
            ['Original score', formatInteger(run.metrics.originalScoreDelta)],
            ['Active time', formatActiveTime(run.metrics.activeSimulationTicks, metadata.tickDuration)],
            ['Ransom collected', formatInteger(run.metrics.ransomCollected)],
            ['Campaign score', `${run.startingCampaignScore} → ${run.finalCampaignScore}`],
            ['Verified', formatDate(run.verifiedAtUnixMs)],
        ]),
        renderReplayActions(api, run),
    );

    const record = element('aside', { className: 'panel detail-section' });
    record.append(element('h2', { text: 'Replay record' }), definitionList([
        ['Replay', `${run.replay.artifact.sha256} · ${formatBytes(run.replay.artifact.byteLength)}`],
        ['Replay schema', String(run.replay.replaySchemaVersion)],
        ['Recorded engine version', run.recordedEngineVersion],
    ]), renderSimConfig(run));
    appendAchievements(record, run.achievements);
    record.append(renderReportForm(api, { kind: 'run', run_id: run.runId }, signal));
    record.append(ownerBridge === null
        ? signerUnavailableNotice('Owner deletion is available only to the named uploader with the protected identity signer.')
        : renderDeletionForm(api, ownerBridge, { kind: 'run', run_id: run.runId }, signal));
    replace(
        app,
        pageHeading(`${labels.missionLabel} — verified run`, 'Full replay and the result the server reproduced from it.'),
        element('div', { className: 'detail-grid' }, [mainPanel, record]),
    );
}

function renderReplayActions(api: HighscoreApi, run: RunDetail): HTMLElement {
    const wrapper = element('div');
    const actions = element('div', { className: 'actions' });
    const download = link('Download full replay', api.replayUrl(run.runId), 'button secondary');
    download.download = `${run.runId}.rhrec`;
    download.rel = 'noopener';
    actions.append(download);
    wrapper.append(actions);
    if (run.viewer.availability.status === 'unavailable') {
        wrapper.append(element('p', { className: 'notice', text: run.viewer.availability.safeReason }));
    } else if (run.viewer.contentRequirement === 'bundled_demo') {
        // The game page selects /wasm/<runtime build>/, the engine that recorded the replay.
        const viewerUrl = new URL('../', window.location.href);
        viewerUrl.search = '';
        viewerUrl.searchParams.set('run', run.runId);
        actions.prepend(link('Watch replay', viewerUrl.toString(), 'button'));
        wrapper.append(element('p', {
            className: 'notice',
            text: `Playback uses engine build ${run.viewer.runtimeBuild} with the Demo data it was published with.`,
        }));
    } else {
        // TODO: browser playback of Full runs needs a local Full content picker outside multiplayer joins.
        wrapper.append(element('p', {
            className: 'notice',
            text: `Playback requires a local Full installation and engine build ${run.viewer.runtimeBuild}. Download the replay to watch it in the game.`,
        }));
    }
    return wrapper;
}

function renderSimConfig(run: RunDetail): HTMLElement {
    const settings = element('details');
    settings.append(element('summary', { text: `Gameplay settings (${Object.keys(run.simConfig).length})` }));
    settings.append(definitionList(Object.entries(run.simConfig).map(([name, value]) => [
        name.replaceAll('_', ' '),
        typeof value === 'boolean' ? (value ? 'Enabled' : 'Disabled')
            : typeof value === 'object' ? JSON.stringify(value) : String(value),
    ])));
    return settings;
}

async function bridgeForKeys(publicKeys: readonly string[]): Promise<LeaderboardSigningBridge | null> {
    if (publicKeys.length === 0) return null;
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
    url.searchParams.delete('submission');
    url.searchParams.delete('player');
    url.searchParams.set('run', runId);
    return internalLink(label, url);
}

function playerLink(publicKey: string, label: string): HTMLAnchorElement {
    const url = new URL(window.location.href);
    url.searchParams.delete('run');
    url.searchParams.delete('submission');
    url.searchParams.set('player', publicKey);
    return internalLink(label, url);
}

function internalLink(label: string, url: URL): HTMLAnchorElement {
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

function rememberedBoard(): string | null {
    try {
        return window.localStorage.getItem(BOARD_STORAGE_KEY);
    } catch {
        // Storage may be disabled; the first published board remains the default.
        return null;
    }
}

function rememberBoard(boardId: string): void {
    try {
        window.localStorage.setItem(BOARD_STORAGE_KEY, boardId);
    } catch {
        // Navigation still works when storage is unavailable.
    }
}

function metricHeading(metric: BoardMetric): string {
    return metric === 'original_score' ? 'Original score' : 'Active time';
}
