import type { DigestVerified } from './canonical.js';
import { apiUrl } from './config.js';
import { DEFAULT_NETWORK_DEADLINE_MS, NetworkDeadline } from '../network_deadline.js';
import {
    parseBoardMetadata,
    parseBoardPage,
    parseCampaignSessionDetail,
    parsePlayerRunHistoryPage,
    parseRunDetail,
} from './public-response.js';
import {
    parseAndVerifyCampaignContentManifest,
} from './content-contract.js';
import {
    parseAndVerifyRulesConfigIdentity,
    parseAndVerifyPublishedRuleset,
    parseAndVerifyRulesetManifest,
} from './ruleset-contract.js';
import {
    parseAbuseReportAccepted,
    parseDeletionChallenge,
    parseDeletionReceipt,
    parsePlayerProfile,
    parseUsernameChallenge,
} from './account-contract.js';
import {
    type BoardMetadata,
    type BoardPage,
    type CampaignContentManifest,
    type CampaignSessionDetail,
    type RulesConfigIdentity,
    type RulesetManifest,
    type AbuseReportAccepted,
    type AbuseReportCategory,
    type DeletionChallenge,
    type DeletionRequestEnvelope,
    type DeletionReceipt,
    type DeletionTarget,
    type PlayerProfile,
    type PlayerRunHistoryPage,
    type RunDetail,
    type UsernameChallenge,
    type UsernameUpdateEnvelope,
} from './types.js';
import { filtersToApiQuery, PAGE_SIZE, type BoardFilters } from './state.js';

const MAX_JSON_BYTES = 2 * 1024 * 1024;
const MAX_REPLAY_BYTES = 32 * 1024 * 1024;

export class PublicApiError extends Error {
    readonly status: number;
    readonly code: string;

    constructor(status: number, code: string, message: string) {
        super(message);
        this.name = 'PublicApiError';
        this.status = status;
        this.code = code;
    }
}

export class HighscoreApi {
    readonly base: string;
    readonly #requestTimeoutMs: number;

    constructor(base: string, requestTimeoutMs: number = DEFAULT_NETWORK_DEADLINE_MS) {
        this.base = base;
        this.#requestTimeoutMs = requestTimeoutMs;
    }

    async metadata(signal?: AbortSignal): Promise<BoardMetadata> {
        return parseBoardMetadata(await this.getJson(['leaderboard-metadata'], signal));
    }

    async board(filters: BoardFilters, signal?: AbortSignal): Promise<BoardPage> {
        const page = parseBoardPage(await this.getJson(
            ['leaderboards'],
            signal,
            filtersToApiQuery(filters),
        ));
        if ((page.previousCursor?.opaqueToken ?? null) !== filters.cursor) {
            throw new PublicApiError(
                0,
                'leaderboard_cursor_mismatch',
                'The leaderboard response did not match the requested cursor.',
            );
        }
        return page;
    }

    async run(id: string, signal?: AbortSignal): Promise<RunDetail> {
        const document = await this.getJson(['runs', id], signal);
        const run = parseRunDetail(document);
        if (run.runId !== id) {
            throw new PublicApiError(0, 'run_identity_mismatch', 'The run response did not match its requested identity.');
        }
        return run;
    }

    async campaignSession(
        aggregateRunId: string,
        ordinal: number,
        signal?: AbortSignal,
    ): Promise<CampaignSessionDetail> {
        requireSessionOrdinal(ordinal);
        const document = await this.getJson(
            ['runs', aggregateRunId, 'sessions', String(ordinal)],
            signal,
        );
        const detail = parseCampaignSessionDetail(document);
        if (detail.aggregateRunId !== aggregateRunId || detail.ordinal !== ordinal) {
            throw new PublicApiError(
                0,
                'campaign_session_identity_mismatch',
                'The campaign session response did not match its requested aggregate and ordinal.',
            );
        }
        return detail;
    }

    async campaignContentManifest(
        sha256: string,
        signal?: AbortSignal,
    ): Promise<DigestVerified<CampaignContentManifest>> {
        requireSha256(sha256, 'Campaign content manifest');
        const document = await this.getJson(
            ['campaign-content-manifests', sha256],
            signal,
            {},
            'force-cache',
        );
        return await parseAndVerifyCampaignContentManifest(document, sha256);
    }

    async rulesConfig(sha256: string, signal?: AbortSignal): Promise<DigestVerified<RulesConfigIdentity>> {
        requireSha256(sha256, 'Rules configuration');
        const document = await this.getJson(['rules-configs', sha256], signal, {}, 'force-cache');
        return await parseAndVerifyRulesConfigIdentity(document, sha256);
    }

    async rulesetManifest(sha256: string, signal?: AbortSignal): Promise<DigestVerified<RulesetManifest>> {
        requireSha256(sha256, 'Ruleset manifest');
        const [immutableDocument, publishedDocument] = await Promise.all([
            this.getJson(['ruleset-manifests', sha256], signal, {}, 'force-cache'),
            this.getJson(['published-rulesets', sha256], signal),
        ]);
        const [immutableManifest, published] = await Promise.all([
            parseAndVerifyRulesetManifest(immutableDocument, sha256),
            parseAndVerifyPublishedRuleset(publishedDocument, sha256),
        ]);
        if (published.operationalStatus.status !== 'active') {
            throw new PublicApiError(
                0,
                'ruleset_quarantined',
                'This immutable ruleset is quarantined and cannot be shown as an active ranked board.',
            );
        }
        // Both documents independently canonical-hash to the requested route
        // digest. This cross-binds mutable publication status to the exact
        // immutable ranking policy without trusting either response's labels.
        return immutableManifest;
    }

    async playerRuns(
        publicKey: string,
        cursor: string | null,
        signal?: AbortSignal,
    ): Promise<PlayerRunHistoryPage> {
        requireSha256(publicKey, 'Player public key');
        requireHistoryCursor(cursor);
        const page = parsePlayerRunHistoryPage(await this.getJson(
            ['players', publicKey, 'runs'],
            signal,
            { schema_version: 1, limit: PAGE_SIZE, cursor },
        ));
        if (page.player.publicKey !== publicKey) {
            throw new PublicApiError(
                0,
                'player_history_identity_mismatch',
                'The run-history response did not match its requested player.',
            );
        }
        return page;
    }

    async usernameChallenge(publicKey: string, signal?: AbortSignal): Promise<UsernameChallenge> {
        return parseUsernameChallenge(await this.sendJson(
            ['username-challenges'],
            'POST',
            { schema_version: 1, public_key: publicKey },
            signal,
        ));
    }

    async updateUsername(
        publicKey: string,
        envelope: UsernameUpdateEnvelope,
        signal?: AbortSignal,
    ): Promise<PlayerProfile> {
        return parsePlayerProfile(await this.sendJson(
            ['players', publicKey, 'username'],
            'PUT',
            envelope,
            signal,
        ));
    }

    async deletionChallenge(
        publicKey: string,
        target: DeletionTarget,
        signal?: AbortSignal,
    ): Promise<DeletionChallenge> {
        return parseDeletionChallenge(await this.sendJson(
            ['deletion-challenges'],
            'POST',
            { schema_version: 1, public_key: publicKey, target },
            signal,
        ));
    }

    async requestDeletion(
        envelope: DeletionRequestEnvelope,
        signal?: AbortSignal,
    ): Promise<DeletionReceipt> {
        return parseDeletionReceipt(await this.sendJson(
            ['deletion-requests'],
            'POST',
            envelope,
            signal,
        ));
    }

    async report(
        target: { readonly kind: 'run'; readonly run_id: string }
            | { readonly kind: 'player'; readonly public_key: string },
        category: AbuseReportCategory,
        detail: string,
        signal?: AbortSignal,
    ): Promise<AbuseReportAccepted> {
        return parseAbuseReportAccepted(await this.sendJson(
            ['reports'],
            'POST',
            { schema_version: 1, target, category, detail },
            signal,
        ));
    }

    replayUrl(id: string): string {
        return apiUrl(this.base, ['runs', id, 'replay']);
    }

    campaignSessionReplayUrl(aggregateRunId: string, ordinal: number): string {
        requireSessionOrdinal(ordinal);
        return apiUrl(this.base, ['runs', aggregateRunId, 'sessions', String(ordinal), 'replay']);
    }

    async replayBytes(id: string, signal?: AbortSignal): Promise<Uint8Array> {
        return await this.replayBytesAt(this.replayUrl(id), signal);
    }

    async campaignSessionReplayBytes(
        aggregateRunId: string,
        ordinal: number,
        signal?: AbortSignal,
    ): Promise<Uint8Array> {
        return await this.replayBytesAt(
            this.campaignSessionReplayUrl(aggregateRunId, ordinal),
            signal,
        );
    }

    private async replayBytesAt(
        url: string,
        signal?: AbortSignal,
    ): Promise<Uint8Array> {
        const deadline = new NetworkDeadline(signal, 'Replay request', this.#requestTimeoutMs);
        try {
            const response = await deadline.race(fetch(
                url,
                requestInit(deadline.signal),
            ));
            await requireSuccess(response, deadline);
            requireExactMediaType(
                response,
                'application/x-robin-rhrec+compact',
                'Replay',
                deadline,
            );
            return await readBounded(response, MAX_REPLAY_BYTES, 'Replay', deadline);
        } catch (error) {
            throw publicDeadlineError(deadline, error);
        } finally {
            deadline.dispose();
        }
    }

    private async getJson(
        path: readonly string[],
        signal?: AbortSignal,
        params: Readonly<Record<string, string | number | null | undefined>> = {},
        cache: RequestCache = 'no-store',
    ): Promise<unknown> {
        return this.requestJson(apiUrl(this.base, path, params), signal,
            requestSignal => requestInit(requestSignal, 'application/json', cache));
    }

    private async sendJson(
        path: readonly string[],
        method: 'POST' | 'PUT',
        body: unknown,
        signal?: AbortSignal,
    ): Promise<unknown> {
        return this.requestJson(apiUrl(this.base, path), signal, requestSignal => ({
            ...requestInit(requestSignal, 'application/json'),
            method,
            headers: { accept: 'application/json', 'content-type': 'application/json' },
            body: JSON.stringify(body),
        }));
    }

    private async requestJson(
        url: string,
        signal: AbortSignal | undefined,
        init: (signal: AbortSignal) => RequestInit,
    ): Promise<unknown> {
        const deadline = new NetworkDeadline(signal, 'API request', this.#requestTimeoutMs);
        try {
            const response = await deadline.race(fetch(url, init(deadline.signal)));
            await requireSuccess(response, deadline);
            const text = new TextDecoder('utf-8', { fatal: true })
                .decode(await readBounded(response, MAX_JSON_BYTES, 'JSON', deadline));
            try {
                return JSON.parse(text) as unknown;
            } catch {
                throw new PublicApiError(0, 'invalid_json', 'The server returned invalid JSON.');
            }
        } catch (error) {
            throw publicDeadlineError(deadline, error);
        } finally {
            deadline.dispose();
        }
    }
}

function requireSha256(value: string, label: string): void {
    if (!/^[0-9a-f]{64}$/u.test(value) || /^0+$/u.test(value)) {
        throw new PublicApiError(0, 'invalid_digest', `${label} identity is not a non-zero lowercase SHA-256 digest.`);
    }
}

function requireHistoryCursor(value: string | null): void {
    if (value === null) return;
    if (value.length === 0
        || new TextEncoder().encode(value).byteLength > 4096
        || value.trim() !== value
        || /\p{Cc}|\p{Bidi_Control}/u.test(value)) {
        throw new PublicApiError(0, 'invalid_player_history_cursor', 'Player history cursor is invalid.');
    }
}

function requireSessionOrdinal(value: number): void {
    if (!Number.isSafeInteger(value) || value < 0 || value > 0xffff_ffff) {
        throw new PublicApiError(0, 'invalid_session_ordinal', 'Campaign session ordinal is invalid.');
    }
}

function requireExactMediaType(
    response: Response,
    expected: string,
    label: string,
    deadline: NetworkDeadline,
): void {
    if (response.headers.get('content-type') === expected) return;
    deadline.cancelBody(response.body);
    throw new PublicApiError(
        0,
        'unexpected_media_type',
        `${label} response did not use its authenticated media type.`,
    );
}

function requestInit(
    signal: AbortSignal | undefined,
    accept?: string,
    cache: RequestCache = 'no-store',
): RequestInit {
    return {
        method: 'GET',
        ...(accept === undefined ? {} : { headers: { accept } }),
        credentials: 'omit',
        mode: 'cors',
        cache,
        redirect: 'error',
        referrerPolicy: 'no-referrer',
        ...(signal === undefined ? {} : { signal }),
    };
}

async function requireSuccess(response: Response, deadline: NetworkDeadline): Promise<void> {
    if (response.ok) {
        return;
    }
    let code = `http_${response.status}`;
    let message = `Highscore API returned HTTP ${response.status}.`;
    try {
        const bytes = await readBounded(response, 16 * 1024, 'Error', deadline);
        const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)) as unknown;
        if (typeof value === 'object' && value !== null && Reflect.get(value, 'schema_version') === 1) {
            const error = Reflect.get(value, 'error');
            const candidateCode = typeof error === 'object' && error !== null
                ? Reflect.get(error, 'code')
                : undefined;
            const candidateMessage = typeof error === 'object' && error !== null
                ? Reflect.get(error, 'message')
                : undefined;
            if (typeof candidateCode === 'string' && /^[a-z0-9_]{1,100}$/u.test(candidateCode)) {
                code = candidateCode;
            }
            if (typeof candidateMessage === 'string' && candidateMessage.length <= 500) {
                message = candidateMessage;
            }
        }
    } catch {
        // The stable status-derived error remains more useful than leaking a raw body.
    }
    throw new PublicApiError(response.status, code, message);
}

async function readBounded(
    response: Response,
    limit: number,
    label: string,
    deadline: NetworkDeadline,
): Promise<Uint8Array> {
    const raw = response.headers.get('content-length');
    if (raw !== null) {
        const length = Number(raw);
        if (!Number.isSafeInteger(length) || length < 0 || length > limit) {
            deadline.cancelBody(response.body, tooLarge(label));
            throw tooLarge(label);
        }
    }
    if (response.body === null) return new Uint8Array();
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let total = 0;
    try {
        while (true) {
            const { value, done } = await deadline.race(reader.read());
            if (done) break;
            total += value.byteLength;
            if (total > limit) {
                void reader.cancel(tooLarge(label)).catch(() => {
                    // The size error remains authoritative.
                });
                throw tooLarge(label);
            }
            chunks.push(value);
        }
    } catch (error) {
        void reader.cancel(error).catch(() => {
            // The original bounded/timeout error remains authoritative.
        });
        throw error;
    }
    const output = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
        output.set(chunk, offset);
        offset += chunk.byteLength;
    }
    return output;
}

function tooLarge(label: string): PublicApiError {
    return new PublicApiError(0, 'response_too_large', `${label} response exceeded the browser safety limit.`);
}

function publicDeadlineError(deadline: NetworkDeadline, error: unknown): unknown {
    if (!deadline.timedOut) return error;
    return new PublicApiError(
        0,
        'network_timeout',
        'The highscore service did not finish its response before the browser deadline.',
    );
}
