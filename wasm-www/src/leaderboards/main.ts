import { missionPlayUrl, rulesHelp } from './browsing.js';
import { missionGroup } from './missions.js';
import { renderUsernameForm, renderDeletionForm } from './account-forms.js';
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
    metricLabel,
} from './format.js';
import {
    type Board,
    type BoardMetadata,
    type BoardMetric,
    type LeaderboardEntry,
    type LatestRun,
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
        document.title = 'Robin Hood — Leaderboards';
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
    if (status.runId !== null) panel.append(runLink(status.runId, 'View result'));
    replace(app, pageHeading('Replay submission', 'Follow the progress of your submitted replay.'), panel);
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
        replace(app, pageHeading('Leaderboards', boardIntro()), statePanel(
            'No leaderboards yet',
            'Check back soon for the first results.',
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

    const heading = pageHeading('Leaderboards', boardIntro());
    const help = submissionHelp();
    const browse = element('button', { className: 'button secondary', text: 'Browse mission rankings', attrs: { type: 'button' } });
    browse.addEventListener('click', () => document.getElementById('mission-rankings')?.scrollIntoView({ block: 'start' }));
    const actions = element('div', { className: 'actions' }, [browse]);
    actions.append(help);
    heading.append(actions);
    const rankings = element('section', { attrs: { id: 'mission-rankings', 'aria-label': 'Mission rankings' } });
    rankings.append(element('h2', { text: 'Mission rankings' }), renderFilters(metadata, filters));
    const emptyActions = element('div', { className: 'actions' }, [
        link('Play this mission', missionPlayUrl(window.location.href, filters.board.edition, filters.missionId), 'button'),
    ]);
    const latest = filters.cursor === null ? renderLatestRuns(api, metadata, signal, entries => {
        if (page.entries.length !== 0) return;
        const other = entries.find(entry => entry.run.missionId !== filters.missionId
            && metadata.boards.some(board => board.boardId === entry.run.boardId));
        if (other === undefined) return;
        const board = metadata.boards.find(board => board.boardId === other.run.boardId)!;
        const url = new URL(urlWithFilters(window.location.href, {
            ...filtersForBoard(board, filters), missionId: other.run.missionId, maxConcurrentPlayers: null,
        }));
        emptyActions.append(internalLink(`Browse ${runLabels(metadata, other.run.boardId, other.run.missionId).missionLabel}`, url));
    }) : null;
    replace(app, heading, ...(latest === null ? [] : [latest]), rankings);
    if (page.entries.length === 0) {
        const filtered = filters.maxConcurrentPlayers !== null || filters.board.presetId !== 'any';
        const title = filters.maxConcurrentPlayers === null ? 'No runs with these rules yet'
            : filters.maxConcurrentPlayers === 1 ? 'No solo runs yet' : `No ${filters.maxConcurrentPlayers}-player runs yet`;
        const empty = statePanel(filtered ? title : 'Be the first to set a record', filtered
            ? 'Try broader filters, or submit a run with these settings.'
            : 'Win this mission, then choose “Submit this run” on the results screen.');
        const allRules = metadata.boards.find(board => board.edition === filters.board.edition && board.presetId === 'any');
        if (filters.maxConcurrentPlayers !== null || (allRules !== undefined && allRules.boardId !== filters.boardId)) {
            const reset = element('button', { className: 'button secondary', text: allRules === undefined ? 'Show all player counts' : 'Show all runs', attrs: { type: 'button' } });
            reset.addEventListener('click', () => navigateFilters({
                ...filtersForBoard(allRules ?? filters.board, filters), maxConcurrentPlayers: null,
            }));
            emptyActions.prepend(reset);
        }
        empty.append(emptyActions);
        rankings.append(empty);
        return;
    }

    const tablePanel = element('section', { className: 'panel table-panel ranking-table' });
    const tableScroll = element('div', { className: 'table-scroll' });
    const table = element('table');
    table.append(element('caption', {
        text: `${filters.board.missions.find(mission => mission.missionId === filters.missionId)?.displayName} · ${metricLabel(filters.metric)}`,
    }));
    const head = element('thead');
    const row = element('tr');
    for (const [label, className] of [
        ['Rank', 'rank'], ['Player', ''], ['Score', ''], ['Time', ''],
        ['Players', 'hide-small'], ['Added', 'hide-small'], ['Replay', ''],
    ] as const) row.append(element('th', { text: label, className, attrs: { scope: 'col', ...(label === metricHeading(filters.metric) ? { 'aria-sort': filters.metric === 'original_score' ? 'descending' : 'ascending' } : {}) } }));
    head.append(row);
    const body = element('tbody');
    for (const entry of page.entries) body.append(renderBoardRow(entry, metadata));
    table.append(head, body);
    tableScroll.append(table);
    tablePanel.append(tableScroll);
    if (page.nextCursor !== null || previousPageUrls.has(window.location.href)) {
        tablePanel.append(renderPagination(page.nextCursor, filters));
    }
    rankings.append(tablePanel);
}

function submissionHelp(): HTMLDetailsElement {
    const steps = element('ol');
    for (const text of [
        'Win a mission in the remake.',
        'On the results screen, choose “Submit this run”. To submit future wins automatically, enable “Always submit won runs”.',
        'Your replay is checked automatically. Once it passes, your score and time appear here.',
    ]) steps.append(element('li', { text }));
    return element('details', { className: 'panel submission-help', attrs: { id: 'how-to-submit' } }, [
        element('summary', { text: 'How to submit a run' }), steps,
        link('Play Robin Hood', '../', 'button'),
    ]);
}

function boardIntro(): string {
    return 'Chase a high score, beat the fastest time, or watch how other players did it.';
}

function renderFilters(metadata: BoardMetadata, filters: NormalizedBoardFilters): HTMLElement {
    const board = filters.board;
    const panel = element('section', { className: 'panel filter-panel', attrs: { 'aria-label': 'Board filters' } });
    const tabs = element('div', { className: 'tabs', attrs: { role: 'tablist', 'aria-label': 'Leaderboard metric' } });
    for (const metric of board.metrics) tabs.append(metricButton(metric, filters));

    const facets = boardFacets(metadata, board);
    const switchBoard = (next: Board): void => navigateFilters(filtersForBoard(next, filters));
    const fields = element('div', { className: 'filters' });
    const missionField = element('label', { className: 'filter-wide', text: 'Mission' });
    const missionSelect = element('select', { attrs: { 'aria-label': 'Mission' } });
    const choices = new Map<string, { board: Board; missionId: string }>();
    const groups = new Map<string, HTMLOptGroupElement>();
    for (const edition of ['full', 'demo'] as const) {
        const available = metadata.boards.filter(item => item.edition === edition);
        if (available.length === 0) continue;
        const preferred = boardForFacet(metadata, board, { edition });
        const missions = new Map(available.flatMap(item => item.missions).map(mission => [mission.missionId, mission]));
        for (const mission of missions.values()) {
            const key = `${edition}:${mission.missionId}`;
            const target = preferred.missions.some(item => item.missionId === mission.missionId)
                ? preferred : available.find(item => item.missions.some(candidate => candidate.missionId === mission.missionId))!;
            choices.set(key, { board: target, missionId: mission.missionId });
            const groupName = edition === 'demo' ? 'Demo missions' : missionGroup(mission.missionId);
            let group = groups.get(groupName);
            if (group === undefined) {
                group = element('optgroup', { attrs: { label: groupName } });
                groups.set(groupName, group);
                missionSelect.append(group);
            }
            const option = element('option', { text: mission.displayName, attrs: { value: key } });
            option.selected = board.edition === edition && filters.missionId === mission.missionId;
            group.append(option);
        }
    }
    missionSelect.addEventListener('change', () => {
        const choice = choices.get(missionSelect.value);
        if (choice === undefined) throw new Error('Unknown mission choice');
        navigateFilters({ ...filtersForBoard(choice.board, filters), missionId: choice.missionId });
    });
    missionField.append(missionSelect);
    fields.append(missionField);
    if (facets.presets.length > 1) fields.append(optionSelect('Rules', facets.presets.map(item => ({ ...item, label: item.id === 'any' ? 'All rules' : item.label })), board.presetId, value => {
        switchBoard(boardForFacet(metadata, board, { presetId: value }));
    }));
    if (facets.difficulties.length > 1) fields.append(optionSelect('Difficulty', facets.difficulties, board.difficultyId, value => {
        switchBoard(boardForFacet(metadata, board, { difficultyId: value }));
    }));
    if (facets.variants.length > 1) fields.append(optionSelect(
        'Board',
        facets.variants.map(item => ({ id: item.boardId, label: item.displayName })),
        board.boardId,
        value => {
            const next = facets.variants.find(item => item.boardId === value);
            if (next !== undefined) switchBoard(next);
        },
    ));
    fields.append(maxConcurrentPlayersInput(filters));
    const rules = element('details', { className: 'rules-help' }, [element('summary', { text: 'What do these rules mean?' }), element('p', { text: rulesHelp })]);
    panel.append(fields, rules, tabs);
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
    return optionSelect('Players', [
        { id: '', label: 'Any number' },
        { id: '1', label: 'Solo' },
        { id: '2', label: '2 players' },
        { id: '3', label: '3 players' },
        { id: '4', label: '4 players' },
    ], filters.maxConcurrentPlayers === null ? '' : String(filters.maxConcurrentPlayers), value => {
        navigateFilters({ ...filters, maxConcurrentPlayers: value === '' ? null : Number(value), cursor: null });
    });
}

function renderBoardRow(entry: LeaderboardEntry, metadata: BoardMetadata): HTMLTableRowElement {
    const row = element('tr');
    row.append(
        element('td', { className: 'rank', text: `#${entry.rank}` }),
        element('td', {}, [uploaderView(entry.uploader, playerLink)]),
        element('td', { className: entry.metricValue.metric === 'original_score' ? 'primary-metric' : '', attrs: { 'data-label': 'Score' },
            text: entry.metrics !== null ? formatInteger(entry.metrics.originalScoreDelta)
                : entry.metricValue.metric === 'original_score' ? formatInteger(entry.metricValue.points) : 'Unavailable' }),
        element('td', { className: entry.metricValue.metric === 'fastest_success' ? 'primary-metric' : '', attrs: { 'data-label': 'Time' },
            text: entry.metrics !== null ? formatActiveTime(entry.metrics.activeSimulationTicks, metadata.tickDuration)
                : entry.metricValue.metric === 'fastest_success' ? formatActiveTime(entry.metricValue.activeSimulationTicks, metadata.tickDuration) : 'Unavailable' }),
        element('td', { className: 'hide-small', text: participationLabel(entry.maxConcurrentPlayers, entry.participantInstanceCount) }),
        element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }),
        element('td', {}, [runLink(entry.runId, 'Watch replay')]),
    );
    return row;
}

function renderLatestRuns(api: HighscoreApi, metadata: BoardMetadata, signal: AbortSignal, loaded: (entries: readonly LatestRun[]) => void): HTMLElement {
    const section = element('section', { className: 'panel table-panel player-results latest-submissions', attrs: { 'aria-label': 'Latest submissions' } });
    const heading = element('h2', { text: 'Latest submissions' });
    section.append(heading, element('p', { className: 'section-intro', text: 'Loading recent runs…' }));
    void api.latestRuns(signal).then(entries => {
        signal.throwIfAborted();
        loaded(entries);
        const intro = element('p', { className: 'section-intro', text: entries.length === 0
            ? 'No submissions yet. Yours could be the first!'
            : 'The latest verified runs across all missions.' });
        replace(section, heading, intro);
        if (entries.length === 0) return;
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        const head = element('thead');
        const header = element('tr');
        for (const title of ['Player', 'Mission', 'Score', 'Time', 'Replay']) {
            header.append(element('th', { text: title, attrs: { scope: 'col' } }));
        }
        head.append(header);
        const body = element('tbody');
        for (const { run, verifiedAtUnixMs } of entries) {
            const row = element('tr');
            row.append(
                element('td', {}, [uploaderView(run.uploader, playerLink), element('small', { className: 'fingerprint', text: formatDate(verifiedAtUnixMs) })]),
                element('td', {}, [internalLink(runLabels(metadata, run.boardId, run.missionId).missionLabel,
                    new URL(urlWithFilters(window.location.href, { boardId: run.boardId, missionId: run.missionId,
                        metric: 'original_score', maxConcurrentPlayers: null, cursor: null })))]),
                element('td', { text: formatInteger(run.metrics.originalScoreDelta), attrs: { 'data-label': 'Score' } }),
                element('td', { text: formatActiveTime(run.metrics.activeSimulationTicks, metadata.tickDuration), attrs: { 'data-label': 'Time' } }),
                element('td', {}, [runLink(run.runId, 'Watch replay')]),
            );
            body.append(row);
        }
        table.append(head, body);
        scroll.append(table);
        section.append(scroll);
    }).catch(error => {
        if (signal.aborted) return;
        console.error('Latest submissions could not be loaded', error);
        replace(section, heading, element('p', { className: 'section-intro', text: 'Latest submissions are unavailable right now.' }));
    });
    return section;
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
        element('div', { className: 'metric-hero', text: profile.username }),
        element('details', {}, [element('summary', { text: 'Player ID' }), definitionList([
            ['ID', profile.publicKeyFingerprint], ['Public key', profile.publicKey],
        ])]),
    );
    const ownerBridge = await bridgeForKeys([profile.publicKey]);
    signal.throwIfAborted();
    if (ownerBridge !== null) panel.append(renderUsernameForm(api, profile, ownerBridge, signal));
    const { renderPlayerPersonalBests, renderPlayerRunHistory } = playerTables(runLink, renderPlayerPagination, metadata);
    replace(
        app,
        pageHeading('Player profile', 'Personal bests and recent runs.'),
        panel,
        renderPlayerPersonalBests(page.personalBests),
        renderPlayerRunHistory(page, cursor),
    );
}

function renderPlayerPagination(page: PlayerRunHistoryPage, cursor: string | null): HTMLElement {
    if (page.nextCursor === null && cursor === null) return element('div');
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
    const title = `${labels.missionLabel} - run by ${run.uploader?.username ?? 'Anonymous'}`;
    document.title = title;
    const mainPanel = element('section', { className: 'panel detail-section' });
    mainPanel.append(
        element('span', { className: 'badge verified', text: '✓ Verified run' }),
        definitionList([
            ['Player', uploaderView(run.uploader, playerLink)],
            ['Players', participationLabel(run.maxConcurrentPlayers, run.participantInstanceCount)],
            ['Rules', labels.board === null ? labels.boardLabel : boardPolicyLabel(labels.board)],
            ['Mission', labels.missionLabel],
            ['Edition', editionLabel(run.edition)],
            ['Net money', formatInteger(run.metrics.ransomCollected)],
            ['Campaign score', `${run.startingCampaignScore} → ${run.finalCampaignScore}`],
            ['Verified', formatDate(run.verifiedAtUnixMs)],
        ]),
    );

    const record = element('aside', { className: 'panel detail-section' });
    const technical = element('details');
    technical.append(element('summary', { text: 'Replay information' }));
    technical.append(definitionList([
        ['Replay', `${run.replay.artifact.sha256} · ${formatBytes(run.replay.artifact.byteLength)}`],
        ['Replay schema', String(run.replay.replaySchemaVersion)],
        ['Recorded engine version', run.recordedEngineVersion],
    ]), renderSimConfig(run));
    appendAchievements(record, run.achievements);
    record.append(technical);
    if (ownerBridge !== null) record.append(renderDeletionForm(api, ownerBridge, { kind: 'run', run_id: run.runId }, signal));
    replace(
        app,
        pageHeading(title, 'Watch the replay and explore the result.'),
        element('section', { className: 'panel run-result', attrs: { 'aria-label': 'Run result' } }, [
            element('div', {}, [element('span', { text: 'Score' }), element('strong', { text: `${formatInteger(run.metrics.originalScoreDelta)} points` })]),
            element('div', {}, [element('span', { text: 'Time' }), element('strong', { text: formatActiveTime(run.metrics.activeSimulationTicks, metadata.tickDuration) })]),
        ]),
        element('section', { className: 'panel replay-section' }, [renderReplayActions(api, run)]),
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
    } else {
        const viewerUrl = new URL('../', window.location.href);
        if (run.replay.artifact.mediaType === 'application/x-robin-rhrec+compact') viewerUrl.pathname = '/legacy-replay/';
        viewerUrl.search = '';
        viewerUrl.searchParams.set('run', run.runId);
        viewerUrl.searchParams.set('embed', '1');
        wrapper.prepend(element('iframe', {
            className: 'replay-player',
            attrs: {
                src: viewerUrl.toString(), title: 'Replay player',
                allow: 'autoplay; fullscreen', allowfullscreen: '',
                referrerpolicy: 'no-referrer',
            },
        }));
        viewerUrl.searchParams.delete('embed');
        const open = link('Open replay in new tab', viewerUrl.toString(), 'button secondary');
        open.target = '_blank';
        open.rel = 'noopener';
        actions.prepend(open);
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

function navigateFilters(filters: BoardFilters): void {
    navigate(urlWithFilters(window.location.href, filters), 'rankings');
}

function navigate(url: string, destination: 'top' | 'rankings' = 'top'): void {
    window.history.pushState(null, '', url);
    if (destination === 'top') window.scrollTo({ top: 0, behavior: 'instant' });
    void renderCurrentRoute().then(() => {
        if (destination === 'rankings' && window.location.href === url) {
            document.getElementById('mission-rankings')?.scrollIntoView({ block: 'start' });
        }
    });
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
    return metric === 'original_score' ? 'Score' : 'Time';
}
