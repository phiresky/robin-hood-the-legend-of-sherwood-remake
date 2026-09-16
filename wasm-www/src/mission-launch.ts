/** A replay's edition takes precedence over ordinary game-launch options. */
export function fullGameContentBuild(
    latestBuild: string,
    requestedEdition: string | null,
    recordedRun: { readonly edition: 'full' | 'demo'; readonly runtimeBuild: string } | null,
    hasSharedReplay: boolean,
): string | null {
    if (recordedRun !== null) return recordedRun.edition === 'full' ? recordedRun.runtimeBuild : null;
    return !hasSharedReplay && requestedEdition === 'full' ? latestBuild : null;
}
