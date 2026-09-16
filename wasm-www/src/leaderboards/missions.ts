import type { BoardMetadata } from './types.js';

// Campaign numbers and titles follow the campaign walkthrough in
// docs/third-party/guides/gamefaqs-swcarter.md.
const campaign: Readonly<Record<string, readonly [number, string]>> = {
    'H01_Lin_VL': [1, 'Finding Godwin'],
    'S01_Not_VL': [2, 'Rescuing Stuteley'],
    'S02_Lei_MP': [3, 'Scarlet Night'],
    'H02_Not_EC': [4, 'Confessions of an Outlaw'],
    'H03_Der_MK': [5, 'The Prince and the Outlaw'],
    'S03_FoB_MP': [6, 'Pillaging'],
    'H04_Lei_VL': [7, 'The Evening Visitor'],
    'H05_Lin_EC': [8, 'The Godfather in Prison'],
    'S04_Der_EC': [9, 'The Lock-up and the Friar'],
    'H07_Not_MK': [10, 'The Silver Arrow'],
    'Str02_Der_MP': [11, 'The Black Castle'],
    'S05_Yrk_EC': [12, 'A Wedding and a Funeral'],
    'H09_Not_VL': [13, 'The Escape'],
    'H10_Yor_VL': [14, 'The Letter'],
    'Str03_Yor_MK': [15, 'The March on York'],
    'H12_Not_MP': [16, 'Last Challenge']
};

function fullMissionId(id: string): string {
    return id === 'Dem_Lei_MP' ? 'S02_Lei_MP' : id === 'Dem_Lin_MP' ? 'H01_Lin_VL' : id;
}

export function isWinnableMission(id: string): boolean {
    return !['sherwood', 'sherwoodoutro'].includes(id.toLowerCase());
}

export function missionGroup(id: string): string {
    if (campaign[fullMissionId(id)] !== undefined) return 'Campaign';
    if (id.startsWith('Emb')) return 'Ambushes';
    if (id.startsWith('Str')) return 'Castle attacks';
    if (id.startsWith('Tac')) return 'Tactical missions';
    return 'Other missions';
}

export function missionLabel(id: string, fallback: string, demo = false): string {
    const entry = campaign[fullMissionId(id)];
    let title: string;
    if (entry !== undefined) title = `${String(entry[0]).padStart(2, '0')} · ${entry[1]}`;
    else if (id === 'EmbTut_FoC_EC') title = 'Ambush tutorial';
    else if (id === 'Str01_Lin_EC') title = 'Attack Lincoln';
    else {
        const number = id.match(/^(?:Emb|Tac)(\d+)/u)?.[1];
        title = number === undefined ? fallback.replace(/^Official (?:FULL|DEMO)\s+/u, '')
            : `${number} · ${id.startsWith('Emb') ? 'Ambush' : 'Tactical mission'}`;
    }
    return `${title}${demo ? ' (Demo)' : ''}`;
}

/** Apply player-facing titles and hide locations without a mission victory. */
export function browsableMetadata(metadata: BoardMetadata): BoardMetadata {
    return { ...metadata, boards: metadata.boards.map(board => ({
        ...board,
        presetName: board.presetId === 'any' ? 'All rules' : board.presetName,
        displayName: board.displayName.replaceAll('Any ruleset', 'All rules'),
        missions: board.missions.filter(mission => isWinnableMission(mission.missionId)).map(mission => ({
            ...mission, displayName: missionLabel(mission.missionId, mission.displayName, board.edition === 'demo'),
        })).sort((a, b) => {
            const groups = ['Campaign', 'Castle attacks', 'Ambushes', 'Tactical missions', 'Other missions'];
            return groups.indexOf(missionGroup(a.missionId)) - groups.indexOf(missionGroup(b.missionId))
                || a.displayName.localeCompare(b.displayName, undefined, { numeric: true });
        }),
    })).filter(board => board.missions.length > 0) };
}
