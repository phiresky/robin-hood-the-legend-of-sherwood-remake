import { cp, copyFile, lstat, mkdir, mkdtemp, readdir, rename, rm } from 'node:fs/promises';
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyDatadirDeploymentReceipt } from './datadir-release-authority.mjs';
import { stageCloudflareHeaders } from './stage-cloudflare-headers.mjs';
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

/** Assemble only `/wasm/*`, binding but never copying the datadir corpus. */
export async function assembleRuntimeCorpus({
    existing,
    addition,
    datadirAuthority,
    datadirDeployment,
    output,
}) {
    const additionRoot = resolve(addition);
    const authorityPath = resolve(datadirAuthority);
    const deploymentPath = resolve(datadirDeployment);
    const outputRoot = resolve(output);
    const additionAuthority = await verifyRuntimeCorpus(additionRoot, { addition: true });
    const deployedDatadir = await verifyDatadirDeploymentReceipt({
        authorityPath,
        receiptPath: deploymentPath,
    });
    requireDemoBinding(additionAuthority, deployedDatadir);

    let existingRoot;
    if (existing !== null) {
        existingRoot = resolve(existing);
        await verifyRuntimeCorpus(existingRoot, { datadirAuthorityPath: authorityPath });
    }
    await requireAbsent(outputRoot);
    requireDisjointOutput(outputRoot, [
        additionRoot,
        authorityPath,
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
        } else {
            for (const entry of await readdir(existingRoot)) {
                await cp(resolve(existingRoot, entry), resolve(stagingRoot, entry), {
                    recursive: true,
                    force: false,
                    errorOnExist: true,
                });
            }
            const destination = resolve(stagingRoot, 'wasm', additionAuthority.latest.short);
            await requireAbsent(destination);
            await cp(resolve(additionRoot, 'wasm', additionAuthority.latest.short), destination, {
                recursive: true,
                force: false,
                errorOnExist: true,
            });
            await copyFile(resolve(additionRoot, 'wasm/latest.json'), resolve(stagingRoot, 'wasm/latest.json'));
        }
        await copyFile(deploymentPath, resolve(stagingRoot, DATADIR_BINDING_PATH));
        await stageCloudflareHeaders('runtime', stagingRoot);
        const metrics = await verifyRuntimeCorpus(stagingRoot, { datadirAuthorityPath: authorityPath });
        await requireAbsent(outputRoot);
        await rename(stagingRoot, outputRoot);
        return metrics;
    } catch (error) {
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
    } else if (mode === '--update' && args.length === 5) {
        options = {
            existing: args[0],
            addition: args[1],
            datadirAuthority: args[2],
            datadirDeployment: args[3],
            output: args[4],
        };
    } else {
        throw new Error('usage: node scripts/assemble-runtime-corpus.mjs --initial ADDITION DATADIR_AUTHORITY DATADIR_DEPLOYMENT OUTPUT | --update EXISTING ADDITION DATADIR_AUTHORITY DATADIR_DEPLOYMENT OUTPUT');
    }
    const metrics = await assembleRuntimeCorpus(options);
    console.log(`assembled complete wasm runtime corpus: ${metrics.assetCount} assets, ${metrics.totalBytes} bytes`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
