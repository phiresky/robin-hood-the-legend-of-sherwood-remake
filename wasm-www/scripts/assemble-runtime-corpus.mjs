import { chmod, cp, copyFile, lstat, mkdir, mkdtemp, readdir, rename, rm } from 'node:fs/promises';
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyDatadirDeploymentReceipt } from './datadir-release-authority.mjs';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';
import { RETAINED_DEMO_GENERATIONS } from './verify-datadir-corpus.mjs';
import { DATADIR_BINDING_PATH, verifyRuntimeCorpus } from './verify-runtime-corpus.mjs';

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
 * Retained runtime corpora are archived read-only and `cp` preserves directory
 * modes. Only the private staging copy's directories become owner-writable, so
 * a new build can be added and a failed staging tree can be removed. The
 * archived source is never modified.
 */
async function makeStagingDirectoriesWritable(directory) {
    const facts = await lstat(directory);
    if (!facts.isDirectory()) return;
    await chmod(directory, facts.mode | 0o700);
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        if (entry.isDirectory()) await makeStagingDirectoriesWritable(resolve(directory, entry.name));
    }
}

/**
 * Replace a mutable pointer file in the staging copy. Archived files may be
 * read-only, so the staged copy is unlinked (its directory is writable) instead
 * of being opened for writing.
 */
async function replaceStagedFile(source, destination) {
    await rm(destination, { force: true });
    await copyFile(source, destination);
}

function requireDemoBinding(runtime, deployment) {
    const declared = runtime.latest.multiplayerContent.demo;
    const deployed = deployment.receipt.demo;
    if (declared.url !== deployed.datadir_url
        || declared.byteLength !== deployed.datadir_byte_length
        || declared.sha256 !== deployed.datadir_sha256
        || declared.nativeContentSha256 !== deployed.native_content_sha256) {
        throw new Error('runtime addition does not authorize the deployed Demo datadir receipt');
    }
}

/**
 * Assemble only `/wasm/*`, binding but never copying the datadir corpus.
 * When the datadir corpus gained a new generation, the existing runtime corpus
 * still carries the receipt of the prior datadir deployment; pass that prior
 * release authority as `existingDatadirAuthority`.
 */
export async function assembleRuntimeCorpus({
    existing,
    addition,
    datadirAuthority,
    datadirDeployment,
    output,
    existingDatadirAuthority = datadirAuthority,
    retainedGenerations = [],
}) {
    const additionRoot = resolve(addition);
    const authorityPath = resolve(datadirAuthority);
    const existingAuthorityPath = resolve(existingDatadirAuthority);
    const deploymentPath = resolve(datadirDeployment);
    const outputRoot = resolve(output);
    const additionAuthority = await verifyRuntimeCorpus(additionRoot, { addition: true });
    const deployedDatadir = await verifyDatadirDeploymentReceipt({
        authorityPath,
        receiptPath: deploymentPath,
        retainedGenerations,
    });
    requireDemoBinding(additionAuthority, deployedDatadir);

    let existingRoot;
    if (existing !== null) {
        existingRoot = resolve(existing);
        // Its `_headers` is replaced below; the output is verified strictly.
        await verifyRuntimeCorpus(existingRoot, {
            datadirAuthorityPath: existingAuthorityPath, retainedGenerations, replacedHeaders: true,
        });
    }
    await requireAbsent(outputRoot);
    requireDisjointOutput(outputRoot, [
        additionRoot,
        authorityPath,
        existingAuthorityPath,
        deploymentPath,
        ...(existingRoot === undefined ? [] : [existingRoot]),
    ]);

    await mkdir(dirname(outputRoot), { recursive: true });
    const stagingRoot = await mkdtemp(resolve(dirname(outputRoot), `.${basename(outputRoot)}.assembling-`));
    try {
        if (existingRoot === undefined) {
            await cp(resolve(additionRoot, 'wasm'), resolve(stagingRoot, 'wasm'), {
                recursive: true,
                force: false,
                errorOnExist: true,
            });
            await makeStagingDirectoriesWritable(stagingRoot);
        } else {
            for (const entry of await readdir(existingRoot)) {
                await cp(resolve(existingRoot, entry), resolve(stagingRoot, entry), {
                    recursive: true,
                    force: false,
                    errorOnExist: true,
                });
            }
            await makeStagingDirectoriesWritable(stagingRoot);
            const destination = resolve(stagingRoot, 'wasm', additionAuthority.latest.short);
            await requireAbsent(destination);
            await cp(resolve(additionRoot, 'wasm', additionAuthority.latest.short), destination, {
                recursive: true,
                force: false,
                errorOnExist: true,
            });
            await replaceStagedFile(resolve(additionRoot, 'wasm/latest.json'), resolve(stagingRoot, 'wasm/latest.json'));
        }
        await replaceStagedFile(deploymentPath, resolve(stagingRoot, DATADIR_BINDING_PATH));
        await rm(resolve(stagingRoot, '_headers'), { force: true });
        await stageCloudflareHeaders('runtime', stagingRoot);
        const metrics = await verifyRuntimeCorpus(stagingRoot, { datadirAuthorityPath: authorityPath, retainedGenerations });
        await requireAbsent(outputRoot);
        await rename(stagingRoot, outputRoot);
        return metrics;
    } catch (error) {
        await makeStagingDirectoriesWritable(stagingRoot);
        await rm(stagingRoot, { recursive: true, force: true });
        throw error;
    }
}

async function main() {
    const [mode, ...args] = process.argv.slice(2);
    let options;
    if (mode === '--initial' && args.length === 4) {
        options = {
            existing: null,
            addition: args[0],
            datadirAuthority: args[1],
            datadirDeployment: args[2],
            output: args[3],
        };
    } else if (mode === '--update' && (args.length === 5 || args.length === 6)) {
        options = {
            existing: args[0],
            addition: args[1],
            datadirAuthority: args[2],
            datadirDeployment: args[3],
            output: args[4],
            ...(args.length === 6 ? { existingDatadirAuthority: args[5] } : {}),
        };
    } else {
        throw new Error('usage: node scripts/assemble-runtime-corpus.mjs --initial ADDITION DATADIR_AUTHORITY DATADIR_DEPLOYMENT OUTPUT | --update EXISTING ADDITION DATADIR_AUTHORITY DATADIR_DEPLOYMENT OUTPUT [EXISTING_DATADIR_AUTHORITY]');
    }
    const metrics = await assembleRuntimeCorpus({ ...options, retainedGenerations: RETAINED_DEMO_GENERATIONS });
    console.log(`assembled complete wasm runtime corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
