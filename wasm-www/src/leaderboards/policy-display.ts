// Human-readable simulation policy labels.
import { type RankedSimulationPolicy } from './types.js';

export function rankedSimulationPolicyLabels(policy: RankedSimulationPolicy): {
    readonly presetId: string;
    readonly presetName: string;
    readonly difficultyId: string;
    readonly difficultyName: string;
} {
    const presets = {
        standard: { presetId: 'standard', presetName: 'Standard' },
        original_parity: { presetId: 'original', presetName: 'Original' },
        custom: { presetId: 'custom', presetName: 'Custom' },
    };
    const difficulties = {
        easy: { difficultyId: 'easy', difficultyName: 'Easy' },
        medium: { difficultyId: 'normal', difficultyName: 'Normal' },
        hard: { difficultyId: 'hard', difficultyName: 'Hard' },
        legendary: { difficultyId: 'legendary', difficultyName: 'Legendary' },
        custom: { difficultyId: 'custom', difficultyName: 'Custom' },
    };
    return { ...presets[policy.preset], ...difficulties[policy.difficulty] };
}

export function rankedSimulationPolicyDisplay(policy: RankedSimulationPolicy): string {
    const labels = rankedSimulationPolicyLabels(policy);
    return `${labels.presetName} / ${labels.difficultyName} (v${policy.version})`;
}
