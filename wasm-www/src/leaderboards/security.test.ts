import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { element, link, replace } from './dom.js';
import { participantView, aggregateParticipantView, appendAchievements, playerTables, campaignCompositionLabel } from './public-components.js';
import { parsePlayerRunHistoryPage, parseRunDetail } from './public-response.js';
import { playerHistoryPage, fullCampaignRun } from './model-fixtures.js';

function browser(t: test.TestContext): Document {
    const original = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const dom = new JSDOM('<!doctype html><body></body>');
    Object.defineProperty(globalThis, 'document', { configurable: true, value: dom.window.document });
    t.after(() => {
        dom.window.close();
        if (original) Object.defineProperty(globalThis, 'document', original);
        else Reflect.deleteProperty(globalThis, 'document');
    });
    return dom.window.document;
}

test('actual participant views keep hostile names inert and expose only public identity', t => {
    const document = browser(t);
    const name = '<img src=x onerror=alert(1)><script>bad()</script>';
    const publicKey = '12'.repeat(32), fingerprint = 'owner fingerprint';
    const playerLink = (key: string, label: string) => link(label, '?player=' + key);
    for (const view of [
        participantView({ seat: 1, username: name, publicKey, publicKeyFingerprint: fingerprint }, playerLink),
        aggregateParticipantView({ currentDisplayName: name, publicKey, publicKeyFingerprint: fingerprint }, playerLink),
    ]) {
        replace(document.body, view);
        assert.equal(document.querySelector('a')?.textContent, name);
        assert.equal(document.querySelector('a')?.getAttribute('href'), '?player=' + publicKey);
        assert.equal(document.querySelector('.fingerprint')?.textContent, fingerprint);
        assert.match(document.querySelector('.fingerprint')?.getAttribute('title') ?? '', /only its owner/u);
        assert.equal(document.querySelectorAll('img, script').length, 0);
        assert.equal(document.body.textContent, name + fingerprint);
    }
});

test('achievement rendering awards only earned decisions and keeps all labels inert', t => {
    const document = browser(t);
    const proof = element('aside');
    appendAchievements(proof, [
        { id: 'earned', label: '<img src=x>', evaluation: 'earned' },
        { id: 'missing', label: '<script>missing</script>', evaluation: 'unverifiable' },
        { id: 'failed', label: 'not earned', evaluation: 'not_earned' },
    ]);
    document.body.append(proof);
    assert.deepEqual([...document.querySelectorAll('.badge')].map(node => node.textContent), ['<img src=x>']);
    assert.match(document.querySelector('.notice')?.textContent ?? '', /Not awarded because verification was unavailable/u);
    assert.equal(document.querySelectorAll('img, script').length, 0);
    assert.equal(document.body.textContent?.includes('not earned'), false);
});

test('player tables expose labelled sections, captions, scoped columns and inert public subject text', t => {
    const document = browser(t);
    const page = parsePlayerRunHistoryPage(playerHistoryPage());
    const hostile = '<img src=x onerror=alert(1)>';
    const tables = playerTables((id, label) => link(label, '?run=' + id), () => element('nav'));
    const bests = page.personalBests.map(best => ({ ...best, filter: { ...best.filter,
        subject: { kind: 'mission' as const, category: 'individual_level' as const, missionId: hostile },
    } }));
    document.body.append(tables.renderPlayerPersonalBests(bests), tables.renderPlayerRunHistory(page, null));
    assert.equal(document.querySelectorAll('table').length, 2);
    for (const section of document.querySelectorAll('section')) {
        const heading = section.getAttribute('aria-labelledby');
        assert.ok(heading !== null && document.getElementById(heading));
    }
    for (const table of document.querySelectorAll('table')) {
        assert.ok(table.querySelector('caption')?.textContent);
        assert.ok([...table.querySelectorAll('th')].every(header => header.scope === 'col'));
    }
    assert.ok(document.body.textContent?.includes(hostile));
    assert.equal(document.querySelectorAll('img,script').length, 0);
});

test('full campaign composition renders the public aggregate identity and ordered session count', t => {
    const document = browser(t);
    const run = parseRunDetail(fullCampaignRun());
    const label = campaignCompositionLabel(run);
    document.body.append(element('dd', { text: label }));
    assert.equal(document.body.textContent, `${run.runId} · ${run.fullCampaignSessions.length} ordered sessions`);
    assert.doesNotMatch(label, /chain_id|chainId/u);
});

test('leaderboard CSP pins the dedicated signer and has no executable blob exception', () => {
    const html = readFileSync(new URL('../../leaderboards/index.html', import.meta.url), 'utf8');
    assert.match(html, /script-src 'self';/u);
    assert.match(html, /frame-src https:\/\/identity\.robinhood\.phiresky\.xyz;/u);
    assert.match(html, /connect-src 'self';/u);
    assert.doesNotMatch(html, /(?:script-src|frame-src|style-src|object-src)[^;]*(?:blob:|\*)/u);
});


test('game-shell CSP confines executable sources and permits verified blob modules', () => {
    const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8');
    assert.match(html, /script-src 'self' blob: 'wasm-unsafe-eval';/u);
    assert.doesNotMatch(html, /github\.io/u);
    assert.doesNotMatch(html, /script-src[^;]*\shttps:\s/u);
    assert.doesNotMatch(html, /script-src[^;]*'unsafe-eval'/u);
    assert.match(html, /connect-src 'self' https: wss:/u);
    assert.match(html, /frame-src https:\/\/identity\.robinhood\.phiresky\.xyz;/u);
    assert.match(html, /object-src 'none'/u);
});
