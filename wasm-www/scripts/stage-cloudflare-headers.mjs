import { copyFile, lstat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const HEADER_SOURCE = Object.freeze({
    public: 'public-headers.txt',
    runtime: 'runtime-headers.txt',
    datadir: 'datadir-headers.txt',
    signer: 'signer-headers.txt',
});

export async function stageCloudflareHeaders(kind, outputDirectory) {
    const sourceName = HEADER_SOURCE[kind];
    if (sourceName === undefined) {
        throw new Error('header kind must be exactly public, runtime, datadir, or signer');
    }
    const output = resolve(outputDirectory);
    const facts = await lstat(output).catch(() => undefined);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
        throw new Error(`static output is not a real directory: ${output}`);
    }
    await copyFile(resolve(import.meta.dirname, '..', 'deploy', sourceName), resolve(output, '_headers'));
}

async function main() {
    const [kind, outputDirectory, extra] = process.argv.slice(2);
    if (kind === undefined || outputDirectory === undefined || extra !== undefined) {
        throw new Error('usage: node scripts/stage-cloudflare-headers.mjs public|runtime|datadir|signer DIST');
    }
    await stageCloudflareHeaders(kind, outputDirectory);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
