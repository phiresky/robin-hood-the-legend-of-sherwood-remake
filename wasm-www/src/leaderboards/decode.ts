// Strict wire decoding primitives shared by the semantic contracts.
import { type JsonObject } from './types.js';

export const METRIC_ORDER = ['original_score', 'fastest_success'] as const;

/** robin_run_types::MAX_REPLAY_SEATS_V1 */
export const MAX_REPLAY_SEATS = 4;

export function versionedObject(value: unknown, path: string, fields: readonly string[]): JsonObject {
    const obj = strictObject(value, path, ['schema_version', ...fields]);
    if (obj.schema_version !== 1) throw new Error(`${path}.schema_version must be 1`);
    return obj;
}

/** Documents whose shape changed with the simplified ranked protocol. */
export function versionedObjectV2(
    value: unknown,
    path: string,
    fields: readonly string[],
    optional: readonly string[] = [],
): JsonObject {
    const obj = strictObject(value, path, ['schema_version', ...fields], optional);
    if (obj.schema_version !== 2) throw new Error(`${path}.schema_version must be 2`);
    return obj;
}

/**
 * Rejects unknown fields (serde `deny_unknown_fields`). Every field must be
 * present except `optional` ones (serde `skip_serializing_if`).
 */
export function strictObject(
    value: unknown,
    path: string,
    fields: readonly string[],
    optional: readonly string[] = [],
): JsonObject {
    const obj = object(value, path);
    assertExactKeys(obj, path, [...fields, ...optional]);
    const missing = fields.find(field => !Object.hasOwn(obj, field));
    if (missing !== undefined) throw new Error(`${path} is missing required field ${missing}`);
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

export function opaqueId(value: unknown, path: string): string {
    return boundedString(value, path, 128);
}

export function sha256(value: unknown, path: string): string {
    if (typeof value !== 'string' || !/^[0-9a-f]{64}$/u.test(value)) {
        throw new Error(`${path} must be a lowercase SHA-256 digest`);
    }
    return value;
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

export function u16(value: unknown, path: string): number {
    const result = nonNegativeInteger(value, path);
    if (result > 0xffff) throw new Error(`${path} exceeds the unsigned 16-bit range`);
    return result;
}

export function replaySeatCount(value: unknown, path: string): number {
    const result = u16(value, path);
    if (result === 0 || result > MAX_REPLAY_SEATS) {
        throw new Error(`${path} must be between 1 and ${MAX_REPLAY_SEATS} players`);
    }
    return result;
}

export function u32(value: unknown, path: string): number {
    const result = nonNegativeInteger(value, path);
    if (result > 0xffff_ffff) throw new Error(`${path} exceeds the unsigned 32-bit range`);
    return result;
}

export function i32(value: unknown, path: string): number {
    const result = safeInteger(value, path);
    if (result < -0x8000_0000 || result > 0x7fff_ffff) throw new Error(`${path} exceeds the signed 32-bit range`);
    return result;
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
