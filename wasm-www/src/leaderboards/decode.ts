// Strict wire decoding primitives shared by the semantic contracts.
import { type JsonObject } from './types.js';

export const CATEGORY_ORDER = ['individual_level', 'campaign'] as const;

export const METRIC_ORDER = ['original_score', 'fastest_success'] as const;

export const TAINT_ORDER = [
    'http_player_command', 'http_simulation_step', 'http_state_mutation', 'console_command',
    'cheat_command', 'headless_automation', 'replay_playback', 'state_load', 'mission_restart',
    'debug_input_injection',
] as const;

export function exactString<const Expected extends string>(
    value: unknown,
    expected: Expected,
    path: string,
): Expected {
    if (value !== expected) throw new Error(`${path} must be ${expected}`);
    return expected;
}

export function exactStringArray<const Expected extends readonly string[]>(
    value: unknown,
    path: string,
    expected: Expected,
): Expected {
    const actual = array(value, path);
    if (actual.length !== expected.length || actual.some((item, index) => item !== expected[index])) {
        throw new Error(`${path} does not match the exact canonical recipe`);
    }
    return expected;
}

export function equalArrays<T>(left: readonly T[], right: readonly T[]): boolean {
    return left.length === right.length && left.every((item, index) => item === right[index]);
}

export function protocolValuesEqual(left: unknown, right: unknown): boolean {
    if (left === right) return true;
    if (left === null || right === null || typeof left !== typeof right) return false;
    if (Array.isArray(left) || Array.isArray(right)) {
        return Array.isArray(left) && Array.isArray(right) && left.length === right.length
            && left.every((value, index) => protocolValuesEqual(value, right[index]));
    }
    if (typeof left !== 'object' || typeof right !== 'object') return false;
    const leftObject = left as Readonly<Record<string, unknown>>;
    const rightObject = right as Readonly<Record<string, unknown>>;
    const leftKeys = Object.keys(leftObject).sort();
    const rightKeys = Object.keys(rightObject).sort();
    return leftKeys.length === rightKeys.length
        && leftKeys.every((key, index) => key === rightKeys[index]
            && protocolValuesEqual(leftObject[key], rightObject[key]));
}

export function checkedAdd(left: number, right: number, path: string): number {
    const sum = left + right;
    if (!Number.isSafeInteger(sum)) throw new Error(`${path} exceeds JavaScript's exact integer range`);
    return sum;
}

export function viewerArtifactPath(value: unknown, path: string): string {
    const result = relativePath(value, path, true);
    if (!/^[A-Za-z0-9/._-]+$/u.test(result) || result.startsWith('//')) {
        throw new Error(`${path} contains characters forbidden in a viewer artifact path`);
    }
    return result;
}

export function relativePath(value: unknown, path: string, _viewer: boolean): string {
    const result = boundedString(value, path, 1024);
    if (result.startsWith('/') || result.includes('\\')
        || result.split('/').some(component => component === '' || component === '.' || component === '..')) {
        throw new Error(`${path} must be a canonical relative path`);
    }
    return result;
}

export function versionedObject(value: unknown, path: string, fields: readonly string[]): JsonObject {
    const obj = strictObject(value, path, ['schema_version', ...fields]);
    if (obj.schema_version !== 1) throw new Error(`${path}.schema_version must be 1`);
    return obj;
}

export function strictObject(value: unknown, path: string, fields: readonly string[]): JsonObject {
    const obj = object(value, path);
    assertExactKeys(obj, path, fields);
    return obj;
}

export function assertExactKeys(obj: JsonObject, path: string, fields: readonly string[]): void {
    const allowed = new Set(fields);
    const unknown = Object.keys(obj).find(field => !allowed.has(field));
    if (unknown !== undefined) throw new Error(`${path} contains unknown field ${unknown}`);
}

export function object(value: unknown, path: string): JsonObject {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        throw new Error(`${path} must be an object`);
    }
    return value as JsonObject;
}

export function array(value: unknown, path: string): readonly unknown[] {
    if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
    return value;
}

export function boolean(value: unknown, path: string): boolean {
    if (typeof value !== 'boolean') throw new Error(`${path} must be a boolean`);
    return value;
}

export function boundedString(value: unknown, path: string, maxLength: number): string {
    if (typeof value !== 'string' || value.length === 0
        || new TextEncoder().encode(value).byteLength > maxLength) {
        throw new Error(`${path} must be a non-empty string no longer than ${maxLength} bytes`);
    }
    if (value.trim() !== value) throw new Error(`${path} must not have surrounding whitespace`);
    if (/\p{Cc}|\p{Bidi_Control}/u.test(value)) throw new Error(`${path} contains forbidden control characters`);
    return value;
}

export function resourceLocaleRoot(value: unknown, path: string): string {
    if (typeof value !== 'string' || !/^[1-9][0-9]{0,7}$/u.test(value)) {
        throw new Error(`${path} must be one numeric component without a leading zero (1–8 digits)`);
    }
    return value;
}

export function opaqueId(value: unknown, path: string): string {
    return boundedString(value, path, 128);
}

export function sha256(value: unknown, path: string): string {
    if (typeof value !== 'string' || !/^[0-9a-f]{64}$/u.test(value)) {
        throw new Error(`${path} must be a lowercase SHA-256 digest`);
    }
    return value;
}

export function nullableSha256(value: unknown, path: string): string | null {
    return value === null ? null : nonzeroSha256(value, path);
}

export function nonzeroSha256(value: unknown, path: string): string {
    const digest = sha256(value, path);
    if (/^0{64}$/u.test(digest)) throw new Error(`${path} must not be the zero digest`);
    return digest;
}

export function publicKey(value: unknown, path: string): string {
    return nonzeroSha256(value, path);
}

export function nonzeroHex(value: unknown, path: string, exactLength: number): string {
    if (typeof value !== 'string' || value.length !== exactLength || !/^[0-9a-f]+$/u.test(value)
        || new RegExp(`^0{${exactLength}}$`, 'u').test(value)) {
        throw new Error(`${path} must be a non-zero ${exactLength}-character lowercase hexadecimal value`);
    }
    return value;
}

export function gitCommit(value: unknown, path: string): string {
    if (typeof value !== 'string' || !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u.test(value)) {
        throw new Error(`${path} must be a full lowercase 40- or 64-character Git object ID`);
    }
    return value;
}

export function safeInteger(value: unknown, path: string): number {
    if (typeof value !== 'number' || !Number.isSafeInteger(value)) throw new Error(`${path} must be an exact safe integer`);
    return value;
}

export function positiveInteger(value: unknown, path: string): number {
    const result = safeInteger(value, path);
    if (result <= 0) throw new Error(`${path} must be positive`);
    return result;
}

export function nonNegativeInteger(value: unknown, path: string): number {
    const result = safeInteger(value, path);
    if (result < 0) throw new Error(`${path} must be non-negative`);
    return result;
}

export function u64DecimalString(value: unknown, path: string): string {
    if (typeof value !== 'string' || !/^(?:0|[1-9][0-9]*)$/u.test(value)) {
        throw new Error(`${path} must be a canonical unsigned decimal string`);
    }
    const parsed = BigInt(value);
    if (parsed > 0xffff_ffff_ffff_ffffn) {
        throw new Error(`${path} exceeds the unsigned 64-bit range`);
    }
    return value;
}

export function u16(value: unknown, path: string): number {
    const result = nonNegativeInteger(value, path);
    if (result > 0xffff) throw new Error(`${path} exceeds the unsigned 16-bit range`);
    return result;
}

export function u16Positive(value: unknown, path: string): number {
    const result = u16(value, path);
    if (result === 0) throw new Error(`${path} must be positive`);
    return result;
}

export function replaySeatCount(value: unknown, path: string): number {
    const result = u16Positive(value, path);
    if (result > 4) throw new Error(`${path} exceeds the replay's four-seat limit`);
    return result;
}

export function multiplayerSeatCount(value: unknown, path: string): number {
    const result = replaySeatCount(value, path);
    if (result < 2) throw new Error(`${path} must describe at least two multiplayer seats`);
    return result;
}

export function participantCount(value: unknown, path: string): number {
    const result = u16Positive(value, path);
    if (result > 1024) throw new Error(`${path} exceeds the participant-instance limit`);
    return result;
}

export function u32(value: unknown, path: string): number {
    const result = nonNegativeInteger(value, path);
    if (result > 0xffff_ffff) throw new Error(`${path} exceeds the unsigned 32-bit range`);
    return result;
}

export function u32Positive(value: unknown, path: string): number {
    const result = u32(value, path);
    if (result === 0) throw new Error(`${path} must be positive`);
    return result;
}

export function i32(value: unknown, path: string): number {
    const result = safeInteger(value, path);
    if (result < -0x8000_0000 || result > 0x7fff_ffff) throw new Error(`${path} exceeds the signed 32-bit range`);
    return result;
}

export function nullablePositiveInteger(value: unknown, path: string): number | null {
    return value === null ? null : positiveInteger(value, path);
}

export function positiveUnixMilliseconds(value: unknown, path: string): number {
    const result = positiveInteger(value, path);
    if (!Number.isFinite(new Date(result).getTime())) throw new Error(`${path} is outside the browser date range`);
    return result;
}

export function enumeration<const T extends readonly string[]>(value: unknown, choices: T, path: string): T[number] {
    if (typeof value !== 'string' || !choices.includes(value)) {
        throw new Error(`${path} must be one of: ${choices.join(', ')}`);
    }
    return value;
}

export function strictlyIncreasingByOrder<const T extends string>(values: readonly T[], order: readonly T[]): boolean {
    return values.every((value, index) => index === 0
        || order.indexOf(values[index - 1] as T) < order.indexOf(value));
}

export function strictlySorted(values: readonly string[]): boolean {
    return values.every((value, index) => index === 0 || (values[index - 1] ?? '') < value);
}
