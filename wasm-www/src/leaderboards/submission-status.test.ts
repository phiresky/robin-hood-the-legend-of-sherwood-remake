import assert from 'node:assert/strict';
import test from 'node:test';
import { parsePublicSubmissionStatus, submissionPresentation } from './submission-status.js';

test('pending submission states never claim a verified result', () => {
    for (const state of ['queued', 'verifying', 'retry_pending']) {
        const status = parsePublicSubmissionStatus({ schema_version: 1, submission_id: 'sub_1', state: { state } }, 'sub_1');
        assert.equal(status.runId, null);
        assert.equal(submissionPresentation(status).pending, true);
        assert.match(submissionPresentation(status).title, /Unverified/u);
    }
});
test('only a verified status carries a run link', () => {
    const status = parsePublicSubmissionStatus({ schema_version: 1, submission_id: 'sub_1', state: { state: 'verified', run_id: 'run_1' } }, 'sub_1');
    assert.equal(status.runId, 'run_1');
    assert.equal(submissionPresentation(status).pending, false);
    for (const state of ['rejected', 'failed']) {
        const terminal = parsePublicSubmissionStatus({ schema_version: 1, submission_id: 'sub_1', state: { state } }, 'sub_1');
        assert.equal(terminal.runId, null);
        assert.equal(submissionPresentation(terminal).pending, false);
    }
});
test('public status rejects wrong IDs, private fields, future schemas and malformed states', () => {
    const value = { schema_version: 1, submission_id: 'sub_1', state: { state: 'queued' } };
    assert.throws(() => parsePublicSubmissionStatus(value, 'sub_2'));
    for (const invalid of [
        { ...value, schema_version: 2 },
        { ...value, campaign_chain_receipt: {} },
        { ...value, state: { state: 'queued', run_id: 'run_1' } },
        { ...value, state: { state: 'verified' } },
        { ...value, state: { state: 'verified', run_id: '' } },
        { ...value, state: { state: 'rejected', private_reason: 'secret' } },
        { ...value, state: { state: 'future' } },
    ]) assert.throws(() => parsePublicSubmissionStatus(invalid, 'sub_1'));
});
