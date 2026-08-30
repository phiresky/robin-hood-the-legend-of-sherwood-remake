const JOIN_CODE_PREFIX = 'rhmp3-';
const JOIN_TICKET_SCHEMA = 3;
const NET_PROTOCOL_VERSION = 27;
const TRANSPORT = 'iroh-relay-websocket';
const SIGNING_DOMAIN = new TextEncoder().encode('robinhood/browser-join-ticket/v3\0');
const MAX_JOIN_CODE_BYTES = 16 * 1024;
const INVITATION_LIFETIME_SECONDS = 30 * 60;
const MAX_CLOCK_SKEW_SECONDS = 2 * 60;

const TICKET_KEYS = [
    'schema',
    'transport',
    'net_protocol',
    'engine_version',
    'host_endpoint_id',
    'host_public_key',
    'relay_url',
    'session_id',
    'issued_at_epoch_s',
    'expires_at_epoch_s',
    'content_edition',
    'content_identity_sha256',
    'mission_id',
    'mission_profile_id',
    'expected_players',
] as const;

export type BrowserContentEdition = 'demo' | 'full';

export type BrowserJoinTicketPayload = {
    readonly schema: number;
    readonly transport: string;
    readonly net_protocol: number;
    readonly engine_version: string;
    readonly host_endpoint_id: string;
    readonly host_public_key: string;
    readonly relay_url: string;
    readonly session_id: string;
    readonly issued_at_epoch_s: number;
    readonly expires_at_epoch_s: number;
    readonly content_edition: BrowserContentEdition;
    readonly content_identity_sha256: string;
    readonly mission_id: string;
    readonly mission_profile_id: number | null;
    readonly expected_players: number;
};

export type VerifiedBrowserJoinTicket = {
    readonly code: string;
    readonly payload: BrowserJoinTicketPayload;
    readonly canonicalPayload: Uint8Array;
    readonly signature: Uint8Array;
};

function fail(message: string): never {
    throw new Error(`browser invitation: ${message}`);
}

function decodeBase64Url(label: string, value: unknown, expectedLength?: number): Uint8Array {
    if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value)) {
        fail(`${label} is not canonical base64url`);
    }
    const padded = value.replaceAll('-', '+').replaceAll('_', '/')
        + '='.repeat((4 - value.length % 4) % 4);
    let binary: string;
    try {
        binary = atob(padded);
    } catch {
        fail(`${label} is not valid base64url`);
    }
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    if (encodeBase64Url(bytes) !== value) {
        fail(`${label} is not canonical base64url`);
    }
    if (expectedLength !== undefined && bytes.byteLength !== expectedLength) {
        fail(`${label} must be ${expectedLength} bytes`);
    }
    return bytes;
}

function encodeBase64Url(bytes: Uint8Array): string {
    let binary = '';
    for (let index = 0; index < bytes.byteLength; index += 1) {
        binary += String.fromCharCode(bytes[index] as number);
    }
    return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function requireInteger(label: string, value: unknown): number {
    if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
        fail(`${label} must be a non-negative safe integer`);
    }
    return value;
}

function requireString(label: string, value: unknown, maxUtf8Bytes: number): string {
    if (typeof value !== 'string' || value.length === 0) {
        fail(`${label} must be a non-empty string`);
    }
    if (new TextEncoder().encode(value).byteLength > maxUtf8Bytes) {
        fail(`${label} exceeds its ${maxUtf8Bytes}-byte limit`);
    }
    return value;
}

function parseCanonicalPayload(bytes: Uint8Array): BrowserJoinTicketPayload {
    let text: string;
    try {
        text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    } catch {
        fail('payload is not valid UTF-8');
    }
    let raw: unknown;
    try {
        raw = JSON.parse(text) as unknown;
    } catch {
        fail('payload is not valid JSON');
    }
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
        fail('payload must be an object');
    }
    const object = raw as Record<string, unknown>;
    const keys = Object.keys(object);
    if (keys.length !== TICKET_KEYS.length || keys.some((key, index) => key !== TICKET_KEYS[index])) {
        fail('payload fields are missing, unknown, duplicated, or out of canonical order');
    }
    if (JSON.stringify(object) !== text) {
        fail('payload JSON is not canonical');
    }

    const schema = requireInteger('schema', object.schema);
    const netProtocol = requireInteger('network protocol', object.net_protocol);
    const issuedAt = requireInteger('issued-at time', object.issued_at_epoch_s);
    const expiresAt = requireInteger('expiry time', object.expires_at_epoch_s);
    const expectedPlayers = requireInteger('expected player count', object.expected_players);
    const engineVersion = requireString('engine version', object.engine_version, 64);
    const endpointId = requireString('host endpoint id', object.host_endpoint_id, 64);
    const hostPublicKey = requireString('host public key', object.host_public_key, 64);
    const relayUrl = requireString('relay URL', object.relay_url, 2048);
    const sessionId = requireString('session id', object.session_id, 64);
    const missionId = requireString('mission id', object.mission_id, 255);
    const contentIdentity = requireString(
        'content identity',
        object.content_identity_sha256,
        64,
    );

    if (schema !== JOIN_TICKET_SCHEMA) fail(`unsupported schema ${schema}`);
    if (object.transport !== TRANSPORT) fail(`unsupported transport ${String(object.transport)}`);
    if (netProtocol !== NET_PROTOCOL_VERSION) fail(`unsupported network protocol ${netProtocol}`);
    if (!/^[0-9a-f]{7,40}$/.test(engineVersion)) fail('engine version must be a lowercase git hash');
    const publicKey = decodeBase64Url('host public key', hostPublicKey, 32);
    if (endpointId !== Array.from(publicKey, (byte) => byte.toString(16).padStart(2, '0')).join('')) {
        fail('host endpoint id does not match the signed public key');
    }
    decodeBase64Url('session id', sessionId, 32);
    if (/^A+$/.test(sessionId)) fail('session id must be non-zero');
    if (expiresAt - issuedAt !== INVITATION_LIFETIME_SECONDS) {
        fail(`invitation lifetime must be exactly ${INVITATION_LIFETIME_SECONDS} seconds`);
    }
    if (object.content_edition !== 'demo' && object.content_edition !== 'full') {
        fail('content edition must be demo or full');
    }
    if (!/^[0-9a-f]{64}$/.test(contentIdentity)) {
        fail('content identity must be a lowercase SHA-256 digest');
    }
    if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(missionId) || missionId.includes('..')) {
        fail('mission id must be one safe basename');
    }
    if (object.mission_profile_id !== null) {
        requireInteger('mission profile id', object.mission_profile_id);
    }
    if (expectedPlayers < 1 || expectedPlayers > 4) fail('expected player count must be 1 through 4');

    let relay: URL;
    try {
        relay = new URL(relayUrl);
    } catch {
        fail('relay URL is invalid');
    }
    if (
        relay.protocol !== 'https:'
        || relay.hostname.length === 0
        || relay.username.length > 0
        || relay.password.length > 0
        || relay.search.length > 0
        || relay.hash.length > 0
        || relay.toString() !== relayUrl
    ) {
        fail('relay URL must be canonical HTTPS without credentials, query, or fragment');
    }
    return object as BrowserJoinTicketPayload;
}

export async function authenticateBrowserJoinTicket(
    code: string,
): Promise<VerifiedBrowserJoinTicket> {
    if (new TextEncoder().encode(code).byteLength > MAX_JOIN_CODE_BYTES) {
        fail(`join code exceeds the ${MAX_JOIN_CODE_BYTES}-byte limit`);
    }
    if (!code.startsWith(JOIN_CODE_PREFIX)) fail(`join code must start with ${JOIN_CODE_PREFIX}`);
    const envelope = code.slice(JOIN_CODE_PREFIX.length);
    const parts = envelope.split('.');
    if (parts.length !== 2 || parts[0]?.length === 0 || parts[1]?.length === 0) {
        fail('join code has a malformed signed envelope');
    }
    const payloadBytes = decodeBase64Url('payload', parts[0]);
    const signature = decodeBase64Url('signature', parts[1], 64);
    const payload = parseCanonicalPayload(payloadBytes);
    // WebCrypto's BufferSource overload deliberately excludes
    // SharedArrayBuffer-backed views. Copy hostile decoded input into a fresh
    // ordinary ArrayBuffer before it crosses that boundary.
    const publicKeyBytes = Uint8Array.from(
        decodeBase64Url('host public key', payload.host_public_key, 32),
    );
    const signatureBytes = Uint8Array.from(signature);
    let publicKey: CryptoKey;
    try {
        publicKey = await crypto.subtle.importKey('raw', publicKeyBytes, 'Ed25519', false, ['verify']);
    } catch (error) {
        fail(`Ed25519 verification is unavailable (${String(error)})`);
    }
    const message = new Uint8Array(SIGNING_DOMAIN.byteLength + payloadBytes.byteLength);
    message.set(SIGNING_DOMAIN);
    message.set(payloadBytes, SIGNING_DOMAIN.byteLength);
    if (!await crypto.subtle.verify('Ed25519', publicKey, signatureBytes, message)) {
        fail('host signature is invalid');
    }
    return { code, payload, canonicalPayload: payloadBytes, signature };
}

export function validateBrowserJoinTicketUse(
    ticket: VerifiedBrowserJoinTicket,
    nowEpochSeconds = Math.floor(Date.now() / 1000),
    redeemed = false,
): void {
    if (ticket.payload.issued_at_epoch_s > nowEpochSeconds + MAX_CLOCK_SKEW_SECONDS) {
        fail('invitation was issued too far in the future');
    }
    if (!redeemed && nowEpochSeconds >= ticket.payload.expires_at_epoch_s) {
        fail('invitation expired before first use');
    }
}

export async function verifyBrowserJoinTicket(
    code: string,
    nowEpochSeconds = Math.floor(Date.now() / 1000),
    redeemed = false,
): Promise<VerifiedBrowserJoinTicket> {
    const ticket = await authenticateBrowserJoinTicket(code);
    validateBrowserJoinTicketUse(ticket, nowEpochSeconds, redeemed);
    return ticket;
}

/**
 * Capture a public invitation from the URL fragment and remove it before any
 * artifact or content request. Fragment data is never sent in HTTP requests;
 * replaceState also removes it from the current history entry.
 */
export function captureAndScrubBrowserJoinCode(locationUrl: URL): string | undefined {
    if (locationUrl.hash.length === 0) return undefined;
    const fragment = locationUrl.hash.slice(1);
    if (!fragment.startsWith('join=')) return undefined;
    const code = fragment.slice('join='.length);
    if (code.length === 0 || code.includes('&')) fail('join fragment is malformed');
    const scrubbed = `${locationUrl.pathname}${locationUrl.search}`;
    history.replaceState(history.state, '', scrubbed);
    return code;
}

export const browserJoinTicketConstants = {
    invitationLifetimeSeconds: INVITATION_LIFETIME_SECONDS,
    maxClockSkewSeconds: MAX_CLOCK_SKEW_SECONDS,
    netProtocolVersion: NET_PROTOCOL_VERSION,
    schema: JOIN_TICKET_SCHEMA,
} as const;
