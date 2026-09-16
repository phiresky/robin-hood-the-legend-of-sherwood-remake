// Participant identity, metric and timing invariants.
import { sha256 as nobleSha256 } from '@noble/hashes/sha2.js';
import {
    type BoardMetricValue,
    type TickDuration,
    type RunMetrics,
    type PublicParticipant,
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

/** BoardMetricValueV2, tagged by `metric`. */
export function parseMetricValue(value: unknown, path: string): BoardMetricValue {
    const obj = object(value, path);
    const metric = enumeration(obj.metric, METRIC_ORDER, `${path}.metric`);
    if (metric === 'original_score') {
        assertExactKeys(obj, path, ['metric', 'points']);
        const points = nonNegativeInteger(obj.points, `${path}.points`);
        if (points > 0xffff_ffff) throw new Error(`${path}.points exceeds the original score range`);
        return { metric, points };
    }
    assertExactKeys(obj, path, ['metric', 'active_simulation_ticks']);
    return {
        metric,
        activeSimulationTicks: nonNegativeInteger(obj.active_simulation_ticks, `${path}.active_simulation_ticks`),
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
        ransomCollected: safeInteger(obj.ransom_collected, `${path}.ransom_collected`),
    };
}

export function parseParticipant(value: unknown, path: string): PublicParticipant {
    const obj = strictObject(value, path, [
        'seat', 'username', 'public_key', 'public_key_fingerprint',
    ]);
    const key = publicKey(obj.public_key, `${path}.public_key`);
    return {
        seat: u16(obj.seat, `${path}.seat`),
        username: boundedString(obj.username, `${path}.username`, 48),
        publicKey: key,
        publicKeyFingerprint: validatePublicFingerprint(obj.public_key_fingerprint, key, `${path}.public_key_fingerprint`),
    };
}

/** A named uploader always occupies the host seat; null means anonymous disclosure. */
export function parseUploader(value: unknown, path: string): PublicParticipant | null {
    if (value === null) return null;
    const uploader = parseParticipant(value, path);
    if (uploader.seat !== 0) throw new Error(`${path} must occupy the host seat 0`);
    return uploader;
}

export function validateParticipantCounts(
    maxConcurrentPlayers: number,
    participantInstanceCount: number,
    path: string,
): void {
    if (participantInstanceCount < maxConcurrentPlayers) {
        throw new Error(`${path}.participant_instance_count is smaller than its concurrent player count`);
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
