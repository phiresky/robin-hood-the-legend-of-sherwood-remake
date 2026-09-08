import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// Historical name retained for operator scripts. CI checks this document
// against compiled Rust with export_runtime_contract --check.
export function validateRuntimeSourceContract(value) {
    const keys = ['schema', 'netProtocol', 'replaySchema', 'ticketSchema',
        'joinCodePrefix', 'contentSchema', 'shippingDatadirSchema'];
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || Object.keys(value).length !== keys.length
        || keys.some(key => !Object.hasOwn(value, key))) {
        throw new Error('runtime contract has missing or unknown fields');
    }
    if (value.schema !== 1) throw new Error('unsupported runtime contract schema');
    for (const key of keys.filter(key => key !== 'joinCodePrefix')) {
        if (!Number.isSafeInteger(value[key]) || value[key] <= 0) {
            throw new Error(`invalid runtime contract ${key}`);
        }
    }
    if (value.joinCodePrefix !== `rhmp${value.ticketSchema}-`) {
        throw new Error('runtime contract join-code prefix does not bind ticket schema');
    }
    return Object.freeze({ ...value });
}

export async function verifyRuntimeSourceContract(repoRoot = resolve(import.meta.dirname, '..', '..')) {
    const document = await readFile(resolve(repoRoot, 'wasm-www/runtime-contract.json'), 'utf8');
    return validateRuntimeSourceContract(JSON.parse(document));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const [option, extra] = process.argv.slice(2);
    if (extra !== undefined || (option !== undefined && option !== '--json')) {
        throw new Error('usage: node scripts/verify-runtime-source-contract.mjs [--json]');
    }
    const contract = await verifyRuntimeSourceContract();
    console.log(option === '--json' ? JSON.stringify(contract)
        : `verified runtime contract: protocol ${contract.netProtocol}, replay ${contract.replaySchema}, ticket ${contract.ticketSchema}`);
}
