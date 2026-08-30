import assert from 'node:assert/strict';
import test from 'node:test';

import {
    MULTIPLAYER_IDENTITY_PROTOCOL,
    browserSeatProofMessage,
    dispatchMultiplayerIdentityRequest,
    type MultiplayerIdentitySigner,
} from './multiplayer_identity_protocol.ts';

const REQUEST_ID = '0123456789abcdef0123456789abcdef';
const SESSION_ID = Buffer.from(new Uint8Array(32).fill(1)).toString('base64url');
const HOST_ID = '02'.repeat(32);
const TRANSPORT_ID = '03'.repeat(32);

function fakeSigner(calls: string[]): MultiplayerIdentitySigner {
    return {
        status: async () => ({ publicKey: Buffer.alloc(32, 4).toString('base64url'), persistent: true }),
        wasRedeemed: async sessionId => {
            calls.push(`was:${sessionId}`);
            return false;
        },
        markRedeemed: async sessionId => {
            calls.push(`mark:${sessionId}`);
        },
        signSeatProof: async (sessionId, host, transport) => {
            calls.push(`sign:${sessionId}:${host}:${transport}`);
            return Buffer.alloc(64, 5).toString('base64url');
        },
    };
}

test('dispatch exposes only typed redemption and seat-proof operations', async () => {
    const calls: string[] = [];
    const response = await dispatchMultiplayerIdentityRequest({
        protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
        requestId: REQUEST_ID,
        operation: 'sign_seat_proof',
        sessionId: SESSION_ID,
        hostEndpointId: HOST_ID,
        transportEndpointId: TRANSPORT_ID,
    }, fakeSigner(calls));
    assert.equal(response.ok, true);
    assert.deepEqual(calls, [`sign:${SESSION_ID}:${HOST_ID}:${TRANSPORT_ID}`]);

    const generic = await dispatchMultiplayerIdentityRequest({
        protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
        requestId: REQUEST_ID,
        operation: 'sign',
        message: 'attacker-selected',
    }, fakeSigner(calls));
    assert.equal(generic.ok, false);
    if (generic.ok) assert.fail('generic signing unexpectedly succeeded');
    assert.equal(generic.error.code, 'invalid_operation');
});

test('seat proof message matches the Rust domain and fixed binary layout', () => {
    const message = browserSeatProofMessage(SESSION_ID, HOST_ID, TRANSPORT_ID);
    const domain = new TextEncoder().encode('robinhood/browser-seat-proof/v1\0');
    assert.deepEqual(message.slice(0, domain.byteLength), domain);
    assert.deepEqual(message.slice(domain.byteLength, domain.byteLength + 32), new Uint8Array(32).fill(1));
    assert.deepEqual(message.slice(domain.byteLength + 32, domain.byteLength + 64), new Uint8Array(32).fill(2));
    assert.deepEqual(message.slice(domain.byteLength + 64), new Uint8Array(32).fill(3));
});

test('protocol rejects malformed ids and additional fields before signer access', async () => {
    const calls: string[] = [];
    for (const request of [
        {
            protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
            requestId: REQUEST_ID,
            operation: 'was_redeemed',
            sessionId: 'A'.repeat(43),
        },
        {
            protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
            requestId: REQUEST_ID,
            operation: 'sign_seat_proof',
            sessionId: SESSION_ID,
            hostEndpointId: 'AB'.repeat(32),
            transportEndpointId: TRANSPORT_ID,
        },
        {
            protocol: MULTIPLAYER_IDENTITY_PROTOCOL,
            requestId: REQUEST_ID,
            operation: 'status',
            payload: 'unexpected',
        },
    ]) {
        const response = await dispatchMultiplayerIdentityRequest(request, fakeSigner(calls));
        assert.equal(response.ok, false);
    }
    assert.deepEqual(calls, []);
});
