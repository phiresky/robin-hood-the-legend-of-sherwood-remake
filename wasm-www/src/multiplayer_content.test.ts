import assert from 'node:assert/strict';
import test from 'node:test';

import {
    parseMultiplayerBuildManifest,
    prepareMultiplayerContent,
} from './multiplayer_content.ts';
import type { VerifiedBrowserJoinTicket } from './join_ticket.ts';

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
            net_protocol: 26,
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

function manifest(nativeContentSha256 = HOST_CONTENT): unknown {
    return {
        commit: COMMIT,
        short: COMMIT.slice(0, 12),
        netProtocol: 26,
        ticketSchema: 3,
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
