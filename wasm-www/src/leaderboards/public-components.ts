import { achievementRules } from './achievements.js';
import { formatActiveTime, formatDate, formatInteger, formatMetricValue } from './format.js';
import type {
    Achievement,
    BoardMetadata,
    PlayerPersonalBest,
    PlayerRunHistoryEntry,
    PlayerRunHistoryPage,
    PublicParticipant,
} from './types.js';
import { element, statePanel } from './dom.js';
import { runLabels } from './view-model.js';

type PlayerLink = (publicKey: string, label: string) => HTMLAnchorElement;

export function participantView(participant: PublicParticipant, playerLink: PlayerLink): HTMLElement {
    return element('span', { className: 'player' }, [
        playerLink(participant.publicKey, participant.username),

    ]);
}

/** The named uploader, or an explicit anonymous marker. */
export function uploaderView(uploader: PublicParticipant | null, playerLink: PlayerLink): HTMLElement {
    return uploader === null
        ? element('span', { className: 'player', text: 'Anonymous' })
        : participantView(uploader, playerLink);
}

export function participationLabel(maxConcurrentPlayers: number, _participantInstanceCount: number): string {
    return maxConcurrentPlayers === 1 ? 'Solo' : `${formatInteger(maxConcurrentPlayers)} players`;
}

export function appendAchievements(target: HTMLElement, achievements: readonly Achievement[]): void {
    const earnedAchievements = achievements.filter(achievement => achievement.evaluation === 'earned');
    const unverifiableAchievements = achievements.filter(
        achievement => achievement.evaluation === 'unverifiable',
    );
    if (earnedAchievements.length > 0) {
        target.append(element('h2', { text: 'Achievements' }));
        const badges = element('div', { className: 'badges' });
        for (const achievement of earnedAchievements) {
            const rules = achievementRules[achievement.id];
            badges.append(element('details', { className: 'achievement' }, [
                element('summary', { className: 'badge', text: rules?.name ?? achievement.label }),
                element('p', { text: rules?.description ?? 'Rules for this badge are unavailable in this version of the site.' }),
            ]));
        }
        target.append(badges);
    }
    if (unverifiableAchievements.length > 0) target.append(element('p', {
        className: 'notice',
        text: `Not awarded because verification was unavailable: ${unverifiableAchievements
            .map(achievement => achievementRules[achievement.id]?.name ?? achievement.label).join(', ')}.`,
    }));
}

export function playerTables(
    runLink: (id: string, label: string) => HTMLAnchorElement,
    renderPlayerPagination: (page: PlayerRunHistoryPage, cursor: string | null) => HTMLElement,
    metadata: BoardMetadata,
): {
    renderPlayerPersonalBests: (bests: readonly PlayerPersonalBest[]) => HTMLElement;
    renderPlayerRunHistory: (page: PlayerRunHistoryPage, cursor: string | null) => HTMLElement;
} {
    function subjectCell(boardId: string, missionId: string, detail: string): HTMLElement {
        const labels = runLabels(metadata, boardId, missionId);
        return element('td', {}, [
            element('div', { className: 'player' }, [
                element('strong', { text: labels.missionLabel }),
                element('span', { className: 'fingerprint', text: `${labels.boardLabel} · ${detail}` }),
            ]),
        ]);
    }

    function renderPlayerPersonalBests(personalBests: readonly PlayerPersonalBest[]): HTMLElement {
        if (personalBests.length === 0) return statePanel(
            'No personal bests yet',
            'This player has not set a record yet.',
        );
        const section = element('section', {
            className: 'panel table-panel player-results profile-results',
            attrs: { 'aria-labelledby': 'player-bests-heading' },
        });
        section.append(element('h2', { text: 'Personal bests', attrs: { id: 'player-bests-heading' } }));
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        table.append(element('caption', {
            text: 'Best score and fastest time for each mission and set of rules.',
        }));
        const head = element('thead');
        const header = element('tr');
        for (const [label, className] of [
            ['Mission', ''], ['Best score', ''], ['Fastest time', ''],
        ] as const) header.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
        head.append(header);
        const body = element('tbody');
        const groups = new Map<string, PlayerPersonalBest[]>();
        for (const best of personalBests) {
            const key = JSON.stringify([best.filter.boardId, best.filter.missionId, best.filter.maxConcurrentPlayers]);
            const group = groups.get(key) ?? [];
            group.push(best);
            groups.set(key, group);
        }
        for (const group of groups.values()) {
            const first = group[0]!;
            const players = first.filter.maxConcurrentPlayers;
            const row = element('tr');
            row.append(subjectCell(first.filter.boardId, first.filter.missionId,
                players === null ? 'Any player count' : participationLabel(players, players)));
            for (const metric of ['original_score', 'fastest_success'] as const) {
                const best = group.find(item => item.filter.metric === metric);
                const cell = element('td', { attrs: { 'data-label': metric === 'original_score' ? 'Best score' : 'Fastest time' } });
                if (best === undefined) cell.textContent = 'No record yet';
                else cell.append(element('strong', { className: 'primary-metric', text: formatMetricValue(best.metricValue, metadata.tickDuration) }),
                    runLink(best.runId, 'Watch replay'));
                row.append(cell);
            }
            body.append(row);
        }
        table.append(head, body);
        scroll.append(table);
        section.append(scroll);
        return section;
    }

    function renderPlayerRunHistory(page: PlayerRunHistoryPage, cursor: string | null): HTMLElement {
        if (page.runs.length === 0) {
            const empty = statePanel(
                cursor === null ? 'No verified run history' : 'No runs on this page',
                cursor === null
                    ? 'This player has no public runs yet.'
                    : 'Return to the previous page to see earlier runs.',
            );
            if (cursor !== null) empty.append(renderPlayerPagination(page, cursor));
            return empty;
        }
        const section = element('section', {
            className: 'panel table-panel player-results profile-results',
            attrs: { 'aria-labelledby': 'player-history-heading' },
        });
        section.append(element('h2', { text: 'Recent runs', attrs: { id: 'player-history-heading' } }));
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        table.append(element('caption', {
            text: `${formatInteger(page.runs.length)} run${page.runs.length === 1 ? '' : 's'} on this page`,
        }));
        const head = element('thead');
        const header = element('tr');
        for (const [label, className] of [
            ['Mission', ''], ['Score', ''], ['Time', ''], ['Added', 'hide-small'], ['Replay', ''],
        ] as const) header.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
        head.append(header);
        const body = element('tbody');
        for (const entry of page.runs) body.append(renderPlayerHistoryRow(entry));
        table.append(head, body);
        scroll.append(table);
        section.append(scroll, renderPlayerPagination(page, cursor));
        return section;
    }

    function renderPlayerHistoryRow(entry: PlayerRunHistoryEntry): HTMLTableRowElement {
        const row = element('tr');
        row.append(
            subjectCell(entry.run.boardId, entry.run.missionId, participationLabel(entry.run.maxConcurrentPlayers, entry.run.participantInstanceCount)),
            element('td', { className: 'primary-metric', attrs: { 'data-label': 'Score' }, text: formatInteger(entry.run.metrics.originalScoreDelta) }),
            element('td', { attrs: { 'data-label': 'Time' }, text: formatActiveTime(entry.run.metrics.activeSimulationTicks, metadata.tickDuration) }),
            element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }),
            element('td', {}, [runLink(entry.run.runId, 'Watch replay')]),
        );
        return row;
    }

    return { renderPlayerPersonalBests, renderPlayerRunHistory };
}
