import assert from 'node:assert/strict';
import test from 'node:test';
import { fullGameContentBuild } from './mission-launch.ts';

test('full-game mission launch selects matching hosted content without overriding replay content', () => {
    assert.equal(fullGameContentBuild('latest', 'full', null, false), 'latest');
    assert.equal(fullGameContentBuild('latest', 'demo', null, false), null);
    assert.equal(fullGameContentBuild('latest', null, null, false), null);
    assert.equal(fullGameContentBuild('latest', 'full', null, true), null);
    assert.equal(fullGameContentBuild('latest', 'demo', { edition: 'full', runtimeBuild: 'recorded' }, false), 'recorded');
    assert.equal(fullGameContentBuild('latest', 'full', { edition: 'demo', runtimeBuild: 'recorded' }, false), null);
});
