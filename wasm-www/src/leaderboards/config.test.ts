import assert from 'node:assert/strict';
import test from 'node:test';
import { apiUrl, ownerWritesArePinned, resolveApiBase } from './config.js';

test('API configuration uses explicit precedence and normalizes trailing slashes', () => {
    assert.equal(resolveApiBase({
        pageUrl: 'http://localhost:5173/game/leaderboards/',
        queryValue: 'http://127.0.0.1:8080/api/v1///',
        globalValue: 'http://localhost:8081/api/v1',
    }), 'http://127.0.0.1:8080/api/v1');
});

test('production API configuration is the immutable same-origin route', () => {
    assert.equal(resolveApiBase({
        pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
    }), '/api/v1');
    assert.equal(ownerWritesArePinned({
        pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
    }), true);
});

test('API configuration permits local HTTP but rejects production overrides', () => {
    assert.equal(resolveApiBase({
        pageUrl: 'http://localhost:5173/leaderboards/',
        queryValue: 'http://127.0.0.1:8080/api/v1',
    }), 'http://127.0.0.1:8080/api/v1');
    assert.throws(() => resolveApiBase({
        pageUrl: 'https://robinhood.phiresky.xyz/',
        buildValue: 'http://api.example/api/v1',
    }), /overrides are forbidden/u);
    assert.throws(() => resolveApiBase({
        pageUrl: 'https://robinhood.phiresky.xyz/',
        buildValue: 'https://user:secret@api.example/api/v1',
    }), /overrides are forbidden/u);
});

test('API route construction encodes opaque path components and query values', () => {
    assert.equal(
        apiUrl('https://api.example/api/v1', ['runs', 'run/../../x'], { cursor: 'a+b&c', empty: null }),
        'https://api.example/api/v1/runs/run%2F..%2F..%2Fx?cursor=a%2Bb%26c',
    );
    assert.equal(
        apiUrl('/api/v1', ['runs', 'run/../../x'], { cursor: 'a+b&c', empty: null }),
        '/api/v1/runs/run%2F..%2F..%2Fx?cursor=a%2Bb%26c',
    );
});

test('privileged writes are enabled only for a deployment-pinned or loopback API', () => {
    assert.equal(ownerWritesArePinned({
        pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
        queryValue: 'https://attacker.example/api/v1',
    }), false);
    assert.equal(ownerWritesArePinned({
        pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
    }), true);
    assert.equal(ownerWritesArePinned({
        pageUrl: 'http://localhost:5173/leaderboards/',
        queryValue: 'http://127.0.0.1:8080/api/v1',
    }), true);
});

test('hostile production API links are rejected before any privileged request', () => {
    assert.throws(() => resolveApiBase({
        pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/?api=https://attacker.example/api/v1',
        queryValue: 'https://attacker.example/api/v1',
    }), /overrides are forbidden/u);
});

test('production rejects mutable HTML and global configuration even when HTTPS', () => {
    for (const override of [
        { metaValue: 'https://attacker.example/api/v1' },
        { globalValue: 'https://attacker.example/api/v1' },
    ]) {
        assert.throws(() => resolveApiBase({
            pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
            ...override,
        }), /overrides are forbidden/u);
        assert.equal(ownerWritesArePinned({
            pageUrl: 'https://robinhood.phiresky.xyz/leaderboards/',
            ...override,
        }), false);
    }
});

test('local API configuration requires the frozen versioned route prefix', () => {
    assert.throws(() => resolveApiBase({
        pageUrl: 'http://localhost:5173/',
        buildValue: 'https://scores.example/v1',
    }), /exact \/api\/v1/u);
});
