const DATABASE_NAME = 'robinhood-multiplayer-identity-v1';
const DATABASE_VERSION = 1;
const IDENTITY_STORE = 'identity';
const REDEMPTION_STORE = 'redemptions';
const IDENTITY_KEY = 'browser-seat-owner-v1';
const MAX_SIGNING_MESSAGE_BYTES = 16 * 1024;

type StoredIdentity = {
    readonly name: typeof IDENTITY_KEY;
    readonly publicKey: CryptoKey;
    readonly privateKey: CryptoKey;
};

export type BrowserMultiplayerIdentity = {
    readonly publicKey: Uint8Array;
    readonly sign: (message: Uint8Array) => Promise<Uint8Array>;
};

declare global {
    var robinMultiplayerIdentity: BrowserMultiplayerIdentity | undefined;
    var robinMarkMultiplayerInvitationRedeemed: ((sessionId: string) => Promise<void>) | undefined;
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        request.addEventListener('success', () => resolve(request.result), { once: true });
        request.addEventListener('error', () => reject(request.error ?? new Error('IndexedDB request failed')), {
            once: true,
        });
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
    });
    const database = await requestResult(request);
    database.addEventListener('versionchange', () => database.close());
    return database;
}

function validateStoredIdentity(value: unknown): asserts value is StoredIdentity {
    if (value === null || typeof value !== 'object') {
        throw new Error('stored browser multiplayer identity is malformed');
    }
    const identity = value as Partial<StoredIdentity>;
    if (
        identity.name !== IDENTITY_KEY
        || !(identity.publicKey instanceof CryptoKey)
        || !(identity.privateKey instanceof CryptoKey)
        || identity.publicKey.type !== 'public'
        || identity.privateKey.type !== 'private'
        || identity.privateKey.extractable
        || identity.publicKey.algorithm.name !== 'Ed25519'
        || identity.privateKey.algorithm.name !== 'Ed25519'
        || !identity.publicKey.usages.includes('verify')
        || !identity.privateKey.usages.includes('sign')
    ) {
        throw new Error('stored browser multiplayer identity failed its CryptoKey invariants');
    }
}

async function readIdentity(database: IDBDatabase): Promise<StoredIdentity | undefined> {
    const transaction = database.transaction(IDENTITY_STORE, 'readonly');
    const value = await requestResult(transaction.objectStore(IDENTITY_STORE).get(IDENTITY_KEY));
    await transactionDone(transaction);
    if (value === undefined) return undefined;
    validateStoredIdentity(value);
    return value;
}

async function generateIdentity(): Promise<StoredIdentity> {
    let keyPair: CryptoKeyPair;
    try {
        keyPair = await crypto.subtle.generateKey('Ed25519', false, ['sign', 'verify']);
    } catch (error) {
        throw new Error(`browser multiplayer requires WebCrypto Ed25519 (${String(error)})`);
    }
    const identity: StoredIdentity = {
        name: IDENTITY_KEY,
        publicKey: keyPair.publicKey,
        privateKey: keyPair.privateKey,
    };
    validateStoredIdentity(identity);
    return identity;
}

async function createIdentityFirstWriterWins(database: IDBDatabase): Promise<StoredIdentity> {
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
            throw new Error('browser multiplayer identity creation raced but no durable winner exists');
        }
        return winner;
    }
}

async function loadIdentity(database: IDBDatabase): Promise<StoredIdentity> {
    return await readIdentity(database) ?? await createIdentityFirstWriterWins(database);
}

function validateSessionId(sessionId: string): void {
    if (!/^[A-Za-z0-9_-]{43}$/.test(sessionId) || /^A+$/.test(sessionId)) {
        throw new Error('browser multiplayer session id is not canonical');
    }
}

export async function wasInvitationRedeemed(sessionId: string): Promise<boolean> {
    validateSessionId(sessionId);
    const database = await openDatabase();
    try {
        const transaction = database.transaction(REDEMPTION_STORE, 'readonly');
        const value = await requestResult(transaction.objectStore(REDEMPTION_STORE).get(sessionId));
        await transactionDone(transaction);
        return value === true;
    } finally {
        database.close();
    }
}

export async function markInvitationRedeemed(sessionId: string): Promise<void> {
    validateSessionId(sessionId);
    const database = await openDatabase();
    try {
        const transaction = database.transaction(REDEMPTION_STORE, 'readwrite');
        transaction.objectStore(REDEMPTION_STORE).put(true, sessionId);
        await transactionDone(transaction);
    } finally {
        database.close();
    }
}

export async function installBrowserMultiplayerIdentity(): Promise<BrowserMultiplayerIdentity> {
    const database = await openDatabase();
    try {
        const stored = await loadIdentity(database);
        const publicKey = new Uint8Array(await crypto.subtle.exportKey('raw', stored.publicKey));
        if (publicKey.byteLength !== 32) {
            throw new Error(`browser multiplayer Ed25519 public key is ${publicKey.byteLength} bytes`);
        }
        const identity: BrowserMultiplayerIdentity = {
            publicKey,
            sign: async (message): Promise<Uint8Array> => {
                if (!(message instanceof Uint8Array) || message.byteLength > MAX_SIGNING_MESSAGE_BYTES) {
                    throw new Error(
                        `browser multiplayer signing input exceeds ${MAX_SIGNING_MESSAGE_BYTES} bytes`,
                    );
                }
                // Rust/WASM may hand us a SharedArrayBuffer-backed view when
                // the threaded build is active. WebCrypto accepts only an
                // ordinary BufferSource, so sign an exact private copy.
                const signingBytes = Uint8Array.from(message);
                return new Uint8Array(await crypto.subtle.sign(
                    'Ed25519',
                    stored.privateKey,
                    signingBytes,
                ));
            },
        };
        globalThis.robinMultiplayerIdentity = identity;
        globalThis.robinMarkMultiplayerInvitationRedeemed = markInvitationRedeemed;
        if (navigator.storage?.persist !== undefined) {
            void navigator.storage.persist().then((persisted) => {
                if (!persisted) console.warn('browser multiplayer identity storage is not persistent');
            });
        }
        return identity;
    } finally {
        database.close();
    }
}
