import test from 'node:test';
import assert from 'node:assert/strict';
import { boundedText, submitDiagnostic, compressDiagnostic, diagnosticQueue } from './diagnostics.ts';
import { IDBFactory } from 'fake-indexeddb';
import { randomBytes } from 'node:crypto';

async function uploadText(body: BodyInit | null | undefined): Promise<string> {
    assert.ok(body instanceof Blob);
    return new Response(body.stream().pipeThrough(new DecompressionStream('gzip'))).text();
}

test('UTF-8 truncation respects byte limits without splitting characters', () => {
    assert.equal(boundedText('a😀b', 4), 'a');
    assert.equal(boundedText('a😀b', 5), 'a😀');
});
test('diagnostic submission checks receipt and refuses failed requests', async () => {
    const body = '{"schema_version":1,"kind":"bug","engine_commit":"test"}';
    const expected = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', await (await compressDiagnostic(body)).arrayBuffer())), b => b.toString(16).padStart(2, '0')).join('');
    const fetcher: typeof fetch = async (url, options) => {
        assert.equal(url, '/api/v1/diagnostics');
        assert.equal(options?.redirect, 'error');
        assert.equal(new Headers(options?.headers).get('Content-Encoding'), 'gzip');
        assert.equal(new Headers(options?.headers).get('X-Diagnostic-Kind'), 'bug');
        assert.equal(new Headers(options?.headers).get('X-Diagnostic-Engine-Commit'), 'test');
        assert.equal(await uploadText(options?.body), body);
        return Response.json({ schema_version: 1, report_id: expected }, { status: 202 });
    };
    assert.equal(await submitDiagnostic(body, fetcher), expected);
    await assert.rejects(submitDiagnostic(body, async () => new Response('', { status: 503 })), /503/);
    await assert.rejects(submitDiagnostic(body, async () => Response.json({ schema_version: 1, report_id: 'wrong' }, { status: 202 })), /Invalid report receipt/);
});

test('browser form retains failed reports and retries on reconnect', async () => {
    const { JSDOM } = await import('jsdom');
    const { installDiagnostics } = await import('./diagnostics.ts');
    const dom = new JSDOM('<button id="report-bug"></button><dialog id="bug-report-dialog"><form id="bug-report-form"><textarea id="bug-report-description"></textarea><p id="bug-report-status"></p><button id="bug-report-send"></button><button id="bug-report-close"></button></form></dialog>', { url: 'https://game.test/' });
    const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const oldDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const oldFetch = globalThis.fetch;
    Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    let online = false;
    globalThis.fetch = async (_url, options) => {
        if (!online) return new Response('', { status: 503 });
        assert.ok(options?.body instanceof Blob);
        const bytes = await options.body.arrayBuffer();
        const id = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), b => b.toString(16).padStart(2, '0')).join('');
        return Response.json({ schema_version: 1, report_id: id }, { status: 202 });
    };
    try {
        const queue = diagnosticQueue(new IDBFactory());
        const reporter = installDiagnostics(queue);
        reporter.setBuild('abc1234');
        reporter.log('game diagnostics');
        dom.window.document.querySelector('textarea')!.value = 'Character is stuck';
        dom.window.document.querySelector('form')!.dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
        for (let retry = 0; retry < 100 && (await queue.list()).length === 0; retry++) await new Promise(resolve => setTimeout(resolve, 10));
        assert.equal((await queue.list()).length, 1);
        const stored = JSON.parse((await queue.list())[0]!.body);
        assert.equal(stored.description, 'Character is stuck');
        assert.equal(stored.engine_commit, 'abc1234');
        assert.match(stored.recent_log, /game diagnostics/);
        // Wait for the initial failed request before reconnecting.
        for (let retry = 0; retry < 100 && !dom.window.document.querySelector('#bug-report-status')!.textContent!.includes('remains queued'); retry++) await new Promise(resolve => setTimeout(resolve, 10));
        online = true;
        dom.window.dispatchEvent(new dom.window.Event('online'));
        for (let retry = 0; retry < 100 && (await queue.list()).length > 0; retry++) await new Promise(resolve => setTimeout(resolve, 10));
        assert.equal((await queue.list()).length, 0);
        assert.match(dom.window.document.querySelector('#bug-report-status')!.textContent!, /Report submitted:/);
    } finally {
        globalThis.fetch = oldFetch;
        if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow); else Reflect.deleteProperty(globalThis, 'window');
        if (oldDocument) Object.defineProperty(globalThis, 'document', oldDocument); else Reflect.deleteProperty(globalThis, 'document');
        dom.window.close();
    }
});

test('large reports are limited by gzip size and survive IndexedDB reopening', async () => {
    const body = JSON.stringify({ recent_log: 'x'.repeat(24 * 1024 * 1024) });
    const compressed = await compressDiagnostic(body);
    assert.ok(compressed.size < 20 * 1024 * 1024);
    assert.equal(await uploadText(compressed), body);
    const factory = new IDBFactory();
    await diagnosticQueue(factory).put({ id: 'large-report', body });
    const reopened = diagnosticQueue(factory);
    assert.equal((await reopened.list())[0]!.body, body);
    await reopened.remove('large-report');
    assert.equal((await reopened.list()).length, 0);
});

test('reports exceeding the compressed limit are rejected before fetch', async () => {
    const body = JSON.stringify({ recent_log: randomBytes(101 * 1024 * 1024).toString('base64') });
    let uploaded = false;
    await assert.rejects(submitDiagnostic(body, async () => {
        uploaded = true;
        throw new Error('unexpected upload');
    }), /100 MiB compressed upload limit/);
    assert.equal(uploaded, false);
});

test('automatic reports ignore resize notifications and coalesce panic/trap duplicates', async t => {
    const { JSDOM } = await import('jsdom');
    const { installDiagnostics } = await import('./diagnostics.ts');
    const dom = new JSDOM('<button id="report-bug"></button><dialog id="bug-report-dialog"><form id="bug-report-form"><textarea id="bug-report-description"></textarea><p id="bug-report-status"></p><button id="bug-report-send"></button></form></dialog>', { url: 'https://game.test/' });
    const previous = ['window', 'document', 'fetch'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
    t.after(() => {
        for (const [key, descriptor] of previous) {
            if (descriptor) Object.defineProperty(globalThis, key, descriptor);
            else Reflect.deleteProperty(globalThis, key);
        }
        dom.window.close();
    });
    Object.defineProperty(globalThis, 'window', { configurable: true, value: dom.window });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    globalThis.fetch = async () => new Response('', { status: 503 });
    const queue = diagnosticQueue(new IDBFactory());
    const reporter = installDiagnostics(queue);
    for (let i = 0; i < 10; i++) {
        dom.window.dispatchEvent(new dom.window.ErrorEvent('error', {
            message: 'ResizeObserver loop completed with undelivered notifications.',
        }));
    }
    // An unexplained WASM trap is still actionable.
    reporter.failure(new WebAssembly.RuntimeError('unreachable'));
    reporter.log('panicked at engine.rs:12: missing entity');
    reporter.failure(new WebAssembly.RuntimeError('unreachable executed'));
    reporter.log('panicked at engine.rs:12: missing entity');
    const missionFailure = "Mission launch failed: Level load failed: IO error: chunk version mismatch in 'SCOT': expected 4, found 5";
    reporter.log(`ERROR run.rs:1000 ${missionFailure}`);
    reporter.log(`ERROR run.rs:1000 ${missionFailure}`);
    for (let retry = 0; retry < 100 && (await queue.list()).length < 3; retry++) await new Promise(resolve => setTimeout(resolve, 10));
    const reports = (await queue.list()).map(row => JSON.parse(row.body));
    assert.equal(reports.length, 3);
    assert.deepEqual(reports.map(report => report.description).sort(), [
        missionFailure, 'panicked at engine.rs:12: missing entity', 'unreachable',
    ]);
    assert.equal(reports.find(report => report.description.includes('panicked at')).kind, 'panic');
    const missionReport = reports.find(report => report.description === missionFailure);
    assert.equal(missionReport.kind, 'bug');
    assert.match(missionReport.recent_log, /chunk version mismatch in 'SCOT'/);
});
