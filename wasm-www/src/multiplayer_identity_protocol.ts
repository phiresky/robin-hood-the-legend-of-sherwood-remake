export const MULTIPLAYER_IDENTITY_PROTOCOL = 'robinhood.multiplayer-identity.v1' as const;
export const MULTIPLAYER_GAME_ORIGIN = 'https://robinhood.phiresky.xyz' as const;

const DATABASE_NAME = 'robinhood-multiplayer-identity-v1';
const DATABASE_VERSION = 1;
const IDENTITY_STORE = 'identity';
const REDEMPTION_STORE = 'redemptions';
const IDENTITY_KEY = 'browser-seat-owner-v1';
const REQUEST_ID_RE = /^[0-9a-f]{32}$/;
const BASE64URL_32_RE = /^[A-Za-z0-9_-]{43}$/;
const ENDPOINT_ID_RE = /^[0-9a-f]{64}$/;
const SEAT_PROOF_DOMAIN = new TextEncoder().encode('robinhood/browser-seat-proof/v1\0');

type IdentityOperation = 'status' | 'was_redeemed' | 'mark_redeemed' | 'sign_seat_proof';

export type MultiplayerIdentityRequest = {
    readonly protocol: typeof MULTIPLAYER_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly operation: IdentityOperation;
    readonly sessionId?: string;
    readonly hostEndpointId?: string;
    readonly transportEndpointId?: string;
};

export type MultiplayerIdentityResponse = {
    readonly protocol: typeof MULTIPLAYER_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly ok: true;
    readonly result: unknown;
} | {
    readonly protocol: typeof MULTIPLAYER_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly ok: false;
    readonly error: { readonly code: string; readonly message: string };
};

export type MultiplayerIdentitySigner = {
    readonly status: () => Promise<{ readonly publicKey: string; readonly persistent: boolean | null }>;
    readonly wasRedeemed: (sessionId: string) => Promise<boolean>;
    readonly markRedeemed: (sessionId: string) => Promise<void>;
    readonly signSeatProof: (
        sessionId: string,
        hostEndpointId: string,
        transportEndpointId: string,
    ) => Promise<string>;
};

type StoredIdentity = {
    readonly name: typeof IDENTITY_KEY;
    readonly publicKey: CryptoKey;
    readonly privateKey: CryptoKey;
};

class ProtocolError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = 'ProtocolError';
        this.code = code;
    }
}

function fail(code: string, message: string): never {
    throw new ProtocolError(code, message);
}

function ownRecord(value: unknown): Record<string, unknown> | null {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
    const prototype = Object.getPrototypeOf(value) as unknown;
    if (prototype !== Object.prototype && prototype !== null) return null;
    return value as Record<string, unknown>;
}

function exactKeys(object: Record<string, unknown>, keys: readonly string[]): boolean {
    const actual = Object.keys(object).sort();
    const expected = [...keys].sort();
    return actual.length === expected.length
        && actual.every((key, index) => key === expected[index]);
}

function canonicalSessionId(value: unknown): string {
    if (typeof value !== 'string' || !BASE64URL_32_RE.test(value) || /^A+$/.test(value)) {
        fail('invalid_session_id', 'Multiplayer session id must be canonical non-zero base64url');
    }
    const bytes = decodeBase64Url(value);
    if (bytes.byteLength !== 32 || encodeBase64Url(bytes) !== value) {
        fail('invalid_session_id', 'Multiplayer session id must encode exactly 32 bytes');
    }
    return value;
}

function canonicalEndpointId(value: unknown, label: string): string {
    if (typeof value !== 'string' || !ENDPOINT_ID_RE.test(value) || /^0+$/.test(value)) {
        fail('invalid_endpoint_id', `${label} must be a canonical non-zero iroh endpoint id`);
    }
    return value;
}

export function decodeMultiplayerIdentityRequest(value: unknown): MultiplayerIdentityRequest {
    const object = ownRecord(value);
    if (object === null) fail('invalid_request', 'Multiplayer identity request must be an object');
    if (object.protocol !== MULTIPLAYER_IDENTITY_PROTOCOL) {
        fail('invalid_protocol', 'Multiplayer identity request protocol is unsupported');
    }
    if (typeof object.requestId !== 'string' || !REQUEST_ID_RE.test(object.requestId)) {
        fail('invalid_request_id', 'Multiplayer identity request id must be 16-byte lowercase hex');
    }
    const operation = object.operation;
    if (
        operation !== 'status'
        && operation !== 'was_redeemed'
        && operation !== 'mark_redeemed'
        && operation !== 'sign_seat_proof'
    ) {
        fail('invalid_operation', 'Multiplayer identity operation is unsupported');
    }
    if (operation === 'status') {
        if (!exactKeys(object, ['protocol', 'requestId', 'operation'])) {
            fail('invalid_request', 'status request has invalid fields');
        }
    } else if (operation === 'was_redeemed' || operation === 'mark_redeemed') {
        if (!exactKeys(object, ['protocol', 'requestId', 'operation', 'sessionId'])) {
            fail('invalid_request', `${operation} request has invalid fields`);
        }
        canonicalSessionId(object.sessionId);
    } else {
        if (!exactKeys(object, [
            'protocol',
            'requestId',
            'operation',
            'sessionId',
            'hostEndpointId',
            'transportEndpointId',
        ])) {
            fail('invalid_request', 'sign_seat_proof request has invalid fields');
        }
        canonicalSessionId(object.sessionId);
        canonicalEndpointId(object.hostEndpointId, 'Host endpoint id');
        canonicalEndpointId(object.transportEndpointId, 'Transport endpoint id');
    }
    return object as MultiplayerIdentityRequest;
}

export async function dispatchMultiplayerIdentityRequest(
    value: unknown,
    signer: MultiplayerIdentitySigner,
): Promise<MultiplayerIdentityResponse> {
    let requestId = '00000000000000000000000000000000';
    try {
        const request = decodeMultiplayerIdentityRequest(value);
        requestId = request.requestId;
        let result: unknown;
        switch (request.operation) {
            case 'status':
                result = await signer.status();
                break;
            case 'was_redeemed':
                result = { redeemed: await signer.wasRedeemed(request.sessionId as string) };
                break;
            case 'mark_redeemed':
                await signer.markRedeemed(request.sessionId as string);
                result = { redeemed: true };
                break;
            case 'sign_seat_proof':
                result = {
                    signature: await signer.signSeatProof(
                        request.sessionId as string,
                        request.hostEndpointId as string,
                        request.transportEndpointId as string,
                    ),
                };
                break;
        }
        return { protocol: MULTIPLAYER_IDENTITY_PROTOCOL, requestId, ok: true, result };
    } catch (error) {
        return {
            protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
            requestId,
            ok: false,
            error: {
                code: error instanceof ProtocolError ? error.code : 'signer_error',
                message: error instanceof Error ? error.message : String(error),
            },
        };
    }
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        request.addEventListener('success', () => resolve(request.result), { once: true });
        request.addEventListener('error', () => reject(
            request.error ?? new Error('IndexedDB request failed'),
        ), { once: true });
    });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
    return new Promise<void>((resolve, reject) => {
        transaction.addEventListener('complete', () => resolve(), { once: true });
        transaction.addEventListener('abort', () => reject(
            transaction.error ?? new Error('IndexedDB transaction aborted'),
        ), { once: true });
        transaction.addEventListener('error', () => reject(
            transaction.error ?? new Error('IndexedDB transaction failed'),
        ), { once: true });
    });
}

async function openDatabase(): Promise<IDBDatabase> {
    if (globalThis.indexedDB === undefined) {
        throw new Error('browser multiplayer requires durable IndexedDB storage');
    }
    const request = indexedDB.open(DATABASE_NAME, DATABASE_VERSION);
    request.addEventListener('upgradeneeded', () => {
        const database = request.result;
        if (!database.objectStoreNames.contains(IDENTITY_STORE)) {
            database.createObjectStore(IDENTITY_STORE, { keyPath: 'name' });
        }
        if (!database.objectStoreNames.contains(REDEMPTION_STORE)) {
            database.createObjectStore(REDEMPTION_STORE);
        }
    }, { once: true });
    const database = await requestResult(request);
    database.addEventListener('versionchange', () => database.close());
    return database;
}

function validateIdentity(value: unknown): asserts value is StoredIdentity {
    const identity = ownRecord(value) as Partial<StoredIdentity> | null;
    if (
        identity === null
        || identity.name !== IDENTITY_KEY
        || !(identity.publicKey instanceof CryptoKey)
        || !(identity.privateKey instanceof CryptoKey)
        || identity.publicKey.type !== 'public'
        || identity.privateKey.type !== 'private'
        || !identity.publicKey.extractable
        || identity.privateKey.extractable
        || identity.publicKey.algorithm.name !== 'Ed25519'
        || identity.privateKey.algorithm.name !== 'Ed25519'
        || identity.publicKey.usages.length !== 1
        || identity.publicKey.usages[0] !== 'verify'
        || identity.privateKey.usages.length !== 1
        || identity.privateKey.usages[0] !== 'sign'
    ) {
        throw new Error('stored browser multiplayer identity failed its CryptoKey invariants');
    }
}

async function readIdentity(database: IDBDatabase): Promise<StoredIdentity | undefined> {
    const transaction = database.transaction(IDENTITY_STORE, 'readonly');
    const value = await requestResult(transaction.objectStore(IDENTITY_STORE).get(IDENTITY_KEY));
    await transactionDone(transaction);
    if (value === undefined) return undefined;
    validateIdentity(value);
    return value;
}

async function generateIdentity(): Promise<StoredIdentity> {
    let pair: CryptoKeyPair;
    try {
        pair = await crypto.subtle.generateKey('Ed25519', false, ['sign', 'verify']);
    } catch (error) {
        throw new Error(`browser multiplayer requires WebCrypto Ed25519 (${String(error)})`);
    }
    const identity: StoredIdentity = {
        name: IDENTITY_KEY,
        publicKey: pair.publicKey,
        privateKey: pair.privateKey,
    };
    validateIdentity(identity);
    return identity;
}

async function loadIdentity(database: IDBDatabase): Promise<StoredIdentity> {
    const stored = await readIdentity(database);
    if (stored !== undefined) return stored;
    const candidate = await generateIdentity();
    const transaction = database.transaction(IDENTITY_STORE, 'readwrite');
    const request = transaction.objectStore(IDENTITY_STORE).add(candidate);
    try {
        await requestResult(request);
        await transactionDone(transaction);
        return candidate;
    } catch (error) {
        if (request.error?.name !== 'ConstraintError' && transaction.error?.name !== 'ConstraintError') {
            throw error;
        }
        const winner = await readIdentity(database);
        if (winner === undefined) {
            throw new Error('browser multiplayer identity creation raced without a durable winner');
        }
        return winner;
    }
}

function decodeBase64Url(value: string): Uint8Array<ArrayBuffer> {
    const padded = value.replaceAll('-', '+').replaceAll('_', '/')
        + '='.repeat((4 - value.length % 4) % 4);
    let binary: string;
    try {
        binary = atob(padded);
    } catch {
        fail('invalid_base64url', 'Value is not valid base64url');
    }
    return Uint8Array.from(binary, character => character.charCodeAt(0));
}

function encodeBase64Url(bytes: Uint8Array): string {
    let binary = '';
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function decodeEndpointId(value: string): Uint8Array<ArrayBuffer> {
    const bytes = new Uint8Array(32);
    for (let index = 0; index < bytes.byteLength; index += 1) {
        bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
    }
    return bytes;
}

export function browserSeatProofMessage(
    sessionId: string,
    hostEndpointId: string,
    transportEndpointId: string,
): Uint8Array<ArrayBuffer> {
    const session = decodeBase64Url(canonicalSessionId(sessionId));
    const host = decodeEndpointId(canonicalEndpointId(hostEndpointId, 'Host endpoint id'));
    const transport = decodeEndpointId(canonicalEndpointId(
        transportEndpointId,
        'Transport endpoint id',
    ));
    const message = new Uint8Array(SEAT_PROOF_DOMAIN.byteLength + 96);
    message.set(SEAT_PROOF_DOMAIN);
    message.set(session, SEAT_PROOF_DOMAIN.byteLength);
    message.set(host, SEAT_PROOF_DOMAIN.byteLength + 32);
    message.set(transport, SEAT_PROOF_DOMAIN.byteLength + 64);
    return message;
}

export class IndexedDbMultiplayerIdentitySigner implements MultiplayerIdentitySigner {
    async status(): Promise<{ readonly publicKey: string; readonly persistent: boolean | null }> {
        const database = await openDatabase();
        try {
            const identity = await loadIdentity(database);
            const publicKey = new Uint8Array(await crypto.subtle.exportKey('raw', identity.publicKey));
            if (publicKey.byteLength !== 32 || publicKey.every(byte => byte === 0)) {
                throw new Error('stored browser multiplayer public key is invalid');
            }
            const persistent = navigator.storage?.persist === undefined
                ? null
                : await navigator.storage.persist();
            return { publicKey: encodeBase64Url(publicKey), persistent };
        } finally {
            database.close();
        }
    }

    async wasRedeemed(sessionId: string): Promise<boolean> {
        canonicalSessionId(sessionId);
        const database = await openDatabase();
        try {
            const transaction = database.transaction(REDEMPTION_STORE, 'readonly');
            const value = await requestResult(transaction.objectStore(REDEMPTION_STORE).get(sessionId));
            await transactionDone(transaction);
            if (value !== undefined && value !== true) {
                throw new Error('stored browser multiplayer redemption is malformed');
            }
            return value === true;
        } finally {
            database.close();
        }
    }

    async markRedeemed(sessionId: string): Promise<void> {
        canonicalSessionId(sessionId);
        const database = await openDatabase();
        try {
            const transaction = database.transaction(REDEMPTION_STORE, 'readwrite');
            transaction.objectStore(REDEMPTION_STORE).put(true, sessionId);
            await transactionDone(transaction);
        } finally {
            database.close();
        }
    }

    async signSeatProof(
        sessionId: string,
        hostEndpointId: string,
        transportEndpointId: string,
    ): Promise<string> {
        const message = browserSeatProofMessage(sessionId, hostEndpointId, transportEndpointId);
        const database = await openDatabase();
        try {
            const identity = await loadIdentity(database);
            const signature = new Uint8Array(await crypto.subtle.sign(
                'Ed25519',
                identity.privateKey,
                message,
            ));
            if (signature.byteLength !== 64 || signature.every(byte => byte === 0)) {
                throw new Error('WebCrypto returned an invalid multiplayer seat signature');
            }
            return encodeBase64Url(signature);
        } finally {
            database.close();
        }
    }
}

function configuredGameOrigin(): string {
    const environment = (import.meta as ImportMeta & {
        readonly env?: Record<string, string | boolean | undefined>;
    }).env;
    const configured = environment?.VITE_GAME_ORIGIN ?? MULTIPLAYER_GAME_ORIGIN;
    if (typeof configured !== 'string') throw new Error('configured game origin is invalid');
    const url = new URL(configured);
    if (
        url.origin !== configured
        || url.protocol !== 'https:'
        || url.username.length !== 0
        || url.password.length !== 0
    ) {
        throw new Error('multiplayer signer game origin must be one canonical HTTPS origin');
    }
    return configured;
}

export function installMultiplayerIdentitySigner(
    signer: MultiplayerIdentitySigner = new IndexedDbMultiplayerIdentitySigner(),
): void {
    if (window.top === window.self || window.parent === window) {
        throw new Error('Multiplayer identity signer refuses to run as a top-level document');
    }
    const parentOrigin = configuredGameOrigin();
    const completed = new Map<string, {
        readonly requestJson: string;
        readonly response: MultiplayerIdentityResponse;
    }>();
    window.addEventListener('message', event => {
        if (event.source !== window.parent || event.origin !== parentOrigin) return;
        void (async (): Promise<void> => {
            let requestJson: string;
            try {
                requestJson = JSON.stringify(event.data);
            } catch {
                return;
            }
            const candidate = ownRecord(event.data);
            const requestId = typeof candidate?.requestId === 'string' ? candidate.requestId : '';
            const prior = completed.get(requestId);
            const response = prior === undefined
                ? await dispatchMultiplayerIdentityRequest(event.data, signer)
                : prior.requestJson === requestJson
                    ? prior.response
                    : {
                        protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
                        requestId,
                        ok: false as const,
                        error: {
                            code: 'request_id_reuse',
                            message: 'Multiplayer identity request id was reused with different fields',
                        },
                    };
            if (prior === undefined && REQUEST_ID_RE.test(response.requestId)) {
                completed.set(response.requestId, { requestJson, response });
                if (completed.size > 256) {
                    const oldest = completed.keys().next().value as string | undefined;
                    if (oldest !== undefined) completed.delete(oldest);
                }
            }
            window.parent.postMessage(response, parentOrigin);
        })();
    });
    window.parent.postMessage({
        protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
        kind: 'ready',
    }, parentOrigin);
}
