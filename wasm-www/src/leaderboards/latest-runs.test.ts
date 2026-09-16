import assert from 'node:assert/strict';
import test from 'node:test';
import { parseLatestRuns } from './public-response.js';
import { playerHistoryPage } from './model-fixtures.js';

test('latest submissions decode named and anonymous runs without exposing private fields', () => {
    const history = playerHistoryPage();
    const entry = (history.runs as Record<string, unknown>[])[0]!;
    const run = entry.run as Record<string, unknown>;
    const item = { run, verified_at_unix_ms: entry.verified_at_unix_ms };
    const page = { schema_version: 1, runs: [item, { ...item, run: { ...run, run_id: 'anonymous-run', uploader: null } }] };
    const parsed = parseLatestRuns(page);
    assert.equal(parsed.length, 2);
    assert.notEqual(parsed[0]!.run.uploader, null);
    assert.equal(parsed[1]!.run.uploader, null);
    assert.deepEqual(parseLatestRuns({ schema_version: 1, runs: [] }), []);
    assert.throws(() => parseLatestRuns({ ...page, schema_version: 2 }));
    assert.throws(() => parseLatestRuns({ ...page, runs: Array(11).fill(item) }));
    assert.throws(() => parseLatestRuns({ ...page, runs: [{ ...item, uploader_public_key: 'private' }] }));
    assert.throws(() => parseLatestRuns({ ...page, runs: [{ ...item, verified_at_unix_ms: -1 }] }));
});
