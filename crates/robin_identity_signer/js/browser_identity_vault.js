/*
 * Durable, non-extractable browser identity shared with Feature 38.
 *
 * Keep the database, stores, record key, and CryptoKey invariants exactly in
 * sync with multiplayer_identity_protocol.ts. This module deliberately
 * exposes only operation-bound leaderboard signatures, never raw key material
 * or a generic signing primitive.
 */

const DATABASE_NAME = 'robinhood-multiplayer-identity-v1';
const DATABASE_VERSION = 1;
const IDENTITY_STORE = 'identity';
const REDEMPTION_STORE = 'redemptions';
const IDENTITY_KEY = 'browser-seat-owner-v1';

const textEncoder = new TextEncoder();

const SIGNING_DOMAINS = Object.freeze({
    username_update: textEncoder.encode('robinhood/leaderboards/1/username-update\0'),
    submission: textEncoder.encode('robinhood/leaderboards/1/submission\0'),
    competition_run_grant_request: textEncoder.encode('robinhood/leaderboards/1/competition-run-grant-request\0'),
    fresh_run_preflight_request: textEncoder.encode('robinhood/leaderboards/1/fresh-run-preflight-request\0'),
    campaign_continuation_preflight_host: textEncoder.encode('robinhood/leaderboards/1/campaign-continuation-preflight-host\0'),
    campaign_continuation_preflight_controller: textEncoder.encode('robinhood/leaderboards/1/campaign-continuation-preflight-controller\0'),
    campaign_continuation: textEncoder.encode('robinhood/leaderboards/1/campaign-continuation\0'),
    multiplayer_campaign_continuation: textEncoder.encode('robinhood/leaderboards/1/co-sign-payload\0'),
    multiplayer_submission: textEncoder.encode('robinhood/leaderboards/1/co-sign-payload\0'),
    named_seat_join: textEncoder.encode('robinhood/multiplayer/1/join-attestation\0'),
    replay_session_genesis: textEncoder.encode('robinhood/multiplayer/1/session-genesis\0'),
    submission_owner_status: textEncoder.encode('robinhood/leaderboards/1/submission-owner-status\0'),
    deletion_request: textEncoder.encode('robinhood/leaderboards/1/deletion-request\0'),
});

const SIGNING_LIMITS = Object.freeze({
    username_update: 4 * 1024,
    submission: 128 * 1024,
    competition_run_grant_request: 128 * 1024,
    fresh_run_preflight_request: 128 * 1024,
    campaign_continuation_preflight_host: 128 * 1024,
    campaign_continuation_preflight_controller: 128 * 1024,
    campaign_continuation: 128 * 1024,
    multiplayer_campaign_continuation: 139,
    multiplayer_submission: 139,
    named_seat_join: 8 * 1024,
    replay_session_genesis: 64 * 1024,
    submission_owner_status: 8 * 1024,
    deletion_request: 8 * 1024,
});

class BrowserIdentityError extends Error {
    constructor(code, message) {
        super(message);
        this.name = 'BrowserIdentityError';
        this.code = code;
    }
}

function fail(code, message) {
    throw new BrowserIdentityError(code, message);
}

function ownRecord(value) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) return null;
    return value;
}

function requestResult(request) {
    return new Promise((resolve, reject) => {
        request.addEventListener('success', () => resolve(request.result), { once: true });
        request.addEventListener('error', () => reject(
            request.error ?? new BrowserIdentityError('indexeddb_error', 'IndexedDB request failed'),
        ), { once: true });
    });
}

function transactionDone(transaction) {
    return new Promise((resolve, reject) => {
        transaction.addEventListener('complete', () => resolve(), { once: true });
        transaction.addEventListener('abort', () => reject(
            transaction.error ?? new BrowserIdentityError('indexeddb_error', 'IndexedDB transaction aborted'),
        ), { once: true });
        transaction.addEventListener('error', () => reject(
            transaction.error ?? new BrowserIdentityError('indexeddb_error', 'IndexedDB transaction failed'),
        ), { once: true });
    });
}

async function openDatabase(factory = globalThis.indexedDB) {
    if (factory === undefined || factory === null) {
        fail('unsupported', 'Durable IndexedDB storage is unavailable');
    }
    const request = factory.open(DATABASE_NAME, DATABASE_VERSION);
    request.addEventListener('upgradeneeded', () => {
        const database = request.result;
        if (!database.objectStoreNames.contains(IDENTITY_STORE)) {
            database.createObjectStore(IDENTITY_STORE, { keyPath: 'name' });
        }
        // Create Feature 38's companion store too when leaderboards open the
        // database first. Both modules therefore observe the same v1 schema.
        if (!database.objectStoreNames.contains(REDEMPTION_STORE)) {
            database.createObjectStore(REDEMPTION_STORE);
        }
    }, { once: true });
    const database = await requestResult(request);
    database.addEventListener('versionchange', () => database.close());
    return database;
}

function validateIdentity(value) {
    const identity = ownRecord(value);
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
        fail('corrupt_identity', 'Stored browser identity failed its CryptoKey invariants');
    }
    return identity;
}

async function readIdentity(database) {
    const transaction = database.transaction(IDENTITY_STORE, 'readonly');
    const value = await requestResult(transaction.objectStore(IDENTITY_STORE).get(IDENTITY_KEY));
    await transactionDone(transaction);
    return value === undefined ? undefined : validateIdentity(value);
}

async function generateIdentity(subtle = globalThis.crypto?.subtle) {
    if (subtle === undefined || subtle === null) {
        fail('unsupported', 'WebCrypto is unavailable');
    }
    let pair;
    try {
        pair = await subtle.generateKey('Ed25519', false, ['sign', 'verify']);
    } catch (error) {
        fail('unsupported', `WebCrypto Ed25519 is unavailable (${String(error)})`);
    }
    return validateIdentity({
        name: IDENTITY_KEY,
        publicKey: pair.publicKey,
        privateKey: pair.privateKey,
    });
}

async function loadIdentity(database) {
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
            fail('indexeddb_error', 'Identity creation raced without a durable winner');
        }
        return winner;
    }
}

let identityPromise = null;

async function identity() {
    identityPromise ??= (async () => loadIdentity(await openDatabase()))();
    try {
        return await identityPromise;
    } catch (error) {
        identityPromise = null;
        throw error;
    }
}

function bytesToLowerHex(bytes) {
    let result = '';
    for (const byte of bytes) result += byte.toString(16).padStart(2, '0');
    return result;
}

async function publicKeyHex(record = undefined, subtle = globalThis.crypto?.subtle) {
    if (subtle === undefined || subtle === null) fail('unsupported', 'WebCrypto is unavailable');
    const current = record ?? await identity();
    let raw;
    try {
        raw = new Uint8Array(await subtle.exportKey('raw', current.publicKey));
    } catch (error) {
        fail('corrupt_identity', `Export public key: ${String(error)}`);
    }
    if (raw.byteLength !== 32 || raw.every(byte => byte === 0)) {
        fail('corrupt_identity', 'Ed25519 public key must be 32 non-zero bytes');
    }
    return bytesToLowerHex(raw);
}

function exactBytes(value) {
    if (value instanceof Uint8Array) return value;
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (ArrayBuffer.isView(value)) {
        return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    }
    fail('invalid_message', 'Signer message must be a byte array');
}

function validateSigningMessage(operation, value) {
    const domain = SIGNING_DOMAINS[operation];
    const maximum = SIGNING_LIMITS[operation];
    if (domain === undefined || maximum === undefined) {
        fail('unsupported_operation', `Unsupported leaderboard identity operation: ${String(operation)}`);
    }
    const message = exactBytes(value);
    const multiplayerPurpose = operation === 'multiplayer_campaign_continuation'
        ? 1
        : operation === 'multiplayer_submission'
            ? 2
            : undefined;
    if (multiplayerPurpose !== undefined) {
        if (message.byteLength !== domain.byteLength + 1 + (3 * 32)
            || message[domain.byteLength] !== multiplayerPurpose) {
            fail('invalid_message', `Invalid ${operation} fixed co-signing payload`);
        }
        return message;
    }
    if (message.byteLength <= domain.byteLength || message.byteLength > maximum) {
        fail('invalid_message', `Invalid ${operation} signing message length`);
    }
    for (let index = 0; index < domain.byteLength; index += 1) {
        if (message[index] !== domain[index]) {
            fail('wrong_domain', `Signer message does not use the ${operation} domain`);
        }
    }
    if (message[domain.byteLength] !== 0x7b) {
        fail('invalid_message', 'Domain-separated payload must begin with a canonical JSON object');
    }
    return message;
}

export async function robinhoodLeaderboardIdentityStatus() {
    return JSON.stringify({ kind: 'status', publicKey: await publicKeyHex() });
}

export async function robinhoodLeaderboardIdentityPublicKey() {
    return await publicKeyHex();
}

export async function robinhoodLeaderboardIdentitySign(operation, message) {
    const exactMessage = validateSigningMessage(operation, message);
    const record = await identity();
    let signature;
    try {
        signature = new Uint8Array(await crypto.subtle.sign('Ed25519', record.privateKey, exactMessage));
    } catch (error) {
        fail('sign_failed', `WebCrypto Ed25519 signature failed (${String(error)})`);
    }
    if (signature.byteLength !== 64 || signature.every(byte => byte === 0)) {
        fail('sign_failed', 'WebCrypto returned an invalid Ed25519 signature');
    }
    return signature;
}

export const leaderboardIdentityVaultTestHooks = Object.freeze({
    databaseName: DATABASE_NAME,
    databaseVersion: DATABASE_VERSION,
    identityStore: IDENTITY_STORE,
    redemptionStore: REDEMPTION_STORE,
    identityKey: IDENTITY_KEY,
    signingDomains: SIGNING_DOMAINS,
    signingLimits: SIGNING_LIMITS,
    validateSigningMessage,
});
