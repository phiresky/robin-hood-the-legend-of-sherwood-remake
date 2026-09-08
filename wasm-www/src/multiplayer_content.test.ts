import assert from 'node:assert/strict';
import test from 'node:test';

import {
    parseMultiplayerBuildManifest,
    prepareMultiplayerContent,
} from './multiplayer_content.ts';
import {
    browserJoinTicketConstants,
    type VerifiedBrowserJoinTicket,
} from './join_ticket.ts';

const COMMIT = '0123456789abcdef0123456789abcdef01234567';
const HOST_CONTENT = '01'.repeat(32);

function ticket(contentIdentity = HOST_CONTENT): VerifiedBrowserJoinTicket {
    return {
        code: 'rhmp3-test',
        canonicalPayload: new Uint8Array(),
        signature: new Uint8Array(),
        payload: {
            schema: 3,
            transport: 'iroh-relay-websocket',
            net_protocol: browserJoinTicketConstants.netProtocolVersion,
            engine_version: COMMIT,
            host_endpoint_id: '02'.repeat(32),
            host_public_key: 'A'.repeat(43),
            relay_url: 'https://relay.example.invalid/',
            session_id: 'A'.repeat(43),
            issued_at_epoch_s: 1,
            expires_at_epoch_s: 1801,
            content_edition: 'demo',
            content_identity_sha256: contentIdentity,
            mission_id: 'Dem_Lei_MP',
            mission_profile_id: 4,
            expected_players: 2,
        },
    };
}

function manifest(
    nativeContentSha256 = HOST_CONTENT,
    javascriptModulePath = 'snippets/robin_rs-build/js/browser_identity_client.js',
): unknown {
    return {
        commit: COMMIT,
        short: COMMIT.slice(0, 12),
        netProtocol: browserJoinTicketConstants.netProtocolVersion,
        ticketSchema: 3,
        javascriptModules: [{
            path: javascriptModulePath,
            byteLength: 42,
            sha256: '05'.repeat(32),
        }],
        multiplayerContent: {
            schema: 2,
            demo: {
                url: 'https://assets.example.invalid/demo.rhdata.zst',
                sha256: '03'.repeat(32),
                byteLength: 123,
                nativeContentSha256,
            },
            full: null,
        },
    };
}

test('catalog binds Demo bytes to the signed native host closure', async () => {
    const hostTicket = ticket();
    const parsed = parseMultiplayerBuildManifest(manifest(), hostTicket);
    assert.equal(parsed.multiplayerContent.demo.nativeContentSha256, HOST_CONTENT);

    const wrong = parseMultiplayerBuildManifest(manifest('04'.repeat(32)), hostTicket);
    await assert.rejects(
        prepareMultiplayerContent(hostTicket, wrong, async () => {
            throw new Error('Full picker must not run for Demo');
        }),
        /does not match the native host content closure/,
    );
});

test('catalog rejects the private identity vault at every module depth', () => {
    for (const path of [
        'browser_identity_vault.js',
        'snippets/robin_rs-build/js/browser_identity_vault.js',
    ]) {
        assert.throws(
            () => parseMultiplayerBuildManifest(manifest(HOST_CONTENT, path), ticket()),
            /is not an allowed imported module/,
        );
    }
});

test('Demo content forwards cancellation and does not wait for stalled fetch providers', async t => {
    const controller = new AbortController();
    let started!: () => void;
    const entered = new Promise<void>(resolve => { started = resolve; });
    t.mock.method(globalThis, 'fetch', async (_url: unknown, init: RequestInit) => {
        assert.equal(init.signal, controller.signal);
        started();
        return new Promise<Response>(() => {});
    });
    const hostTicket = ticket();
    const pending = prepareMultiplayerContent(hostTicket,
        parseMultiplayerBuildManifest(manifest(), hostTicket),
        async () => { throw new Error('Demo must not open the Full picker'); }, controller.signal);
    await entered;
    controller.abort();
    await assert.rejects(pending, { name: 'AbortError' });
});
