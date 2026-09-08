import assert from 'node:assert/strict';
import test from 'node:test';
import { validateRuntimeSourceContract, verifyRuntimeSourceContract } from './verify-runtime-source-contract.mjs';

test('generated contract validates without depending on source layout', async () => {
    const contract = await verifyRuntimeSourceContract();
    assert.equal(contract.schema, 1);
    assert.equal(contract.joinCodePrefix, `rhmp${contract.ticketSchema}-`);
});

test('contract rejects unknown schemas, invalid numbers and incompatible prefixes', async () => {
    const contract = await verifyRuntimeSourceContract();
    for (const override of [{ schema: 2 }, { replaySchema: 0 }, { netProtocol: 1.5 },
        { netProtocol: '39' }, { joinCodePrefix: 'rhmp2-' }, { extra: true }]) {
        assert.throws(() => validateRuntimeSourceContract({ ...contract, ...override }));
    }
    const missing = { ...contract };
    delete missing.contentSchema;
    assert.throws(() => validateRuntimeSourceContract(missing));
});
