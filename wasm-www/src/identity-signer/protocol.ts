export const LEADERBOARD_IDENTITY_PROTOCOL = 'robinhood.browser-identity.v1' as const;
export const LEADERBOARD_GAME_ORIGIN = 'https://robinhood.phiresky.xyz' as const;

const REQUEST_ID_RE = /^[0-9a-f]{32}$/u;
const PUBLIC_KEY_RE = /^[0-9a-f]{64}$/u;
const MAX_COMPLETED_REQUESTS = 256;

const SIGNING_LIMITS = {
    sign_username_update: 4 * 1024,
    sign_submission: 128 * 1024,
    sign_multiplayer_leaderboard_request: 4 * 1024,
    sign_named_seat_join: 8 * 1024,
    sign_replay_session_genesis: 64 * 1024,
    sign_competition_run_grant_request: 128 * 1024,
    sign_fresh_run_preflight_request: 128 * 1024,
    sign_campaign_continuation_preflight_as_host: 128 * 1024,
    sign_campaign_continuation_preflight_as_controller: 128 * 1024,
    sign_campaign_continuation: 128 * 1024,
    sign_submission_owner_status: 8 * 1024,
    sign_deletion_request: 8 * 1024,
} as const;

export type LeaderboardIdentityOperation =
    | 'status'
    | 'public_key'
    | keyof typeof SIGNING_LIMITS;

export type LeaderboardIdentityRequest = {
    readonly protocol: typeof LEADERBOARD_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly operation: LeaderboardIdentityOperation;
    readonly payloadJson?: string;
};

export type LeaderboardIdentityResponse = {
    readonly protocol: typeof LEADERBOARD_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly ok: true;
    readonly result: unknown;
} | {
    readonly protocol: typeof LEADERBOARD_IDENTITY_PROTOCOL;
    readonly requestId: string;
    readonly ok: false;
    readonly error: { readonly code: string; readonly message: string };
};

/** The exact no-start wasm-bindgen surface exported by leaderboard_identity_bridge. */
export type LeaderboardIdentityBridgeModule = {
    readonly robinhoodAuthorizeLeaderboardIdentityParent: (parentOrigin: string) => void;
    readonly robinhoodLeaderboardIdentityStatus: (parentOrigin: string) => Promise<string>;
    readonly robinhoodLeaderboardPublicKey: (parentOrigin: string) => Promise<string>;
    readonly robinhoodSignUsernameUpdate: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignSubmissionClaim: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignMultiplayerLeaderboardRequest: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignNamedSeatJoin: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignReplaySessionGenesis: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignCompetitionRunGrantRequest: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignFreshRunPreflightRequest: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignCampaignContinuationPreflightAsHost: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignCampaignContinuationPreflightAsController: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignCampaignContinuation: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignSubmissionOwnerStatus: (parentOrigin: string, json: string) => Promise<string>;
    readonly robinhoodSignDeletionRequest: (parentOrigin: string, json: string) => Promise<string>;
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

function payloadBytes(value: unknown, maximum: number, operation: string): string {
    if (typeof value !== 'string' || value.length === 0) {
        fail('invalid_payload', `${operation} payload must be non-empty JSON`);
    }
    if (new TextEncoder().encode(value).byteLength > maximum) {
        fail('payload_too_large', `${operation} payload exceeds ${maximum} bytes`);
    }
    try {
        const parsed = JSON.parse(value) as unknown;
        if (ownRecord(parsed) === null) fail('invalid_payload', `${operation} payload must be a JSON object`);
    } catch (error) {
        if (error instanceof ProtocolError) throw error;
        fail('invalid_payload', `${operation} payload is not valid JSON`);
    }
    return value;
}

export function decodeLeaderboardIdentityRequest(value: unknown): LeaderboardIdentityRequest {
    const object = ownRecord(value);
    if (object === null) fail('invalid_request', 'Leaderboard identity request must be an object');
    if (object.protocol !== LEADERBOARD_IDENTITY_PROTOCOL) {
        fail('invalid_protocol', 'Leaderboard identity request protocol is unsupported');
    }
    if (typeof object.requestId !== 'string' || !REQUEST_ID_RE.test(object.requestId)) {
        fail('invalid_request_id', 'Leaderboard identity request id must be 16-byte lowercase hex');
    }
    const operation = object.operation;
    if (operation === 'status' || operation === 'public_key') {
        if (!exactKeys(object, ['protocol', 'requestId', 'operation'])) {
            fail('invalid_request', `${operation} request has invalid fields`);
        }
        return object as LeaderboardIdentityRequest;
    }
    if (typeof operation !== 'string' || !(operation in SIGNING_LIMITS)) {
        fail('invalid_operation', 'Leaderboard identity operation is unsupported');
    }
    if (!exactKeys(object, ['protocol', 'requestId', 'operation', 'payloadJson'])) {
        fail('invalid_request', `${operation} request has invalid fields`);
    }
    payloadBytes(object.payloadJson, SIGNING_LIMITS[operation as keyof typeof SIGNING_LIMITS], operation);
    return object as LeaderboardIdentityRequest;
}

function publicKey(value: unknown): string {
    if (typeof value !== 'string' || !PUBLIC_KEY_RE.test(value) || /^0+$/u.test(value)) {
        fail('invalid_bridge_result', 'Identity bridge returned an invalid public key');
    }
    return value;
}

function bridgeJson(value: string, maximum: number, label: string): Record<string, unknown> {
    if (new TextEncoder().encode(value).byteLength > maximum) {
        fail('invalid_bridge_result', `${label} exceeds its result limit`);
    }
    try {
        const parsed = ownRecord(JSON.parse(value) as unknown);
        if (parsed === null) fail('invalid_bridge_result', `${label} must be a JSON object`);
        return parsed;
    } catch (error) {
        if (error instanceof ProtocolError) throw error;
        fail('invalid_bridge_result', `${label} is not valid JSON`);
    }
}

function statusResult(json: string): { readonly kind: 'status'; readonly publicKey: string } {
    const value = bridgeJson(json, 256, 'identity status');
    if (!exactKeys(value, ['kind', 'publicKey']) || value.kind !== 'status') {
        fail('invalid_bridge_result', 'Identity status has invalid fields');
    }
    return { kind: 'status', publicKey: publicKey(value.publicKey) };
}

function signedDocument(json: string, maximum: number): {
    readonly kind: 'signed_document';
    readonly documentJson: string;
} {
    bridgeJson(json, maximum, 'signed identity document');
    return { kind: 'signed_document', documentJson: json };
}

async function execute(
    request: LeaderboardIdentityRequest,
    bridge: LeaderboardIdentityBridgeModule,
    parentOrigin: string,
): Promise<unknown> {
    const payload = request.payloadJson as string;
    switch (request.operation) {
        case 'status':
            return statusResult(await bridge.robinhoodLeaderboardIdentityStatus(parentOrigin));
        case 'public_key':
            return { kind: 'public_key', publicKey: publicKey(
                await bridge.robinhoodLeaderboardPublicKey(parentOrigin),
            ) };
        case 'sign_username_update':
            return signedDocument(
                await bridge.robinhoodSignUsernameUpdate(parentOrigin, payload),
                SIGNING_LIMITS.sign_username_update,
            );
        case 'sign_submission': {
            const json = await bridge.robinhoodSignSubmissionClaim(parentOrigin, payload);
            bridgeJson(json, SIGNING_LIMITS.sign_submission, 'participant signature');
            return { kind: 'participant_signature', participantSignatureJson: json };
        }
        case 'sign_multiplayer_leaderboard_request': {
            const json = await bridge.robinhoodSignMultiplayerLeaderboardRequest(
                parentOrigin,
                payload,
            );
            bridgeJson(
                json,
                SIGNING_LIMITS.sign_multiplayer_leaderboard_request,
                'multiplayer participant signature',
            );
            return { kind: 'participant_signature', participantSignatureJson: json };
        }
        case 'sign_named_seat_join':
            return signedDocument(
                await bridge.robinhoodSignNamedSeatJoin(parentOrigin, payload),
                SIGNING_LIMITS.sign_named_seat_join,
            );
        case 'sign_replay_session_genesis':
            return signedDocument(
                await bridge.robinhoodSignReplaySessionGenesis(parentOrigin, payload),
                SIGNING_LIMITS.sign_replay_session_genesis,
            );
        case 'sign_competition_run_grant_request':
            return signedDocument(
                await bridge.robinhoodSignCompetitionRunGrantRequest(parentOrigin, payload),
                SIGNING_LIMITS.sign_competition_run_grant_request,
            );
        case 'sign_fresh_run_preflight_request':
            return signedDocument(
                await bridge.robinhoodSignFreshRunPreflightRequest(parentOrigin, payload),
                SIGNING_LIMITS.sign_fresh_run_preflight_request,
            );
        case 'sign_campaign_continuation_preflight_as_host': {
            const json = await bridge.robinhoodSignCampaignContinuationPreflightAsHost(
                parentOrigin,
                payload,
            );
            bridgeJson(
                json,
                SIGNING_LIMITS.sign_campaign_continuation_preflight_as_host,
                'campaign continuation preflight host signature',
            );
            return { kind: 'participant_signature', participantSignatureJson: json };
        }
        case 'sign_campaign_continuation_preflight_as_controller': {
            const json = await bridge.robinhoodSignCampaignContinuationPreflightAsController(
                parentOrigin,
                payload,
            );
            bridgeJson(
                json,
                SIGNING_LIMITS.sign_campaign_continuation_preflight_as_controller,
                'campaign continuation preflight controller signature',
            );
            return { kind: 'participant_signature', participantSignatureJson: json };
        }
        case 'sign_campaign_continuation':
            return signedDocument(
                await bridge.robinhoodSignCampaignContinuation(parentOrigin, payload),
                SIGNING_LIMITS.sign_campaign_continuation,
            );
        case 'sign_submission_owner_status':
            return signedDocument(
                await bridge.robinhoodSignSubmissionOwnerStatus(parentOrigin, payload),
                SIGNING_LIMITS.sign_submission_owner_status,
            );
        case 'sign_deletion_request':
            return signedDocument(
                await bridge.robinhoodSignDeletionRequest(parentOrigin, payload),
                SIGNING_LIMITS.sign_deletion_request,
            );
    }
}

export async function dispatchLeaderboardIdentityRequest(
    value: unknown,
    bridge: LeaderboardIdentityBridgeModule,
    parentOrigin: string = LEADERBOARD_GAME_ORIGIN,
): Promise<LeaderboardIdentityResponse> {
    let requestId = '00000000000000000000000000000000';
    try {
        const request = decodeLeaderboardIdentityRequest(value);
        requestId = request.requestId;
        return {
            protocol: LEADERBOARD_IDENTITY_PROTOCOL,
            requestId,
            ok: true,
            result: await execute(request, bridge, parentOrigin),
        };
    } catch (error) {
        return {
            protocol: LEADERBOARD_IDENTITY_PROTOCOL,
            requestId,
            ok: false,
            error: {
                code: error instanceof ProtocolError ? error.code : 'signer_error',
                message: error instanceof Error ? error.message : String(error),
            },
        };
    }
}

function configuredGameOrigin(): string {
    const environment = (import.meta as ImportMeta & {
        readonly env?: Record<string, string | boolean | undefined>;
    }).env;
    const configured = environment?.VITE_GAME_ORIGIN ?? LEADERBOARD_GAME_ORIGIN;
    if (typeof configured !== 'string') throw new Error('configured game origin is invalid');
    const url = new URL(configured);
    if (
        url.origin !== configured
        || url.protocol !== 'https:'
        || url.username.length !== 0
        || url.password.length !== 0
    ) {
        throw new Error('leaderboard signer game origin must be one canonical HTTPS origin');
    }
    return configured;
}

export function installLeaderboardIdentitySigner(bridge: LeaderboardIdentityBridgeModule): void {
    if (window.top === window.self || window.parent === window) {
        throw new Error('Leaderboard identity signer refuses to run as a top-level document');
    }
    const parentOrigin = configuredGameOrigin();
    bridge.robinhoodAuthorizeLeaderboardIdentityParent(parentOrigin);
    const completed = new Map<string, {
        readonly requestJson: string;
        readonly response: LeaderboardIdentityResponse;
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
                ? await dispatchLeaderboardIdentityRequest(event.data, bridge, parentOrigin)
                : prior.requestJson === requestJson
                    ? prior.response
                    : {
                        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
                        requestId,
                        ok: false as const,
                        error: {
                            code: 'request_id_reuse',
                            message: 'Leaderboard identity request id was reused with different fields',
                        },
                    };
            if (prior === undefined && REQUEST_ID_RE.test(response.requestId)) {
                completed.set(response.requestId, { requestJson, response });
                if (completed.size > MAX_COMPLETED_REQUESTS) {
                    const oldest = completed.keys().next().value as string | undefined;
                    if (oldest !== undefined) completed.delete(oldest);
                }
            }
            window.parent.postMessage(response, parentOrigin);
        })();
    });
    window.parent.postMessage({
        protocol: LEADERBOARD_IDENTITY_PROTOCOL,
        kind: 'ready',
    }, parentOrigin);
}

export const leaderboardIdentityProtocolTestHooks = Object.freeze({
    signingLimits: SIGNING_LIMITS,
    statusResult,
});
