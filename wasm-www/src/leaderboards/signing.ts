const SIGNER_PROTOCOL = 'robinhood.browser-identity.v1';
const SIGNER_PATH = '/identity-signer/';
const REQUEST_TIMEOUT_MS = 30_000;
const PUBLIC_KEY_RE = /^[0-9a-f]{64}$/u;
const REQUEST_ID_RE = /^[0-9a-f]{32}$/u;
const ERROR_CODE_RE = /^[a-z0-9_]{1,100}$/u;

type SignerOperation =
    | 'status'
    | 'public_key'
    | 'sign_username_update'
    | 'sign_deletion_request';

type SignerRequest = {
    readonly protocol: typeof SIGNER_PROTOCOL;
    readonly requestId: string;
    readonly operation: SignerOperation;
    readonly payloadJson?: string;
};

type StatusResult = {
    readonly kind: 'status';
    readonly publicKey: string;
};

type PublicKeyResult = {
    readonly kind: 'public_key';
    readonly publicKey: string;
};

type SignedDocumentResult = {
    readonly kind: 'signed_document';
    readonly documentJson: string;
};

type PendingRequest = {
    readonly resolve: (value: unknown) => void;
    readonly reject: (reason: unknown) => void;
    readonly timer: number;
};

export type LeaderboardSigningBridge = {
    readonly publicKey: () => Promise<string>;
    readonly signUsernameUpdate: (unsignedEnvelopeJson: string) => Promise<string>;
    readonly signDeletionRequest: (unsignedEnvelopeJson: string) => Promise<string>;
};

export type LeaderboardSigningConnection = {
    readonly kind: 'ready';
    readonly bridge: LeaderboardSigningBridge;
};

export class IdentitySignerError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = 'IdentitySignerError';
        this.code = code;
    }
}

let connectionPromise: Promise<LeaderboardSigningConnection | null> | null = null;

/** Connect to the dedicated signer origin. Private key material never crosses this boundary. */
export async function connectedSigningBridge(): Promise<LeaderboardSigningConnection | null> {
    if (window.top !== window.self) return null;
    connectionPromise ??= loadConnection();
    return await connectionPromise;
}

async function loadConnection(): Promise<LeaderboardSigningConnection | null> {
    try {
        const transport = await SignerTransport.connect(configuredSignerOrigin());
        const status = parseStatusResult(await transport.request('status'));
        const bridge = bridgeFor(transport);
        if (await bridge.publicKey() !== status.publicKey) {
            throw new Error('The identity signer changed keys during connection setup.');
        }
        return { kind: 'ready', bridge };
    } catch (error) {
        console.info('leaderboard identity signer is unavailable:', error);
        return null;
    }
}

function bridgeFor(transport: SignerTransport): LeaderboardSigningBridge {
    return {
        publicKey: async () => {
            const result = parsePublicKeyResult(await transport.request('public_key'));
            return result.publicKey;
        },
        signUsernameUpdate: async payloadJson => {
            const result = parseSignedDocumentResult(await transport.request(
                'sign_username_update',
                payloadJson,
            ));
            return result.documentJson;
        },
        signDeletionRequest: async payloadJson => {
            const result = parseSignedDocumentResult(await transport.request(
                'sign_deletion_request',
                payloadJson,
            ));
            return result.documentJson;
        },
    };
}

export class SignerTransport {
    readonly #origin: string;
    readonly #frame: HTMLIFrameElement;
    readonly #pending = new Map<string, PendingRequest>();
    readonly #ready: Promise<void>;
    #resolveReady: (() => void) | null = null;
    #rejectReady: ((reason: unknown) => void) | null = null;
    #readyTimer: number;

    private constructor(origin: string, frame: HTMLIFrameElement) {
        this.#origin = origin;
        this.#frame = frame;
        this.#ready = new Promise<void>((resolve, reject) => {
            this.#resolveReady = resolve;
            this.#rejectReady = reject;
        });
        this.#readyTimer = window.setTimeout(() => {
            this.#rejectReady?.(new IdentitySignerError(
                'signer_ready_timeout',
                'The identity signer did not become ready in time.',
            ));
            this.#resolveReady = null;
            this.#rejectReady = null;
        }, REQUEST_TIMEOUT_MS);
        window.addEventListener('message', this.#receive);
        window.addEventListener('pagehide', this.#close, { once: true });
    }

    static async connect(origin: string): Promise<SignerTransport> {
        const frame = document.createElement('iframe');
        frame.hidden = true;
        frame.title = 'Robin Hood identity signer';
        frame.referrerPolicy = 'no-referrer';
        // The signer keeps its real cross-origin storage identity but receives
        // no forms, popups, modals, top navigation, or download capability.
        frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
        frame.src = new URL(SIGNER_PATH, `${origin}/`).toString();
        const transport = new SignerTransport(origin, frame);
        frame.addEventListener('error', () => transport.#failReady(new IdentitySignerError(
            'signer_load_failed',
            'The identity signer could not be loaded.',
        )), { once: true });
        document.body.append(frame);
        try {
            await transport.#ready;
            return transport;
        } catch (error) {
            transport.#close();
            throw error;
        }
    }

    async request(operation: SignerOperation, payloadJson?: string): Promise<unknown> {
        const signing = operation.startsWith('sign_');
        if (signing !== (payloadJson !== undefined)) {
            throw new Error(`Identity signer operation ${operation} has an invalid payload shape.`);
        }
        if (payloadJson !== undefined && new TextEncoder().encode(payloadJson).byteLength > 128 * 1024) {
            throw new Error('Identity signer payload exceeds 128 KiB.');
        }
        const target = this.#frame.contentWindow;
        if (target === null || !this.#frame.isConnected) {
            throw new IdentitySignerError('signer_disconnected', 'The identity signer is no longer connected.');
        }
        const requestId = randomRequestId();
        const request: SignerRequest = {
            protocol: SIGNER_PROTOCOL,
            requestId,
            operation,
            ...(payloadJson === undefined ? {} : { payloadJson }),
        };
        return await new Promise<unknown>((resolve, reject) => {
            const timer = window.setTimeout(() => {
                this.#pending.delete(requestId);
                reject(new IdentitySignerError('signer_request_timeout', 'The identity signer did not respond in time.'));
            }, REQUEST_TIMEOUT_MS);
            this.#pending.set(requestId, { resolve, reject, timer });
            try {
                target.postMessage(request, this.#origin);
            } catch (error) {
                window.clearTimeout(timer);
                this.#pending.delete(requestId);
                reject(error);
            }
        });
    }

    #receive = (event: MessageEvent<unknown>): void => {
        // Both checks are mandatory: another frame from the right origin, or
        // this frame navigated to the wrong origin, must never be trusted.
        if (event.origin !== this.#origin || event.source !== this.#frame.contentWindow) return;
        if (isReadyEnvelope(event.data)) {
            window.clearTimeout(this.#readyTimer);
            this.#resolveReady?.();
            this.#resolveReady = null;
            this.#rejectReady = null;
            return;
        }
        let envelope: ReturnType<typeof parseResponseEnvelope>;
        try {
            envelope = parseResponseEnvelope(event.data);
        } catch {
            return;
        }
        const pending = this.#pending.get(envelope.requestId);
        if (pending === undefined) return;
        window.clearTimeout(pending.timer);
        this.#pending.delete(envelope.requestId);
        if (envelope.ok) pending.resolve(envelope.result);
        else pending.reject(new IdentitySignerError(envelope.error.code, envelope.error.message));
    };

    #failReady(reason: unknown): void {
        window.clearTimeout(this.#readyTimer);
        this.#rejectReady?.(reason);
        this.#resolveReady = null;
        this.#rejectReady = null;
    }

    #close = (): void => {
        this.#failReady(new IdentitySignerError('signer_disconnected', 'The identity signer disconnected.'));
        window.removeEventListener('message', this.#receive);
        for (const pending of this.#pending.values()) {
            window.clearTimeout(pending.timer);
            pending.reject(new IdentitySignerError('signer_disconnected', 'The identity signer disconnected.'));
        }
        this.#pending.clear();
        this.#frame.remove();
    };
}

function configuredSignerOrigin(): string {
    const raw = import.meta.env.VITE_ROBIN_IDENTITY_SIGNER_ORIGIN;
    if (typeof raw !== 'string' || raw.length === 0 || raw.trim() !== raw) {
        throw new Error('No identity signer origin is compiled into this deployment.');
    }
    const url = new URL(raw);
    const loopback = url.protocol === 'http:'
        && (url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]');
    if (url.protocol !== 'https:' && !loopback) {
        throw new Error('The identity signer origin must use HTTPS outside loopback development.');
    }
    if (url.username !== '' || url.password !== '' || url.search !== '' || url.hash !== '' || raw !== url.origin) {
        throw new Error('The identity signer must be configured as one exact origin without credentials or a path.');
    }
    if (!loopback && url.origin === window.location.origin) {
        throw new Error('The identity signer must use a dedicated origin.');
    }
    return url.origin;
}

function randomRequestId(): string {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

function isReadyEnvelope(value: unknown): boolean {
    const obj = strictObjectOrNull(value);
    return obj !== null
        && exactKeysMatch(obj, ['protocol', 'kind'])
        && obj.protocol === SIGNER_PROTOCOL
        && obj.kind === 'ready';
}

function parseResponseEnvelope(value: unknown):
    | { readonly requestId: string; readonly ok: true; readonly result: unknown }
    | { readonly requestId: string; readonly ok: false; readonly error: { readonly code: string; readonly message: string } } {
    const obj = strictObject(value, 'identity signer response');
    if (obj.protocol !== SIGNER_PROTOCOL) throw new Error('Identity signer protocol mismatch.');
    const requestId = boundedString(obj.requestId, 'identity signer requestId', 32);
    if (!REQUEST_ID_RE.test(requestId)) throw new Error('Identity signer requestId is invalid.');
    if (obj.ok === true) {
        exactKeys(obj, ['protocol', 'requestId', 'ok', 'result'], 'identity signer success response');
        return { requestId, ok: true, result: obj.result };
    }
    if (obj.ok !== false) throw new Error('Identity signer response ok flag is invalid.');
    exactKeys(obj, ['protocol', 'requestId', 'ok', 'error'], 'identity signer error response');
    const error = strictObject(obj.error, 'identity signer error');
    exactKeys(error, ['code', 'message'], 'identity signer error');
    const code = boundedString(error.code, 'identity signer error code', 100);
    const message = boundedString(error.message, 'identity signer error message', 500);
    if (!ERROR_CODE_RE.test(code)) throw new Error('Identity signer error code is invalid.');
    return { requestId, ok: false, error: { code, message } };
}

function parseStatusResult(value: unknown): StatusResult {
    const obj = strictObject(value, 'identity signer status');
    exactKeys(obj, ['kind', 'publicKey'], 'identity signer status');
    if (obj.kind !== 'status') throw new Error('Identity signer status result is invalid.');
    return { kind: 'status', publicKey: validatePublicKey(obj.publicKey) };
}

function parsePublicKeyResult(value: unknown): PublicKeyResult {
    const obj = strictObject(value, 'identity signer public-key result');
    exactKeys(obj, ['kind', 'publicKey'], 'identity signer public-key result');
    if (obj.kind !== 'public_key') throw new Error('Identity signer public-key result is invalid.');
    return { kind: 'public_key', publicKey: validatePublicKey(obj.publicKey) };
}

function parseSignedDocumentResult(value: unknown): SignedDocumentResult {
    const obj = strictObject(value, 'identity signer signed-document result');
    exactKeys(obj, ['kind', 'documentJson'], 'identity signer signed-document result');
    if (obj.kind !== 'signed_document') throw new Error('Identity signer signed-document result is invalid.');
    return {
        kind: 'signed_document',
        documentJson: boundedString(obj.documentJson, 'identity signer documentJson', 128 * 1024),
    };
}

function validatePublicKey(value: unknown): string {
    const key = boundedString(value, 'identity signer public key', 64);
    if (!PUBLIC_KEY_RE.test(key) || /^0{64}$/u.test(key)) {
        throw new Error('The identity signer returned an invalid public key.');
    }
    return key;
}

function strictObjectOrNull(value: unknown): Record<string, unknown> | null {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
    const prototype = Object.getPrototypeOf(value) as unknown;
    if (prototype !== Object.prototype && prototype !== null) return null;
    return value as Record<string, unknown>;
}

function strictObject(value: unknown, label: string): Record<string, unknown> {
    const object = strictObjectOrNull(value);
    if (object === null) throw new Error(`${label} must be an object.`);
    return object;
}

function exactKeysMatch(value: Record<string, unknown>, expected: readonly string[]): boolean {
    const actual = Object.keys(value).sort();
    const canonical = [...expected].sort();
    return actual.length === canonical.length
        && actual.every((key, index) => key === canonical[index]);
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[], label: string): void {
    if (!exactKeysMatch(value, expected)) throw new Error(`${label} has missing or unknown fields.`);
}

function boundedString(value: unknown, label: string, maximumLength: number): string {
    if (typeof value !== 'string' || value.length === 0 || value.length > maximumLength) {
        throw new Error(`${label} is invalid.`);
    }
    return value;
}

// Export parsers only to exercise the hostile-message boundary without a DOM.
export const signerProtocolTestHooks = {
    isReadyEnvelope,
    parseResponseEnvelope,
    parseStatusResult,
    parsePublicKeyResult,
    parseSignedDocumentResult,
};
