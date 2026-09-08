// Canonical JSON and content-address verification; keep aligned with robin_run_protocol.
import { sha256 as nobleSha256 } from '@noble/hashes/sha256';
import { nonzeroSha256, object, safeInteger, boundedString } from './decode.js';
import { type CanonicalValue } from './types.js';

declare const digestVerified: unique symbol;
/** Parsed data whose original wire document matched the requested content address. */
export type DigestVerified<T> = T & { readonly [digestVerified]: true };

export async function verifyCanonicalDocument<T>(
    wire: unknown,
    parsed: T,
    expected: string,
    path: string,
): Promise<DigestVerified<T>> {
    await requireCanonicalDigest(wire, expected, path);
    return parsed as DigestVerified<T>;
}

/** Mirrors robin_run_protocol::canonical_json_bytes for JSON-safe integer documents. */
export async function canonicalDocumentSha256(value: unknown): Promise<string> {
    const bytes = new TextEncoder().encode(canonicalJson(value, '$', 0));
    const digest = await crypto.subtle.digest('SHA-256', bytes);
    return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, '0')).join('');
}

export function canonicalDocumentSha256Sync(value: unknown): string {
    return bytesHex(nobleSha256(new TextEncoder().encode(canonicalJson(value, '$', 0))));
}

export function bytesHex(value: Uint8Array): string {
    return [...value].map(byte => byte.toString(16).padStart(2, '0')).join('');
}

export async function requireCanonicalDigest(value: unknown, expected: string, path: string): Promise<void> {
    const expectedDigest = nonzeroSha256(expected, `${path} expected digest`);
    const actual = await canonicalDocumentSha256(value);
    if (actual !== expectedDigest) throw new Error(`${path} canonical SHA-256 does not match its content address`);
}

export function canonicalJson(value: unknown, path: string, depth: number): string {
    if (depth > 128) throw new Error(`${path} exceeds the supported canonical document depth`);
    if (value === null) return 'null';
    if (typeof value === 'boolean') return value ? 'true' : 'false';
    if (typeof value === 'string') return JSON.stringify(value);
    if (typeof value === 'number') {
        if (!Number.isSafeInteger(value) || Object.is(value, -0)) {
            throw new Error(`${path} contains a non-canonical or inexact JSON number`);
        }
        return String(value);
    }
    if (Array.isArray(value)) {
        return `[${value.map((item, index) => canonicalJson(item, `${path}[${index}]`, depth + 1)).join(',')}]`;
    }
    const obj = object(value, path);
    const entries = Object.keys(obj).sort(compareUtf8).map(key =>
        `${JSON.stringify(key)}:${canonicalJson(obj[key], `${path}.${key}`, depth + 1)}`);
    return `{${entries.join(',')}}`;
}

export function compareUtf8(left: string, right: string): number {
    const encoder = new TextEncoder();
    const leftBytes = encoder.encode(left);
    const rightBytes = encoder.encode(right);
    const sharedLength = Math.min(leftBytes.length, rightBytes.length);
    for (let index = 0; index < sharedLength; index += 1) {
        const difference = (leftBytes[index] ?? 0) - (rightBytes[index] ?? 0);
        if (difference !== 0) return difference;
    }
    return leftBytes.length - rightBytes.length;
}

export function validateCanonicalValue(
    value: unknown,
    path: string,
    depth: number,
    maximumDepth: number,
): void {
    if (depth > maximumDepth) throw new Error(`${path} exceeds the canonical value depth limit`);
    if (value === null || typeof value === 'boolean') return;
    if (typeof value === 'string') {
        if (new TextEncoder().encode(value).byteLength > 16 * 1024) throw new Error(`${path} string is too long`);
        return;
    }
    if (typeof value === 'number') {
        safeInteger(value, path);
        if (Object.is(value, -0)) throw new Error(`${path} must not be negative zero`);
        return;
    }
    if (Array.isArray(value)) {
        value.forEach((item, index) => validateCanonicalValue(
            item,
            `${path}[${index}]`,
            depth + 1,
            maximumDepth,
        ));
        return;
    }
    const obj = object(value, path);
    for (const [key, item] of Object.entries(obj)) {
        boundedString(key, `${path} key`, 256);
        validateCanonicalValue(item, `${path}.${key}`, depth + 1, maximumDepth);
    }
}

export function parseCanonicalMap(
    value: unknown,
    path: string,
    maximumDepth = 64,
    maximumTopLevelKeyBytes = 256,
): Readonly<Record<string, CanonicalValue>> {
    const obj = object(value, path);
    for (const [key, item] of Object.entries(obj)) {
        boundedString(key, `${path} key`, maximumTopLevelKeyBytes);
        validateCanonicalValue(item, `${path}.${key}`, 0, maximumDepth);
    }
    return obj as Readonly<Record<string, CanonicalValue>>;
}
