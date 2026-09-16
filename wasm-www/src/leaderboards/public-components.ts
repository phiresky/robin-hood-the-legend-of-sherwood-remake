import { formatActiveTime, formatDate, formatInteger, formatMetricValue, metricLabel } from './format.js';
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
        for (const achievement of earnedAchievements) badges.append(element('span', {
            className: 'badge', text: achievement.label,
        }));
        target.append(badges);
    }
    if (unverifiableAchievements.length > 0) target.append(element('p', {
        className: 'notice',
        text: `Not awarded because verification was unavailable: ${unverifiableAchievements
            .map(achievement => achievement.label).join(', ')}.`,
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
            className: 'panel table-panel player-results',
            attrs: { 'aria-labelledby': 'player-bests-heading' },
        });
        section.append(element('h2', { text: 'Personal bests', attrs: { id: 'player-bests-heading' } }));
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        table.append(element('caption', {
            text: `${formatInteger(personalBests.length)} personal best${personalBests.length === 1 ? '' : 's'}`,
        }));
        const head = element('thead');
        const header = element('tr');
        for (const [label, className] of [
            ['Board', ''], ['Best', ''], ['Players', 'hide-small'], ['Replay', ''],
        ] as const) header.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
        head.append(header);
        const body = element('tbody');
        for (const best of personalBests) {
            const row = element('tr');
            row.append(
                subjectCell(best.filter.boardId, best.filter.missionId, metricLabel(best.filter.metric)),
                element('td', { className: 'primary-metric', text: formatMetricValue(best.metricValue, metadata.tickDuration) }),
                element('td', {
                    className: 'hide-small',
                    text: best.filter.maxConcurrentPlayers === null
                        ? 'Any player count'
                        : `${formatInteger(best.filter.maxConcurrentPlayers)} players`,
                }),
                element('td', {}, [runLink(best.runId, 'Watch replay')]),
            );
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
            className: 'panel table-panel player-results',
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
            ['Run', ''], ['Result', ''], ['Players', 'hide-small'], ['Added', 'hide-small'], ['Replay', ''],
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
            element('td', {}, [
                element('div', { className: 'player' }, [
                    element('span', {
                        className: 'primary-metric',
                        text: `${formatInteger(entry.run.metrics.originalScoreDelta)} points`,
                    }),
                    element('span', {
                        className: 'fingerprint',
                        text: `${formatActiveTime(entry.run.metrics.activeSimulationTicks, metadata.tickDuration)} · ${formatInteger(entry.run.metrics.ransomCollected)} net money`,
                    }),
                ]),
            ]),
            element('td', {
                className: 'hide-small',
                text: participationLabel(entry.run.maxConcurrentPlayers, entry.run.participantInstanceCount),
            }),
            element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }),
            element('td', {}, [runLink(entry.run.runId, 'Watch replay')]),
        );
        return row;
    }

    return { renderPlayerPersonalBests, renderPlayerRunHistory };
}
