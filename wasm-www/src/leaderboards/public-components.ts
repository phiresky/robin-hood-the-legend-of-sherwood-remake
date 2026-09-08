import { formatDate, formatInteger, formatMetricValue } from './format.js';
import type { PlayerPersonalBest, PlayerRunHistoryPage, PlayerRunHistoryEntry, RunDetail } from './types.js';
import { element, statePanel } from './dom.js';
import type { PublicParticipant, AggregatePublicParticipant, Achievement } from './types.js';

type PlayerLink = (publicKey: string, label: string) => HTMLAnchorElement;

export function participantView(participant: PublicParticipant, playerLink: PlayerLink): HTMLElement {
    return element('span', { className: 'player' }, [
        playerLink(participant.publicKey, participant.username),
        element('span', {
            className: 'fingerprint',
            text: participant.publicKeyFingerprint,
            attrs: { title: `Public key ${participant.publicKey}; only its owner can change this username` },
        }),
    ]);
}

export function aggregateParticipantView(
    participant: AggregatePublicParticipant,
    playerLink: PlayerLink,
): HTMLElement {
    return element('span', { className: 'player' }, [
        playerLink(participant.publicKey, participant.currentDisplayName),
        element('span', {
            className: 'fingerprint',
            text: participant.publicKeyFingerprint,
            attrs: { title: `Public key ${participant.publicKey}; only its owner can change this username` },
        }),
    ]);
}

export function appendAchievements(proof: HTMLElement, achievements: readonly Achievement[]): void {
    const earnedAchievements = achievements.filter(achievement => achievement.evaluation === 'earned');
    const unverifiableAchievements = achievements.filter(
        achievement => achievement.evaluation === 'unverifiable',
    );
    if (earnedAchievements.length > 0) {
        proof.append(element('h2', { text: 'Achievements' }));
        const badges = element('div', { className: 'badges' });
        for (const achievement of earnedAchievements) badges.append(element('span', {
            className: 'badge', text: achievement.label,
        }));
        proof.append(badges);
    }
    if (unverifiableAchievements.length > 0) proof.append(element('p', {
        className: 'notice',
        text: `Not awarded because verification was unavailable: ${unverifiableAchievements
            .map(achievement => achievement.label).join(', ')}.`,
    }));

}

export function playerTables(
    runLink: (id: string, label: string) => HTMLAnchorElement,
    renderPlayerPagination: (page: PlayerRunHistoryPage, cursor: string | null) => HTMLElement,
): {
    renderPlayerPersonalBests: (bests: readonly PlayerPersonalBest[]) => HTMLElement;
    renderPlayerRunHistory: (page: PlayerRunHistoryPage, cursor: string | null) => HTMLElement;
} {
    function renderPlayerPersonalBests(personalBests: readonly PlayerPersonalBest[]): HTMLElement {
        if (personalBests.length === 0) return statePanel(
            'No personal bests yet',
            'This public key does not currently hold a verified result on any published board.',
        );
        const section = element('section', {
            className: 'panel table-panel player-results',
            attrs: { 'aria-labelledby': 'player-bests-heading' },
        });
        section.append(element('h2', { text: 'Personal bests', attrs: { id: 'player-bests-heading' } }));
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        table.append(element('caption', {
            text: `${formatInteger(personalBests.length)} verified board best${personalBests.length === 1 ? '' : 's'}`,
        }));
        const head = element('thead');
        const header = element('tr');
        for (const [label, className] of [
            ['Board', ''], ['Best', ''], ['Players', 'hide-small'], ['Identity', 'hide-small'], ['Record', ''],
        ] as const) header.append(element('th', { text: label, className, attrs: { scope: 'col' } }));
        head.append(header);
        const body = element('tbody');
        for (const best of personalBests) {
            const row = element('tr');
            row.append(
                element('td', {}, [
                    element('div', { className: 'player' }, [
                        element('strong', { text: playerSubjectLabel(best.filter.subject) }),
                        element('span', {
                            className: 'fingerprint',
                            text: best.filter.metric === 'original_score' ? 'Original score' : 'Fastest successful',
                        }),
                    ]),
                ]),
                element('td', { className: 'primary-metric', text: formatMetricValue(best.metricValue) }),
                element('td', {
                    className: 'hide-small',
                    text: best.filter.maxConcurrentPlayers === null
                        ? 'Any maximum'
                        : `${formatInteger(best.filter.maxConcurrentPlayers)} max concurrent`,
                }),
                element('td', {
                    className: 'hide-small fingerprint',
                    text: best.filter.competitionManifestSha256 === null ? 'Main board' : 'Pinned challenge',
                    attrs: {
                        title: `Ruleset ${best.filter.rulesetManifestSha256}; configuration ${best.filter.rulesConfigSha256}`,
                    },
                }),
                element('td', {}, [runLink(best.runId, 'Details')]),
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
                    ? 'No publicly attributed verified runs are currently available for this key.'
                    : 'The authenticated history cursor reached an empty page. Return to the previous page.',
            );
            if (cursor !== null) empty.append(renderPlayerPagination(page, cursor));
            return empty;
        }
        const section = element('section', {
            className: 'panel table-panel player-results',
            attrs: { 'aria-labelledby': 'player-history-heading' },
        });
        section.append(element('h2', { text: 'Verified run history', attrs: { id: 'player-history-heading' } }));
        const scroll = element('div', { className: 'table-scroll' });
        const table = element('table');
        table.append(element('caption', {
            text: `${formatInteger(page.runs.length)} publicly attributed run${page.runs.length === 1 ? '' : 's'} on this page`,
        }));
        const head = element('thead');
        const header = element('tr');
        for (const [label, className] of [
            ['Run', ''], ['Result', ''], ['Players', 'hide-small'], ['Verified', 'hide-small'], ['Record', ''],
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
            element('td', {}, [
                element('div', { className: 'player' }, [
                    element('strong', { text: playerSubjectLabel(entry.run.subject) }),
                    element('span', {
                        className: 'fingerprint',
                        text: entry.run.competitionManifestSha256 === null ? 'Main board' : 'Pinned challenge',
                    }),
                ]),
            ]),
            element('td', {}, [
                element('div', { className: 'player' }, [
                    element('span', {
                        className: 'primary-metric',
                        text: `${formatInteger(entry.run.metrics.originalScoreDelta)} score`,
                    }),
                    element('span', {
                        className: 'fingerprint',
                        text: `${formatInteger(entry.run.metrics.activeSimulationTicks)} active ticks · ${formatInteger(entry.run.metrics.ransomCollected)} ransom`,
                    }),
                ]),
            ]),
            element('td', {
                className: 'hide-small',
                text: `${formatInteger(entry.run.maxConcurrentPlayers)} max concurrent · ${formatInteger(entry.run.participantInstanceCount)} total instances`,
            }),
            element('td', { className: 'hide-small', text: formatDate(entry.verifiedAtUnixMs) }),
            element('td', {}, [runLink(entry.run.runId, 'Details')]),
        );
        return row;
    }

    function playerSubjectLabel(subject: PlayerRunHistoryEntry['run']['subject']): string {
        if (subject.kind === 'full_campaign') return 'Full Campaign';
        return `${subject.missionId} · ${subject.category === 'campaign' ? 'Campaign mission' : 'Individual Level'}`;
    }

    return { renderPlayerPersonalBests, renderPlayerRunHistory };
}

export function campaignCompositionLabel(run: RunDetail): string {
    return run.composition.kind === 'full_campaign'
        ? `${run.runId} · ${run.composition.orderedSessionRunIds.length} ordered `
            + `session${run.composition.orderedSessionRunIds.length === 1 ? '' : 's'}`
        : 'Authenticated replay genesis';
}
