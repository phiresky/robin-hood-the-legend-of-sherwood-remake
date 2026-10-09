import type { ContentEdition } from './types.js';

export const rulesHelp = 'All rules combines solo rules choices. Co-op ranks cooperative runs separately, with all team sizes and gameplay settings together. Original uses classic gameplay settings. Standard uses the remake’s updated gameplay, including reusable cloaks and revised combat. Choose a rule set to compare runs with the same rules, then choose a difficulty.';

export function missionPlayUrl(currentUrl: string, edition: ContentEdition, missionId: string): string {
    const url = new URL('../', currentUrl);
    url.search = '';
    url.hash = '';
    url.searchParams.set('mission', missionId);
    url.searchParams.set('edition', edition);
    return url.toString();
}
