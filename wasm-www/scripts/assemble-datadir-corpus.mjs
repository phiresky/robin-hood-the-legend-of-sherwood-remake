import { constants } from 'node:fs';
import { chmod, cp, copyFile, lstat, mkdir, mkdtemp, readdir, rename, rm } from 'node:fs/promises';
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';
import {
    RETAINED_DEMO_GENERATIONS,
    verifyDatadirCorpus,
    verifyDemoWebContentPackage,
} from './verify-datadir-corpus.mjs';

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

/**
 * Retained corpora are archived read-only and `cp` preserves directory modes.
 * Only the private staging copy's directories become owner-writable, so a new
 * generation can be added and a failed staging tree can be removed.
 */
async function makeStagingDirectoriesWritable(directory) {
    const facts = await lstat(directory);
    if (!facts.isDirectory()) return;
    await chmod(directory, facts.mode | 0o700);
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        if (entry.isDirectory()) await makeStagingDirectoriesWritable(resolve(directory, entry.name));
    }
}

async function copyEntries(source, destinationRoot) {
    for (const entry of source.copyEntries) {
        const destination = resolve(destinationRoot, entry.destination);
        await mkdir(dirname(destination), { recursive: true });
        await copyFile(resolve(source.root, entry.source), destination, constants.COPYFILE_EXCL);
    }
}

/**
 * Assemble only the public Demo byte closure for the dedicated datadir
 * Worker. The update form retains every existing object byte-for-byte: it
 * either proves the current generation is unchanged or adds the current
 * generation beside the retained ones.
 */
export async function assembleDatadirCorpus({ existing, demo, output, retainedGenerations = [] }) {
    const demoRoot = resolve(demo);
    const outputRoot = resolve(output);
    const demoAuthority = await verifyDemoWebContentPackage(demoRoot);
    let existingRoot;
    let existingHasCurrent = false;
    if (existing !== null) {
        existingRoot = resolve(existing);
        const retained = await verifyDatadirCorpus(existingRoot, { retainedGenerations, requireCurrent: false });
        existingHasCurrent = retained.stableClosure !== undefined;
        if (existingHasCurrent && retained.stableClosure !== demoAuthority.stableClosure) {
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
            await makeStagingDirectoriesWritable(stagingRoot);
            // A new generation is added beside every retained object; the
            // exclusive copy refuses to replace any published path.
            if (!existingHasCurrent) await copyEntries(demoAuthority, stagingRoot);
        }
        await stageCloudflareHeaders('datadir', stagingRoot);
        const result = await verifyDatadirCorpus(stagingRoot, { retainedGenerations });
        await requireAbsent(outputRoot);
        await rename(stagingRoot, outputRoot);
        return result;
    } catch (error) {
        await makeStagingDirectoriesWritable(stagingRoot);
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
    const metrics = await assembleDatadirCorpus({ ...options, retainedGenerations: RETAINED_DEMO_GENERATIONS });
    console.log(`assembled standalone Demo datadir corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
