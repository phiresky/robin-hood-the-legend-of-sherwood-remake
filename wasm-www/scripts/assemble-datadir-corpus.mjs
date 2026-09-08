import { cp, copyFile, lstat, mkdir, mkdtemp, readdir, rename, rm } from 'node:fs/promises';
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';
import { verifyDatadirCorpus, verifyDemoWebContentPackage } from './verify-datadir-corpus.mjs';

async function requireAbsent(path) {
    if (await lstat(path).catch(() => undefined) !== undefined) {
        throw new Error(`output already exists; choose a new directory: ${path}`);
    }
}

function containsPath(parent, child) {
    const remainder = relative(parent, child);
    return remainder === '' || (!remainder.startsWith('..') && !isAbsolute(remainder));
}

function requireDisjointOutput(output, inputs) {
    for (const input of inputs) {
        if (containsPath(input, output) || containsPath(output, input)) {
            throw new Error(`output must be disjoint from every input: ${output} overlaps ${input}`);
        }
    }
}

async function copyEntries(source, destinationRoot) {
    for (const entry of source.copyEntries) {
        const destination = resolve(destinationRoot, entry.destination);
        await mkdir(dirname(destination), { recursive: true });
        await copyFile(resolve(source.root, entry.source), destination);
    }
}

/**
 * Assemble only the public Demo byte closure for the dedicated datadir
 * Worker. The immutable update form proves the complete closure is unchanged.
 */
export async function assembleDatadirCorpus({ existing, demo, output }) {
    const demoRoot = resolve(demo);
    const outputRoot = resolve(output);
    const demoAuthority = await verifyDemoWebContentPackage(demoRoot);
    let existingRoot;
    if (existing !== null) {
        existingRoot = resolve(existing);
        const retained = await verifyDatadirCorpus(existingRoot);
        if (retained.stableClosure !== demoAuthority.stableClosure) {
            throw new Error('supplied Demo converter output differs from the immutable deployed datadir corpus');
        }
    }
    await requireAbsent(outputRoot);
    requireDisjointOutput(outputRoot, [demoRoot, ...(existingRoot === undefined ? [] : [existingRoot])]);
    await mkdir(dirname(outputRoot), { recursive: true });
    const stagingRoot = await mkdtemp(resolve(dirname(outputRoot), `.${basename(outputRoot)}.assembling-`));
    try {
        if (existingRoot === undefined) {
            await copyEntries(demoAuthority, stagingRoot);
        } else {
            for (const entry of await readdir(existingRoot)) {
                if (entry === '_headers') continue;
                await cp(resolve(existingRoot, entry), resolve(stagingRoot, entry), {
                    recursive: true,
                    force: false,
                    errorOnExist: true,
                });
            }
        }
        await stageCloudflareHeaders('datadir', stagingRoot);
        const result = await verifyDatadirCorpus(stagingRoot);
        await requireAbsent(outputRoot);
        await rename(stagingRoot, outputRoot);
        return result;
    } catch (error) {
        await rm(stagingRoot, { recursive: true, force: true });
        throw error;
    }
}

async function main() {
    const [mode, ...args] = process.argv.slice(2);
    let options;
    if (mode === '--initial' && args.length === 2) {
        options = { existing: null, demo: args[0], output: args[1] };
    } else if (mode === '--update' && args.length === 3) {
        options = { existing: args[0], demo: args[1], output: args[2] };
    } else {
        throw new Error('usage: node scripts/assemble-datadir-corpus.mjs --initial DEMO_CONVERTER_OUTPUT OUTPUT | --update EXISTING DEMO_CONVERTER_OUTPUT OUTPUT');
    }
    const metrics = await assembleDatadirCorpus(options);
    console.log(`assembled standalone Demo datadir corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
