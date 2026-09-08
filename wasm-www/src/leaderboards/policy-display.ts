// Human-readable simulation policy labels.
import { type RankedSimulationPolicy } from './types.js';

export function rankedSimulationPolicyLabels(policy: RankedSimulationPolicy): {
    readonly presetId: 'standard' | 'original';
    readonly presetName: 'Standard' | 'Original';
    readonly difficultyId: 'easy' | 'normal' | 'hard';
    readonly difficultyName: 'Easy' | 'Normal' | 'Hard';
} {
    const preset = policy.preset === 'standard'
        ? { presetId: 'standard' as const, presetName: 'Standard' as const }
        : { presetId: 'original' as const, presetName: 'Original' as const };
    const difficulty = policy.difficulty === 'easy'
        ? { difficultyId: 'easy' as const, difficultyName: 'Easy' as const }
        : policy.difficulty === 'medium'
            ? { difficultyId: 'normal' as const, difficultyName: 'Normal' as const }
            : { difficultyId: 'hard' as const, difficultyName: 'Hard' as const };
    return { ...preset, ...difficulty };
}

export function rankedSimulationPolicyDisplay(policy: RankedSimulationPolicy): string {
    const labels = rankedSimulationPolicyLabels(policy);
    return `${labels.presetName} / ${labels.difficultyName} (v${policy.version})`;
}
