import test from 'node:test';
import assert from 'node:assert/strict';
import { boundedText, submitDiagnostic } from './diagnostics.ts';

test('UTF-8 truncation respects byte limits without splitting characters', () => {
    assert.equal(boundedText('a😀b', 4), 'a');
    assert.equal(boundedText('a😀b', 5), 'a😀');
});
test('diagnostic submission checks receipt and refuses failed requests', async () => {
    const body = '{"schema_version":1}';
    const expected = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body))), b => b.toString(16).padStart(2, '0')).join('');
    const fetcher: typeof fetch = async (url, options) => {
        assert.equal(url, '/api/v1/diagnostics');
        assert.equal(options?.redirect, 'error');
        assert.equal(options?.body, body);
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
        const bytes = new TextEncoder().encode(String(options?.body));
        const id = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), b => b.toString(16).padStart(2, '0')).join('');
        return Response.json({ schema_version: 1, report_id: id }, { status: 202 });
    };
    try {
        const reporter = installDiagnostics();
        reporter.setBuild('abc1234');
        reporter.log('game diagnostics');
        dom.window.document.querySelector('textarea')!.value = 'Character is stuck';
        dom.window.document.querySelector('form')!.dispatchEvent(new dom.window.Event('submit', { cancelable: true }));
        await new Promise(resolve => setTimeout(resolve, 20));
        assert.equal(dom.window.localStorage.length, 1);
        const stored = JSON.parse(dom.window.localStorage.getItem(dom.window.localStorage.key(0)!)!);
        assert.equal(stored.description, 'Character is stuck');
        assert.equal(stored.engine_commit, 'abc1234');
        assert.match(stored.recent_log, /game diagnostics/);
        online = true;
        dom.window.dispatchEvent(new dom.window.Event('online'));
        for (let retry = 0; retry < 50 && dom.window.localStorage.length > 0; retry++) await new Promise(resolve => setTimeout(resolve, 10));
        assert.equal(dom.window.localStorage.length, 0);
        assert.match(dom.window.document.querySelector('#bug-report-status')!.textContent!, /Report submitted:/);
    } finally {
        globalThis.fetch = oldFetch;
        if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow); else Reflect.deleteProperty(globalThis, 'window');
        if (oldDocument) Object.defineProperty(globalThis, 'document', oldDocument); else Reflect.deleteProperty(globalThis, 'document');
        dom.window.close();
    }
});
