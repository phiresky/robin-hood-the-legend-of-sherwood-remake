// Participant identity, roster, metric and timing invariants.
import { sha256 as nobleSha256 } from '@noble/hashes/sha256';
import {
    type BoardMetricValue,
    type TickDuration,
    type RunMetrics,
    type PublicParticipant,
    type AggregatePublicParticipant,
    type ParsedCampaignSessionKind,
    type ParsedAchievementDecision,
    type Achievement,
    type InputProvenance,
} from './types.js';
import {
    object,
    enumeration,
    METRIC_ORDER,
    assertExactKeys,
    nonNegativeInteger,
    strictObject,
    positiveInteger,
    safeInteger,
    u16,
    publicKey,
    boundedString,
} from './decode.js';
import { bytesHex } from './canonical.js';

export function parseMetricValue(value: unknown, path: string): BoardMetricValue {
    const obj = object(value, path);
    const metric = enumeration(obj.metric, METRIC_ORDER, `${path}.metric`);
    if (metric === 'original_score') {
        assertExactKeys(obj, path, ['metric', 'points']);
        return { metric, points: nonNegativeInteger(obj.points, `${path}.points`) };
    }
    assertExactKeys(obj, path, ['metric', 'active_simulation_ticks', 'tick_duration']);
    const tickDuration = parseTickDuration(obj.tick_duration, `${path}.tick_duration`);
    return {
        metric,
        activeSimulationTicks: nonNegativeInteger(obj.active_simulation_ticks, `${path}.active_simulation_ticks`),
        tickDuration,
        tickDurationMicros: tickDuration.numeratorMicros / tickDuration.denominator,
    };
}

export function parseTickDuration(value: unknown, path: string): TickDuration {
    const obj = strictObject(value, path, ['numerator_micros', 'denominator']);
    const numeratorMicros = positiveInteger(obj.numerator_micros, `${path}.numerator_micros`);
    const denominator = positiveInteger(obj.denominator, `${path}.denominator`);
    if (greatestCommonDivisor(numeratorMicros, denominator) !== 1) {
        throw new Error(`${path} must be a reduced canonical fraction`);
    }
    return { numeratorMicros, denominator };
}

export function greatestCommonDivisor(left: number, right: number): number {
    let a = left;
    let b = right;
    while (b !== 0) {
        const remainder = a % b;
        a = b;
        b = remainder;
    }
    return a;
}

export function parseRunMetrics(value: unknown, path: string): RunMetrics {
    const obj = strictObject(value, path, [
        'original_score_delta', 'active_simulation_ticks', 'ransom_collected',
    ]);
    return {
        originalScoreDelta: safeInteger(obj.original_score_delta, `${path}.original_score_delta`),
        activeSimulationTicks: nonNegativeInteger(obj.active_simulation_ticks, `${path}.active_simulation_ticks`),
        ransomCollected: nonNegativeInteger(obj.ransom_collected, `${path}.ransom_collected`),
    };
}

export function parseParticipant(value: unknown, path: string): PublicParticipant {
    const obj = strictObject(value, path, [
        'seat', 'username', 'public_key', 'public_key_fingerprint',
    ]);
    const seat = u16(obj.seat, `${path}.seat`);
    if (seat >= 64) throw new Error(`${path}.seat exceeds the replay seat range`);
    const key = publicKey(obj.public_key, `${path}.public_key`);
    return {
        seat,
        username: boundedString(obj.username, `${path}.username`, 48),
        publicKey: key,
        publicKeyFingerprint: validatePublicFingerprint(obj.public_key_fingerprint, key, `${path}.public_key_fingerprint`),
    };
}

export function parseAggregateParticipant(value: unknown, path: string): AggregatePublicParticipant {
    const obj = strictObject(value, path, [
        'current_display_name', 'public_key', 'public_key_fingerprint',
    ]);
    const key = publicKey(obj.public_key, `${path}.public_key`);
    return {
        currentDisplayName: boundedString(obj.current_display_name, `${path}.current_display_name`, 48),
        publicKey: key,
        publicKeyFingerprint: validatePublicFingerprint(
            obj.public_key_fingerprint,
            key,
            `${path}.public_key_fingerprint`,
        ),
    };
}

export function compareSeatPublicKey(
    left: { readonly seat: number; readonly publicKey: string },
    right: { readonly seat: number; readonly publicKey: string },
): number {
    return left.seat - right.seat || left.publicKey.localeCompare(right.publicKey);
}

export function validateRoster(
    maxConcurrentPlayers: number,
    participantInstanceCount: number,
    namedParticipantInstanceCount: number,
    namedParticipants: readonly PublicParticipant[],
    aggregateNamedParticipants: readonly AggregatePublicParticipant[],
    anonymousParticipantInstanceCount: number,
    missionInstancesRequired: boolean,
    path: string,
): void {
    if (participantInstanceCount < maxConcurrentPlayers
        || namedParticipantInstanceCount + anonymousParticipantInstanceCount !== participantInstanceCount) {
        throw new Error(`${path} contains invalid, non-canonical, or out-of-range participant claims`);
    }
    if (missionInstancesRequired) {
        if (aggregateNamedParticipants.length !== 0
            || namedParticipants[0]?.seat !== 0
            || namedParticipantInstanceCount !== namedParticipants.length
            || new Set(namedParticipants.map(participant => participant.publicKey)).size
                !== namedParticipants.length
            || namedParticipants.some((participant, index) => {
                const previous = namedParticipants[index - 1];
                return index > 0 && previous !== undefined
                    && compareSeatPublicKey(previous, participant) >= 0;
            })) {
            throw new Error(`${path} contains invalid mission participant claims`);
        }
        return;
    }
    if (namedParticipants.length !== 0 || aggregateNamedParticipants.length === 0
        || aggregateNamedParticipants.length > namedParticipantInstanceCount
        || aggregateNamedParticipants.some((participant, index) => index > 0
            && (aggregateNamedParticipants[index - 1]?.publicKey ?? '') >= participant.publicKey)) {
        throw new Error(`${path} contains an invalid aggregate participant roster`);
    }
}

export function validatePublicFingerprint(value: unknown, publicKeyHex: string, path: string): string {
    const fingerprint = boundedString(value, path, 80);
    if (!/^[0-9a-f]{32}$/u.test(fingerprint)) {
        throw new Error(`${path} must be a 32-character lowercase hexadecimal fingerprint`);
    }
    const domain = new TextEncoder().encode('robinhood-run-key-fingerprint-v1\0');
    const key = hexBytes(publicKeyHex);
    const input = new Uint8Array(domain.length + key.length);
    input.set(domain);
    input.set(key, domain.length);
    const expected = bytesHex(nobleSha256(input).slice(0, 16));
    if (fingerprint !== expected) {
        throw new Error(`${path} does not match the canonical public-key fingerprint`);
    }
    return fingerprint;
}

export function hexBytes(value: string): Uint8Array {
    const output = new Uint8Array(value.length / 2);
    for (let index = 0; index < output.length; index += 1) {
        output[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
    }
    return output;
}

export function metricsEqual(left: RunMetrics, right: RunMetrics): boolean {
    return left.originalScoreDelta === right.originalScoreDelta
        && left.activeSimulationTicks === right.activeSimulationTicks
        && left.ransomCollected === right.ransomCollected;
}

export function campaignSessionKindsEqual(
    left: ParsedCampaignSessionKind,
    right: ParsedCampaignSessionKind,
): boolean {
    return left.kind === right.kind
        && (left.kind === 'field_mission'
            ? right.kind === 'field_mission' && left.missionId === right.missionId
            : right.kind === 'headquarters' && left.hqSequence === right.hqSequence);
}

export function achievementsEqual(
    verified: readonly ParsedAchievementDecision[],
    summaries: readonly Achievement[],
): boolean {
    return verified.length === summaries.length && verified.every((achievement, index) => {
        const summary = summaries[index];
        return summary !== undefined
            && achievement.id === summary.id
            && achievement.evaluation === summary.evaluation;
    });
}

export function provenanceEqual(left: InputProvenance, right: InputProvenance): boolean {
    return JSON.stringify(left) === JSON.stringify(right);
}
