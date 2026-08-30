import assert from 'node:assert/strict';
import test from 'node:test';

import {
    browserJoinTicketConstants,
    captureAndScrubBrowserJoinCode,
    type BrowserJoinTicketPayload,
    verifyBrowserJoinTicket,
} from './join_ticket.ts';

const NOW = 2_000_000_000;
const DOMAIN = new TextEncoder().encode('robinhood/browser-join-ticket/v3\0');

test('uses the current Rust join-ticket and network schemas', () => {
    assert.equal(browserJoinTicketConstants.schema, 3);
    assert.equal(browserJoinTicketConstants.netProtocolVersion, 31);
});

function base64Url(bytes: Uint8Array): string {
    return Buffer.from(bytes).toString('base64url');
}

function hex(bytes: Uint8Array): string {
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function signedTicket(
    change: Partial<BrowserJoinTicketPayload> = {},
): Promise<{ code: string; payload: BrowserJoinTicketPayload }> {
    const keys = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);
    const publicKey = new Uint8Array(await crypto.subtle.exportKey('raw', keys.publicKey));
    const payload: BrowserJoinTicketPayload = {
        schema: browserJoinTicketConstants.schema,
        transport: 'iroh-relay-websocket',
        net_protocol: browserJoinTicketConstants.netProtocolVersion,
        engine_version: '0123456789abcdef0123456789abcdef01234567',
        host_endpoint_id: hex(publicKey),
        host_public_key: base64Url(publicKey),
        relay_url: 'https://relay.example.invalid/',
        session_id: base64Url(new Uint8Array(32).fill(9)),
        issued_at_epoch_s: NOW,
        expires_at_epoch_s: NOW + browserJoinTicketConstants.invitationLifetimeSeconds,
        content_edition: 'demo',
        content_identity_sha256: '01'.repeat(32),
        mission_id: 'Dem_Lei_MP',
        mission_profile_id: 4,
        expected_players: 2,
        ...change,
    };
    const payloadBytes = new TextEncoder().encode(JSON.stringify(payload));
    const message = new Uint8Array(DOMAIN.byteLength + payloadBytes.byteLength);
    message.set(DOMAIN);
    message.set(payloadBytes, DOMAIN.byteLength);
    const signature = new Uint8Array(await crypto.subtle.sign('Ed25519', keys.privateKey, message));
    return {
        code: `rhmp3-${base64Url(payloadBytes)}.${base64Url(signature)}`,
        payload,
    };
}

test('accepts any canonical signed HTTPS relay and exact ticket', async () => {
    const { code, payload } = await signedTicket({
        relay_url: 'https://third-party-relay.example:444/custom-path',
    });
    const verified = await verifyBrowserJoinTicket(code, NOW);
    assert.deepEqual(verified.payload, payload);
    assert.equal(verified.code, code);
});

test('rejects payload tampering and non-canonical relay URLs', async () => {
    const signed = await signedTicket();
    const parts = signed.code.slice('rhmp3-'.length).split('.');
    const payloadBytes = Buffer.from(parts[0] as string, 'base64url');
    const missionOffset = payloadBytes.indexOf('Dem_Lei_MP');
    assert.notEqual(missionOffset, -1);
    payloadBytes[missionOffset] = 'X'.charCodeAt(0);
    const tampered = `rhmp3-${payloadBytes.toString('base64url')}.${parts[1]}`;
    await assert.rejects(verifyBrowserJoinTicket(tampered, NOW), /signature is invalid/);

    const nonCanonical = await signedTicket({ relay_url: 'https://RELAY.example.invalid/' });
    await assert.rejects(verifyBrowserJoinTicket(nonCanonical.code, NOW), /canonical HTTPS/);
});

test('enforces exact unused expiry and bounded clock skew', async () => {
    const { code } = await signedTicket();
    await verifyBrowserJoinTicket(
        code,
        NOW + browserJoinTicketConstants.invitationLifetimeSeconds - 1,
    );
    await assert.rejects(
        verifyBrowserJoinTicket(code, NOW + browserJoinTicketConstants.invitationLifetimeSeconds),
        /expired before first use/,
    );
    await verifyBrowserJoinTicket(
        code,
        NOW + browserJoinTicketConstants.invitationLifetimeSeconds,
        true,
    );
    await assert.rejects(
        verifyBrowserJoinTicket(code, NOW - browserJoinTicketConstants.maxClockSkewSeconds - 1),
        /too far in the future/,
    );
});

test('scrubs invitation fragment without changing query parameters', () => {
    const calls: string[] = [];
    Object.defineProperty(globalThis, 'history', {
        configurable: true,
        value: {
            state: { retained: true },
            replaceState: (_state: unknown, _unused: string, url: string): void => {
                calls.push(url);
            },
        },
    });
    const code = captureAndScrubBrowserJoinCode(
        new URL('https://game.example/play?binaries-base=local#join=rhmp3-public.ticket'),
    );
    assert.equal(code, 'rhmp3-public.ticket');
    assert.deepEqual(calls, ['/play?binaries-base=local']);
});
