import assert from 'node:assert/strict';
import test from 'node:test';
import { signerProtocolTestHooks as hooks, SignerTransport } from './signing.js';
import { JSDOM } from 'jsdom';

const protocol = 'robinhood.browser-identity.v1';
const requestId = 'ab'.repeat(16);
const publicKey = '12'.repeat(32);

test('signer transport ignores forged readiness and responses and closes pending work on pagehide', async t => {
    const dom = new JSDOM('<!doctype html><body></body>', { url: 'https://robinhood.phiresky.xyz' });
    const originals = ['window', 'document'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
    Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    t.after(() => {
        dom.window.dispatchEvent(new dom.window.Event('pagehide'));
        dom.window.close();
        for (const [key, value] of originals) {
            if (value) Object.defineProperty(globalThis, key, value);
            else Reflect.deleteProperty(globalThis, key);
        }
    });
    const origin = 'https://identity.robinhood.phiresky.xyz';
    const connection = SignerTransport.connect(origin);
    let connected = false;
    void connection.then(() => { connected = true; });
    const frame = dom.window.document.querySelector('iframe')!;
    assert.equal(frame.getAttribute('sandbox'), 'allow-scripts allow-same-origin');
    assert.equal(frame.referrerPolicy, 'no-referrer');
    const send = (data: unknown, source: Window | null, from = origin) => dom.window.dispatchEvent(new dom.window.MessageEvent('message', { data, origin: from, source }));
    send({ protocol, kind: 'ready' }, null);
    send({ protocol, kind: 'ready' }, frame.contentWindow, 'https://attacker.example');
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(connected, false);
    send({ protocol, kind: 'ready' }, frame.contentWindow);
    const transport = await connection;
    const requests: { data: { requestId: string }; origin: string }[] = [];
    frame.contentWindow!.postMessage = ((data: { requestId: string }, to: string) => { requests.push({ data, origin: to }); }) as typeof window.postMessage;
    const pending = transport.request('public_key');
    let resolved = false;
    void pending.then(() => { resolved = true; });
    assert.equal(requests[0]?.origin, origin);
    const response = { protocol, requestId: requests[0]!.data.requestId, ok: true, result: { kind: 'public_key', publicKey } };
    send(response, null);
    send(response, frame.contentWindow, 'https://attacker.example');
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(resolved, false);
    send(response, frame.contentWindow);
    assert.deepEqual(await pending, response.result);
    const interrupted = transport.request('status');
    const rejected = assert.rejects(interrupted, /closed|disconnected/u);
    dom.window.dispatchEvent(new dom.window.Event('pagehide'));
    await rejected;
    assert.equal(frame.isConnected, false);
});

test('identity signer response contract accepts only frozen typed results', () => {
    assert.equal(hooks.isReadyEnvelope({ protocol, kind: 'ready' }), true);
    assert.deepEqual(hooks.parseResponseEnvelope({
        protocol, requestId, ok: true, result: { kind: 'public_key', publicKey },
    }), { requestId, ok: true, result: { kind: 'public_key', publicKey } });
    assert.deepEqual(hooks.parseStatusResult({ kind: 'status', publicKey }), {
        kind: 'status', publicKey,
    });
    assert.deepEqual(hooks.parsePublicKeyResult({ kind: 'public_key', publicKey }), {
        kind: 'public_key', publicKey,
    });
    assert.deepEqual(hooks.parseSignedDocumentResult({
        kind: 'signed_document', documentJson: '{"schema_version":1}',
    }), { kind: 'signed_document', documentJson: '{"schema_version":1}' });
});

test('identity signer response contract rejects substitution and schema expansion', () => {
    for (const response of [
        { protocol: 'other', requestId, ok: true, result: {} },
        { protocol, requestId: 'short', ok: true, result: {} },
        { protocol, requestId, ok: true, result: {}, extra: true },
        { protocol, requestId, ok: false, error: { code: 'BAD', message: 'bad' } },
        { protocol, requestId, ok: false, error: { code: 'bad', message: 'bad', extra: true } },
    ]) {
        assert.throws(() => hooks.parseResponseEnvelope(response));
    }
    assert.equal(hooks.isReadyEnvelope({ protocol, kind: 'ready', extra: true }), false);
    assert.throws(() => hooks.parseStatusResult({
        kind: 'status', publicKey: null,
    }), /public key is invalid/u);
    assert.throws(() => hooks.parsePublicKeyResult({
        kind: 'public_key', publicKey: '0'.repeat(64),
    }), /invalid public key/u);
    assert.throws(() => hooks.parseSignedDocumentResult({
        kind: 'participant_signature', participantSignatureJson: '{}',
    }), /missing or unknown fields/u);
});

test('no legacy rekey state or raw-key fields remain in the public parser surface', () => {
    assert.deepEqual(Object.keys(hooks).sort(), [
        'isReadyEnvelope',
        'parsePublicKeyResult',
        'parseResponseEnvelope',
        'parseSignedDocumentResult',
        'parseStatusResult',
    ]);
    assert.throws(() => hooks.parseStatusResult({
        kind: 'status', state: 'rekey_required', publicKey: null,
    }), /missing or unknown fields/u);
});
