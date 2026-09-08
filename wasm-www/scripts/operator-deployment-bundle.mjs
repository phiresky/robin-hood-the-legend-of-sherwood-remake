import { execFile as execFileCallback, spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { chmod, lstat, mkdir, mkdtemp, open, opendir, realpath, rm, rmdir, writeFile } from 'node:fs/promises';
import { basename, dirname, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { validateOperatorRouteAuthority } from './operator-cloudflare-routes.mjs';
import { verifyPublicBuild, verifySignerBuild } from './verify-static-build.mjs';
import { verifyStaticOriginInventory } from './verify-static-origin-inventory.mjs';
import { verifyRuntimeCorpus } from './verify-runtime-corpus.mjs';
import { verifyRuntimeSourceContract } from './verify-runtime-source-contract.mjs';

const execFile = promisify(execFileCallback);
const DIGEST = /^[0-9a-f]{64}$/u;
const COMMIT = /^[0-9a-f]{40}$/u;
const VERSION = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;
const NODE_VERSION = 'v24.19.0';
const PUBLIC_HOST = 'robinhood.phiresky.xyz';
const MATERIALIZATION_AUTHORITY_DIRECTORY = 'authorities/materialization';
const ORIGINS = Object.freeze({
    identity_signer: Object.freeze({
        bundleDirectory: 'origins/identity-signer',
        inventory: `${MATERIALIZATION_AUTHORITY_DIRECTORY}/inventories/cloudflare-identity-signer-v1.json`,
        materializationDirectory: 'cloudflare-identity-signer',
    }),
    public: Object.freeze({
        bundleDirectory: 'origins/public',
        inventory: `${MATERIALIZATION_AUTHORITY_DIRECTORY}/inventories/cloudflare-public-v1.json`,
        materializationDirectory: 'cloudflare-public',
    }),
});
const RUNTIME = Object.freeze({ bundleDirectory: 'origins/runtime', inventory: 'inventories/runtime.json' });
const FORBIDDEN_STATIC_PATH = /(?:^|\/)(?:full|fullgame|fullgame_shipping|verifier-bundles?|datadirs?)(?:\/|$)/iu;
const FORBIDDEN_STATIC_TERMS = Object.freeze([
    'private',
    'projection-authority',
    'projection-exporter',
    'projection-receipt',
    'source-tree-manifest',
    'projection-execution',
    'verifier-source-binding',
    'campaign-state',
    'operator-config',
]);
const NOREPLACE_MV = '/usr/bin/mv';
const MATERIALIZATION_RECEIPT = 'cloudflare-publication-materialization-v1.json';
const MATERIALIZATION_SIDECAR = 'cloudflare-publication-materialization-v1.sha256';
const MATERIALIZATION_ORIGINS = Object.freeze({
    public: Object.freeze({
        inventoryPath: 'inventories/cloudflare-public-v1.json',
        root: 'cloudflare-public',
    }),
    identity_signer: Object.freeze({
        inventoryPath: 'inventories/cloudflare-identity-signer-v1.json',
        root: 'cloudflare-identity-signer',
    }),
    deployment_authority: Object.freeze({
        inventoryPath: 'inventories/deployment-authority-v1.json',
        root: 'deployment',
    }),
});
const MATERIALIZATION_ORIGIN_ORDER = Object.freeze(['public', 'identity_signer', 'deployment_authority']);
const MAX_TREE_DEPTH = 64;
const MAX_TREE_ENTRIES = 100_000;
const MAX_TREE_NAME_BYTES = 255;
const MAX_TREE_PATH_BYTES = 4096;
const MAX_REGULAR_FILE_BYTES = 25 * 1024 * 1024;
const MAX_TREE_BYTES = 512 * 1024 * 1024;
const PRIVATE_TEMP_ROOT = '/tmp';
const DEPLOYMENT_AUTHORITY_FILES = Object.freeze([
    'datadir-authority.json',
    'datadir-deployment.json',
    'exposure-v3.json',
]);
const DEPLOYMENT_CONFIG_FILES = Object.freeze([
    'wrangler-public.json',
    'wrangler-runtime.json',
    'wrangler-signer.json',
]);
const WRANGLER_SNAPSHOT_ORIGINS = Object.freeze({
    public: 'dist',
    runtime: 'runtime-dist',
    signer: 'signer-dist',
});

export class OperatorBundleInstalledButParentSyncFailed extends Error {
    constructor(destination, cause) {
        super(`operator deployment bundle was installed at ${destination}, but its parent directory could not be synchronized`, { cause });
        this.code = 'OPERATOR_BUNDLE_INSTALLED_PARENT_SYNC_FAILED';
        this.destination = destination;
        this.durability_unknown = true;
        this.installed = true;
        this.name = 'OperatorBundleInstalledButParentSyncFailed';
    }
}

export class OperatorBundleInstalledCommandError extends Error {
    constructor(destination, cause) {
        super(`operator deployment bundle was installed and fully validated at ${destination}, but the install command reported an error`, { cause });
        this.code = 'OPERATOR_BUNDLE_INSTALLED_COMMAND_ERROR';
        this.destination = destination;
        this.installed = true;
        this.name = 'OperatorBundleInstalledCommandError';
    }
}

export class OperatorBundleInstallStateUncertain extends Error {
    constructor(destination, cause) {
        super(`operator deployment bundle installation state is uncertain at ${destination}`, { cause });
        this.code = 'OPERATOR_BUNDLE_INSTALL_STATE_UNCERTAIN';
        this.destination = destination;
        this.installed = undefined;
        this.name = 'OperatorBundleInstallStateUncertain';
        this.state_uncertain = true;
    }
}

export class OperatorBundleInstalledInvalid extends Error {
    constructor(destination, cause) {
        super(`operator deployment bundle was installed at ${destination}, but mandatory post-install validation failed`, { cause });
        this.code = 'OPERATOR_BUNDLE_INSTALLED_INVALID';
        this.destination = destination;
        this.installed = true;
        this.name = 'OperatorBundleInstalledInvalid';
        this.valid = false;
    }
}

function sha256(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function requireExactNodeVersion(asserted) {
    if (process.version !== NODE_VERSION || asserted !== NODE_VERSION) {
        throw new Error(`operator deployment bundle requires exact Node.js ${NODE_VERSION}, got ${process.version} (asserted ${asserted})`);
    }
}
function utf8Order(left, right) { return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8')); }
function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value).sort(([a], [b]) => utf8Order(a, b))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}
function canonicalBytes(value, newline = true) { return Buffer.from(`${JSON.stringify(canonical(value))}${newline ? '\n' : ''}`); }
function containsForbiddenStaticPath(path) {
    const folded = path.toLowerCase();
    return FORBIDDEN_STATIC_PATH.test(path) || FORBIDDEN_STATIC_TERMS.some(term => folded.includes(term));
}

async function optionalLstat(path, options) {
    try {
        return await lstat(path, options);
    } catch (error) {
        if (error?.code === 'ENOENT') return undefined;
        throw error;
    }
}

function inodeIdentity(facts) { return `${facts.dev}:${facts.ino}`; }
function sameInode(facts, expectedIdentity) {
    return facts !== undefined && inodeIdentity(facts) === expectedIdentity;
}

function exactKeys(value, keys, label) {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
    const actual = Object.keys(value).sort(utf8Order);
    const expected = [...keys].sort(utf8Order);
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${label} has unexpected keys: ${actual.join(', ')}`);
}

async function parseJson(path, label, requireCanonical = false) {
    const bytes = (await stableRegularFile(path, label)).bytes;
    let value;
    try { value = JSON.parse(bytes.toString('utf8')); } catch (error) {
        throw new Error(`${label} is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (requireCanonical && !bytes.equals(canonicalBytes(value, false))) throw new Error(`${label} is not canonical compact JSON`);
    return { bytes, value };
}

function validateRelativePath(path, label, allowRoot = false) {
    if (typeof path !== 'string' || (path === '.' && !allowRoot) || (path !== '.' && (
        path.length === 0 || path.startsWith('/') || path.split('/').some(part => part === '' || part === '.' || part === '..')
    ))) throw new Error(`${label} is not a canonical relative path`);
}

function validateArtifactRef(artifact, label) {
    exactKeys(artifact, ['byte_length', 'media_type', 'sha256'], label);
    if (!Number.isSafeInteger(artifact.byte_length) || artifact.byte_length < 1
        || !DIGEST.test(artifact.sha256)
        || typeof artifact.media_type !== 'string' || artifact.media_type.length === 0
        || artifact.media_type.length > 128 || /[\u0000-\u001f\u007f]/u.test(artifact.media_type)) {
        throw new Error(`${label} is not an exact ArtifactRefV1`);
    }
}

function validateMaterializedTreeInventory(inventory, label) {
    exactKeys(inventory, ['directories', 'files'], label);
    if (!Array.isArray(inventory.files) || inventory.files.length === 0 || !Array.isArray(inventory.directories)
        || inventory.directories.length === 0) throw new Error(`${label} is empty or malformed`);
    let previous;
    const directories = new Set();
    for (const [index, entry] of inventory.directories.entries()) {
        exactKeys(entry, ['path', 'unix_mode'], `${label}.directories[${index}]`);
        validateRelativePath(entry.path, `${label}.directories[${index}].path`, true);
        if (entry.unix_mode !== 0o555 || (previous !== undefined && utf8Order(previous, entry.path) >= 0)) {
            throw new Error(`${label} directory inventory is not strictly sorted mode-0555 authority`);
        }
        previous = entry.path;
        directories.add(entry.path);
    }
    if (inventory.directories[0].path !== '.') throw new Error(`${label} does not begin with its root directory`);
    previous = undefined;
    for (const [index, entry] of inventory.files.entries()) {
        exactKeys(entry, ['artifact', 'path', 'unix_mode'], `${label}.files[${index}]`);
        validateRelativePath(entry.path, `${label}.files[${index}].path`);
        validateArtifactRef(entry.artifact, `${label}.files[${index}].artifact`);
        if (entry.unix_mode !== 0o444 || (previous !== undefined && utf8Order(previous, entry.path) >= 0)) {
            throw new Error(`${label} file inventory is not strictly sorted mode-0444 authority`);
        }
        const parent = dirname(entry.path) === '.' ? '.' : dirname(entry.path).split(sep).join('/');
        if (!directories.has(parent)) throw new Error(`${label} has a file whose parent directory is absent: ${entry.path}`);
        previous = entry.path;
    }
}

function validateMaterializedOriginInventory(inventory, origin) {
    const expected = MATERIALIZATION_ORIGINS[origin];
    exactKeys(inventory, ['directories', 'files', 'origin', 'root', 'schema_version'], `${origin} materialization inventory`);
    if (inventory.schema_version !== 1 || inventory.origin !== origin || inventory.root !== expected.root) {
        throw new Error(`${origin} materialization inventory has invalid fixed authority`);
    }
    validateMaterializedTreeInventory({
        directories: inventory.directories,
        files: inventory.files,
    }, `${origin} materialization inventory`);
    for (const path of [
        ...inventory.directories.map(entry => entry.path),
        ...inventory.files.map(entry => entry.path),
    ]) {
        if (path !== '.' && containsForbiddenStaticPath(path)) {
            throw new Error(`${origin} materialization inventory contains forbidden private authority path ${path}`);
        }
    }
    if (origin === 'deployment_authority'
        && (!sameJson(inventory.directories.map(entry => entry.path), ['.'])
            || !sameJson(inventory.files.map(entry => entry.path), DEPLOYMENT_AUTHORITY_FILES))) {
        throw new Error('deployment_authority materialization inventory is not the exact frozen deployment document closure');
    }
}

function materializedFile(path, artifact) { return { artifact, path, unix_mode: 0o444 }; }
function materializedDirectory(path) { return { path, unix_mode: 0o555 }; }
function genericMaterializationArtifact(artifact) {
    return { ...artifact, media_type: 'application/octet-stream' };
}

function expectedMaterializationOutputInventory(originInventories, inventoryArtifacts) {
    const files = [];
    const directories = new Set(['.', 'inventories']);
    for (const origin of MATERIALIZATION_ORIGIN_ORDER) {
        const inventory = originInventories[origin];
        for (const directory of inventory.directories) {
            directories.add(directory.path === '.' ? inventory.root : `${inventory.root}/${directory.path}`);
        }
        for (const file of inventory.files) {
            files.push(materializedFile(`${inventory.root}/${file.path}`, genericMaterializationArtifact(file.artifact)));
        }
        files.push(materializedFile(
            MATERIALIZATION_ORIGINS[origin].inventoryPath,
            genericMaterializationArtifact(inventoryArtifacts[origin]),
        ));
    }
    files.sort((left, right) => utf8Order(left.path, right.path));
    return {
        directories: [...directories].sort(utf8Order).map(materializedDirectory),
        files,
    };
}

function stableNodeIdentity(facts) {
    return [facts.dev, facts.ino, facts.mode, facts.nlink, facts.size, facts.mtimeNs, facts.ctimeNs]
        .map(value => value.toString()).join(':');
}

async function readBoundedFileHandle(handle, maximum) {
    const chunks = [];
    let offset = 0;
    while (offset <= maximum) {
        const length = Math.min(64 * 1024, maximum + 1 - offset);
        const chunk = Buffer.allocUnsafe(length);
        const { bytesRead } = await handle.read(chunk, 0, length, offset);
        if (bytesRead === 0) break;
        chunks.push(chunk.subarray(0, bytesRead));
        offset += bytesRead;
    }
    if (offset > maximum) throw new Error(`retained file exceeds bounded read limit ${maximum}`);
    return Buffer.concat(chunks, offset);
}

async function stableRegularFile(path, label, expectedFacts) {
    const before = await optionalLstat(path, { bigint: true });
    if (before === undefined || !before.isFile() || before.isSymbolicLink() || before.nlink !== 1n) {
        throw new Error(`${label} must be a singleton regular file`);
    }
    if (expectedFacts !== undefined && stableNodeIdentity(before) !== stableNodeIdentity(expectedFacts)) {
        throw new Error(`${label} changed before it was opened`);
    }
    if (before.size < 1n || before.size > BigInt(MAX_REGULAR_FILE_BYTES)) {
        throw new Error(`${label} exceeds the bounded regular-file size policy`);
    }
    const handle = await open(path, 'r');
    try {
        const opened = await handle.stat({ bigint: true });
        if (!opened.isFile() || opened.nlink !== 1n || stableNodeIdentity(before) !== stableNodeIdentity(opened)) {
            throw new Error(`${label} changed while it was opened`);
        }
        // The fstat size bound is authoritative for this retained descriptor;
        // readFile(handle) cannot be redirected by a path-entry replacement.
        const bytes = await readBoundedFileHandle(handle, MAX_REGULAR_FILE_BYTES);
        if (bytes.length < 1 || bytes.length > MAX_REGULAR_FILE_BYTES) {
            throw new Error(`${label} exceeds the bounded regular-file size policy`);
        }
        const retained = await handle.stat({ bigint: true });
        const after = await optionalLstat(path, { bigint: true });
        if (stableNodeIdentity(opened) !== stableNodeIdentity(retained)
            || after === undefined || stableNodeIdentity(opened) !== stableNodeIdentity(after)) {
            throw new Error(`${label} changed while it was read`);
        }
        return { bytes, facts: retained };
    } finally {
        await handle.close();
    }
}

async function physicalTree(root, label) {
    const files = new Map();
    const directories = new Map();
    let entriesSeen = 0;
    let regularBytes = 0n;
    let rootDevice;
    async function visit(directory, logicalDirectory, depth, expectedFacts) {
        if (depth > MAX_TREE_DEPTH) throw new Error(`${label} exceeds maximum directory depth ${MAX_TREE_DEPTH}`);
        const facts = await optionalLstat(directory, { bigint: true });
        if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
            throw new Error(`${label} contains a non-directory at ${logicalDirectory}`);
        }
        rootDevice ??= facts.dev;
        if (facts.dev !== rootDevice) throw new Error(`${label} crosses a mount boundary at ${logicalDirectory}`);
        if (expectedFacts !== undefined && stableNodeIdentity(facts) !== stableNodeIdentity(expectedFacts)) {
            throw new Error(`${label} directory changed before it was opened: ${logicalDirectory}`);
        }
        const retained = await open(directory, 'r');
        const opened = await retained.stat({ bigint: true });
        if (!opened.isDirectory() || stableNodeIdentity(facts) !== stableNodeIdentity(opened)) {
            await retained.close();
            throw new Error(`${label} directory changed while it was opened: ${logicalDirectory}`);
        }
        directories.set(logicalDirectory, facts);
        const entries = [];
        try {
            const handle = await opendir(`/proc/self/fd/${retained.fd}/.`);
            for await (const entry of handle) {
                entriesSeen += 1;
                if (entriesSeen > MAX_TREE_ENTRIES) throw new Error(`${label} exceeds maximum entry count ${MAX_TREE_ENTRIES}`);
                if (Buffer.byteLength(entry.name, 'utf8') > MAX_TREE_NAME_BYTES) {
                    throw new Error(`${label} contains an overlong entry name`);
                }
                entries.push(entry);
            }
            for (const entry of entries.sort((a, b) => utf8Order(a.name, b.name))) {
                const path = resolve(`/proc/self/fd/${retained.fd}`, entry.name);
                const logical = logicalDirectory === '.' ? entry.name : `${logicalDirectory}/${entry.name}`;
                if (Buffer.byteLength(logical, 'utf8') > MAX_TREE_PATH_BYTES) {
                    throw new Error(`${label} contains an overlong relative path`);
                }
                const child = await lstat(path, { bigint: true });
                if (child.isSymbolicLink()) throw new Error(`${label} contains symlink ${logical}`);
                if (child.dev !== rootDevice) throw new Error(`${label} crosses a mount boundary at ${logical}`);
                if (child.isDirectory()) await visit(path, logical, depth + 1, child);
                else if (child.isFile()) {
                    const value = await stableRegularFile(path, `${label} ${logical}`, child);
                    regularBytes += value.facts.size;
                    if (regularBytes > BigInt(MAX_TREE_BYTES)) throw new Error(`${label} exceeds maximum regular-file byte total`);
                    files.set(logical, value);
                } else throw new Error(`${label} contains non-regular entry ${logical}`);
            }
            const retainedAfter = await retained.stat({ bigint: true });
            const after = await optionalLstat(directory, { bigint: true });
            if (stableNodeIdentity(opened) !== stableNodeIdentity(retainedAfter)
                || after === undefined || stableNodeIdentity(opened) !== stableNodeIdentity(after)) {
                throw new Error(`${label} directory changed while it was traversed: ${logicalDirectory}`);
            }
        } finally {
            await retained.close();
        }
    }
    await visit(root, '.', 0);
    return { directories, files };
}

function verifyPhysicalTree(tree, inventory, label) {
    const expectedDirectories = new Set(inventory.directories.map(entry => entry.path));
    const expectedFiles = new Set(inventory.files.map(entry => entry.path));
    const missingDirectories = [...expectedDirectories].filter(path => !tree.directories.has(path));
    const extraDirectories = [...tree.directories.keys()].filter(path => !expectedDirectories.has(path));
    const missingFiles = [...expectedFiles].filter(path => !tree.files.has(path));
    const extraFiles = [...tree.files.keys()].filter(path => !expectedFiles.has(path));
    if ([missingDirectories, extraDirectories, missingFiles, extraFiles].some(items => items.length > 0)) {
        throw new Error(`${label} closure mismatch; missing directories [${missingDirectories.join(', ')}], extra directories [${extraDirectories.join(', ')}], missing files [${missingFiles.join(', ')}], extra files [${extraFiles.join(', ')}]`);
    }
    for (const entry of inventory.directories) {
        if (Number(tree.directories.get(entry.path).mode & 0o7777n) !== entry.unix_mode) {
            throw new Error(`${label} directory mode differs at ${entry.path}`);
        }
    }
    for (const entry of inventory.files) {
        const actual = tree.files.get(entry.path);
        if (Number(actual.facts.mode & 0o7777n) !== entry.unix_mode
            || actual.bytes.length !== entry.artifact.byte_length
            || sha256(actual.bytes) !== entry.artifact.sha256) {
            throw new Error(`${label} file authority differs at ${entry.path}`);
        }
    }
}

function parseCanonicalBytes(bytes, label) {
    let value;
    try { value = JSON.parse(bytes.toString('utf8')); } catch (error) {
        throw new Error(`${label} is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (!bytes.equals(canonicalBytes(value, false))) throw new Error(`${label} is not canonical compact JSON`);
    return value;
}

function materializationFinalInventory(receipt, receiptArtifact, sidecarArtifact) {
    return {
        directories: receipt.output_inventory.directories,
        files: [...receipt.output_inventory.files,
            materializedFile(MATERIALIZATION_RECEIPT, receiptArtifact),
            materializedFile(MATERIALIZATION_SIDECAR, sidecarArtifact),
        ].sort((left, right) => utf8Order(left.path, right.path)),
    };
}

function materializationAuthorityInventory(receipt, originInventories, receiptArtifact, sidecarArtifact) {
    const deployment = originInventories.deployment_authority;
    const directories = new Set(['.', 'inventories']);
    for (const directory of deployment.directories) {
        directories.add(directory.path === '.' ? 'deployment' : `deployment/${directory.path}`);
    }
    const files = deployment.files.map(file => materializedFile(`deployment/${file.path}`, file.artifact));
    for (const [index, origin] of MATERIALIZATION_ORIGIN_ORDER.entries()) {
        files.push(materializedFile(receipt.origins[index].inventory_path, receipt.origins[index].inventory));
    }
    files.push(materializedFile(MATERIALIZATION_RECEIPT, receiptArtifact));
    files.push(materializedFile(MATERIALIZATION_SIDECAR, sidecarArtifact));
    files.sort((left, right) => utf8Order(left.path, right.path));
    return { directories: [...directories].sort(utf8Order).map(materializedDirectory), files };
}

function parseMaterializationDocuments(tree, expectedReceiptSha256) {
    if (!DIGEST.test(expectedReceiptSha256)) throw new Error('approved materialization receipt SHA-256 is invalid');
    const receiptFile = tree.files.get(MATERIALIZATION_RECEIPT);
    const sidecarFile = tree.files.get(MATERIALIZATION_SIDECAR);
    if (receiptFile === undefined || sidecarFile === undefined) throw new Error('Cloudflare Publication materialization lacks its receipt or sidecar');
    if (sha256(receiptFile.bytes) !== expectedReceiptSha256
        || sidecarFile.bytes.toString('utf8') !== expectedReceiptSha256) {
        throw new Error('Cloudflare Publication materialization receipt differs from its independently approved digest or sidecar');
    }
    const receipt = parseCanonicalBytes(receiptFile.bytes, 'Cloudflare Publication materialization receipt');
    exactKeys(receipt, [
        'cargo_lock_sha256', 'origins', 'output_inventory', 'publication_lock_sha256',
        'publication_manifest_sha256', 'publication_schema_version', 'schema_version',
        'source_commit', 'source_tree_sha1',
    ], 'Cloudflare Publication materialization receipt');
    if (receipt.schema_version !== 1 || receipt.publication_schema_version !== 3
        || !COMMIT.test(receipt.source_commit) || !COMMIT.test(receipt.source_tree_sha1)
        || !DIGEST.test(receipt.cargo_lock_sha256) || !DIGEST.test(receipt.publication_manifest_sha256)
        || !DIGEST.test(receipt.publication_lock_sha256) || !Array.isArray(receipt.origins)
        || receipt.origins.length !== MATERIALIZATION_ORIGIN_ORDER.length) {
        throw new Error('Cloudflare Publication materialization receipt has invalid fixed authority');
    }
    validateMaterializedTreeInventory(receipt.output_inventory, 'Cloudflare Publication materialization output inventory');
    const originInventories = {};
    const inventoryArtifacts = {};
    for (const [index, origin] of MATERIALIZATION_ORIGIN_ORDER.entries()) {
        const expected = MATERIALIZATION_ORIGINS[origin];
        const binding = receipt.origins[index];
        exactKeys(binding, ['inventory', 'inventory_path', 'origin', 'root'], `materialization origins[${index}]`);
        validateArtifactRef(binding.inventory, `materialization origins[${index}].inventory`);
        if (binding.origin !== origin || binding.root !== expected.root
            || binding.inventory_path !== expected.inventoryPath || binding.inventory.media_type !== 'application/json') {
            throw new Error(`Cloudflare Publication materialization has substituted ${origin} origin authority`);
        }
        const inventoryFile = tree.files.get(expected.inventoryPath);
        if (inventoryFile === undefined || inventoryFile.bytes.length !== binding.inventory.byte_length
            || sha256(inventoryFile.bytes) !== binding.inventory.sha256) {
            throw new Error(`Cloudflare Publication materialization has substituted ${origin} inventory`);
        }
        const inventory = parseCanonicalBytes(inventoryFile.bytes, `${origin} materialization inventory`);
        validateMaterializedOriginInventory(inventory, origin);
        originInventories[origin] = inventory;
        inventoryArtifacts[origin] = binding.inventory;
    }
    const expectedOutput = expectedMaterializationOutputInventory(originInventories, inventoryArtifacts);
    if (!canonicalBytes(expectedOutput, false).equals(canonicalBytes(receipt.output_inventory, false))) {
        throw new Error('Cloudflare Publication materialization output inventory differs from its typed origin authorities');
    }
    const receiptArtifact = { byte_length: receiptFile.bytes.length, media_type: 'application/json', sha256: expectedReceiptSha256 };
    const sidecarArtifact = { byte_length: sidecarFile.bytes.length, media_type: 'text/plain', sha256: sha256(sidecarFile.bytes) };
    return { originInventories, receipt, receiptArtifact, sidecarArtifact };
}

export async function validateCloudflarePublicationMaterialization({ root, expectedReceiptSha256, capability }) {
    const owned = capability === undefined;
    const retained = capability ?? await openUnaliasedDirectory(root, 'Cloudflare Publication materialization');
    try {
        const materializationRoot = retained.capPath;
        const tree = await physicalTree(materializationRoot, 'Cloudflare Publication materialization');
        const parsed = parseMaterializationDocuments(tree, expectedReceiptSha256);
        const finalInventory = materializationFinalInventory(parsed.receipt, parsed.receiptArtifact, parsed.sidecarArtifact);
        verifyPhysicalTree(tree, finalInventory, 'Cloudflare Publication materialization');
        return {
            ...parsed,
            receiptSha256: expectedReceiptSha256,
            root: materializationRoot,
            capability: retained,
        };
    } finally {
        if (owned) await retained.handle.close();
    }
}

async function validateCarriedMaterialization({ authorityRoot, expectedReceiptSha256, mappedOrigins }) {
    const root = await requireRealDirectory(authorityRoot, 'carried materialization authority');
    const authorityTree = await physicalTree(root, 'carried materialization authority');
    const parsed = parseMaterializationDocuments(authorityTree, expectedReceiptSha256);
    const authorityInventory = materializationAuthorityInventory(
        parsed.receipt, parsed.originInventories, parsed.receiptArtifact, parsed.sidecarArtifact,
    );
    verifyPhysicalTree(authorityTree, authorityInventory, 'carried materialization authority');
    for (const origin of MATERIALIZATION_ORIGIN_ORDER) {
        const mappedRoot = await requireRealDirectory(mappedOrigins[origin], `mapped ${origin} origin`);
        const tree = await physicalTree(mappedRoot, `mapped ${origin} origin`);
        verifyPhysicalTree(tree, parsed.originInventories[origin], `mapped ${origin} origin`);
    }
    return { ...parsed, receiptSha256: expectedReceiptSha256, root };
}

async function requireAbsent(path, label) {
    if (await optionalLstat(path) !== undefined) throw new Error(`${label} must be an absent path: ${path}`);
}
async function requireRealDirectory(path, label) {
    const requested = resolve(path);
    const isCapability = /^\/proc\/self\/fd\/[0-9]+(?:\/|$)/u.test(requested);
    const capabilityRoot = /^\/proc\/self\/fd\/[0-9]+$/u.test(requested) ? `${requested}/.` : requested;
    const facts = await optionalLstat(capabilityRoot);
    if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) throw new Error(`${label} must be a real directory: ${requested}`);
    return isCapability ? capabilityRoot : realpath(requested);
}

async function openUnaliasedDirectory(path, label) {
    const requested = resolve(path);
    const canonicalPath = await requireRealDirectory(requested, label);
    if (canonicalPath !== requested) throw new Error(`${label} must not use a symlinked ancestor: ${requested}`);
    const before = await lstat(requested, { bigint: true });
    const handle = await open(requested, 'r');
    const opened = await handle.stat({ bigint: true });
    if (!opened.isDirectory() || inodeIdentity(before) !== inodeIdentity(opened)) {
        await handle.close();
        throw new Error(`${label} changed while it was pinned`);
    }
    return {
        capPath: `/proc/self/fd/${handle.fd}/.`,
        handle,
        identity: inodeIdentity(opened),
        path: requested,
    };
}

async function requireRetainedDirectoryPath(capability, label) {
    const current = await optionalLstat(capability.path, { bigint: true });
    if (!sameInode(current, capability.identity)) throw new Error(`${label} path no longer names its retained directory`);
}

async function openDirectoryEntry(parent, name, label) {
    if (basename(name) !== name || name === '.' || name === '..') throw new Error(`${label} has an invalid entry name`);
    const path = resolve(parent.capPath, name);
    const before = await lstat(path, { bigint: true });
    if (!before.isDirectory() || before.isSymbolicLink()) throw new Error(`${label} is not a real directory entry`);
    const handle = await open(path, 'r');
    const opened = await handle.stat({ bigint: true });
    if (!opened.isDirectory() || inodeIdentity(opened) !== inodeIdentity(before)) {
        await handle.close();
        throw new Error(`${label} changed while it was pinned`);
    }
    return {
        capPath: `/proc/self/fd/${handle.fd}/.`,
        handle,
        identity: inodeIdentity(opened),
        name,
        parent,
        path,
    };
}

async function clearOwnedDirectory(handle, label, budget = { entries: 0 }, depth = 0, logicalDirectory = '.') {
    if (depth > MAX_TREE_DEPTH) throw new Error(`${label} cleanup exceeds maximum directory depth ${MAX_TREE_DEPTH}`);
    await handle.chmod(0o700);
    const pinned = `/proc/self/fd/${handle.fd}`;
    const entries = [];
    const directory = await opendir(`${pinned}/.`);
    for await (const item of directory) {
        const entry = item.name;
        budget.entries += 1;
        if (budget.entries > MAX_TREE_ENTRIES) throw new Error(`${label} cleanup exceeds maximum entry count ${MAX_TREE_ENTRIES}`);
        if (Buffer.byteLength(entry, 'utf8') > MAX_TREE_NAME_BYTES) {
            throw new Error(`${label} cleanup contains an overlong entry name`);
        }
        const logical = logicalDirectory === '.' ? entry : `${logicalDirectory}/${entry}`;
        if (Buffer.byteLength(logical, 'utf8') > MAX_TREE_PATH_BYTES) {
            throw new Error(`${label} cleanup contains an overlong relative path`);
        }
        entries.push(entry);
    }
    for (const entry of entries) {
        const logical = logicalDirectory === '.' ? entry : `${logicalDirectory}/${entry}`;
        const child = resolve(pinned, entry);
        const facts = await lstat(child, { bigint: true });
        if (facts.isDirectory() && !facts.isSymbolicLink()) {
            const childHandle = await open(child, 'r');
            try {
                const opened = await childHandle.stat({ bigint: true });
                if (!opened.isDirectory() || inodeIdentity(opened) !== inodeIdentity(facts)) {
                    throw new Error(`${label} child changed before cleanup: ${entry}`);
                }
                await clearOwnedDirectory(childHandle, label, budget, depth + 1, logical);
                const retained = await childHandle.stat({ bigint: true });
                if (inodeIdentity(retained) !== inodeIdentity(facts)) throw new Error(`${label} child changed during cleanup: ${entry}`);
            } finally {
                await childHandle.close();
            }
            const current = await lstat(child, { bigint: true });
            if (inodeIdentity(current) !== inodeIdentity(facts)) throw new Error(`${label} child path was substituted during cleanup: ${entry}`);
            await rmdir(child);
        } else {
            await rm(child, { force: false });
        }
    }
}

async function removeOwnedStage(stage) {
    const facts = await optionalLstat(stage.path, { bigint: true });
    if (facts === undefined) return;
    if (!facts.isDirectory() || facts.isSymbolicLink() || inodeIdentity(facts) !== stage.identity) {
        throw new Error(`refusing to clean substituted operator-owned stage: ${stage.path}`);
    }
    const opened = await stage.handle.stat({ bigint: true });
    if (inodeIdentity(opened) !== stage.identity) throw new Error(`operator-owned stage changed while cleanup was pinned: ${stage.path}`);
    const tombstoneName = `.operator-cleanup-${randomBytes(16).toString('hex')}`;
    const tombstonePath = resolve(stage.parent.capPath, tombstoneName);
    let moveError;
    try {
        await moveNoreplaceWithParentFd(stage.parent.handle, stage.name, tombstoneName);
    } catch (error) {
        moveError = error;
    }
    const sourceAfter = await optionalLstat(stage.path, { bigint: true });
    const tombstone = await optionalLstat(tombstonePath, { bigint: true });
    if (sourceAfter !== undefined || !sameInode(tombstone, stage.identity)) {
        throw new Error('operator-owned stage could not be moved to an exact cleanup tombstone', { cause: moveError });
    }
    await clearOwnedDirectory(stage.handle, 'operator-owned stage');
    const retained = await lstat(tombstonePath, { bigint: true });
    if (inodeIdentity(retained) !== stage.identity) throw new Error(`operator-owned stage tombstone changed before final cleanup: ${tombstonePath}`);
    await rmdir(tombstonePath);
    if (await optionalLstat(tombstonePath) !== undefined) throw new Error(`operator-owned stage tombstone remained after cleanup: ${tombstonePath}`);
}

async function moveNoreplaceWithParentFd(parentHandle, sourceName, destinationName) {
    await new Promise((resolvePromise, rejectPromise) => {
        const child = spawn(NOREPLACE_MV, [
            '--no-copy', '--no-clobber', '--no-target-directory',
            `/proc/self/fd/3/${sourceName}`, `/proc/self/fd/3/${destinationName}`,
        ], { stdio: ['ignore', 'pipe', 'pipe', parentHandle.fd] });
        const stderr = [];
        let stderrBytes = 0;
        child.stderr.on('data', chunk => {
            stderrBytes += chunk.length;
            if (stderrBytes <= 1024 * 1024) stderr.push(chunk);
        });
        const timer = setTimeout(() => child.kill('SIGKILL'), 30_000);
        child.once('error', error => {
            clearTimeout(timer);
            rejectPromise(error);
        });
        child.once('close', (code, signal) => {
            clearTimeout(timer);
            if (code === 0) resolvePromise();
            else rejectPromise(new Error(`atomic mv failed (${signal ?? code}): ${Buffer.concat(stderr).toString('utf8').trim()}`));
        });
    });
}

export async function atomicInstallNoreplace(source, destination, {
    moveImpl = moveNoreplaceWithParentFd,
    parentCapability,
} = {}) {
    const sourcePath = resolve(source);
    const destinationPath = resolve(destination);
    if (dirname(sourcePath) !== dirname(destinationPath)) throw new Error('atomic installation requires sibling source and destination');
    const ownedParent = parentCapability === undefined;
    const parent = parentCapability ?? await openUnaliasedDirectory(dirname(sourcePath), 'atomic installation parent');
    try {
        await requireRetainedDirectoryPath(parent, 'atomic installation parent');
        const pinnedParent = parent.capPath;
        const sourceName = basename(sourcePath);
        const destinationName = basename(destinationPath);
        const pinnedSource = resolve(pinnedParent, sourceName);
        const pinnedDestination = resolve(pinnedParent, destinationName);
        const sourceFacts = await lstat(pinnedSource, { bigint: true });
        if (!sourceFacts.isDirectory() || sourceFacts.isSymbolicLink()) throw new Error('atomic installation source is not a real directory');
        const expectedIdentity = inodeIdentity(sourceFacts);
        let commandError;
        try {
            await moveImpl(parent.handle, sourceName, destinationName);
        } catch (error) {
            commandError = error;
        }
        const sourceAfter = await optionalLstat(pinnedSource, { bigint: true });
        const destinationAfter = await optionalLstat(pinnedDestination, { bigint: true });
        if (sourceAfter === undefined && sameInode(destinationAfter, expectedIdentity)) {
            return { commandError, identity: expectedIdentity };
        }
        if (sameInode(sourceAfter, expectedIdentity) && !sameInode(destinationAfter, expectedIdentity)) {
            if (commandError !== undefined) throw commandError;
            throw new Error(`operator deployment bundle destination appeared during atomic installation: ${destinationPath}`);
        }
        throw new OperatorBundleInstallStateUncertain(destinationPath, commandError);
    } finally {
        if (ownedParent) await parent.handle.close();
    }
}

async function syncParentDirectory(parent) {
    if (typeof parent !== 'string') {
        await parent.sync();
        return;
    }
    const handle = await open(parent, 'r');
    try { await handle.sync(); } finally { await handle.close(); }
}
async function syncTree(path) {
    const tree = await physicalTree(path, 'tree synchronization input');
    const entries = [
        ...[...tree.files.entries()].map(([logical, file]) => [logical, file.facts]),
        ...[...tree.directories.entries()].sort(([left], [right]) => right.split('/').length - left.split('/').length),
    ];
    for (const [logical, facts] of entries) {
        const target = logical === '.' ? path : resolve(path, logical);
        const handle = await open(target, 'r');
        try {
            const opened = await handle.stat({ bigint: true });
            if (stableNodeIdentity(opened) !== stableNodeIdentity(facts)) {
                throw new Error(`synchronization target changed while it was opened: ${target}`);
            }
            await handle.sync();
            const retained = await handle.stat({ bigint: true });
            if (stableNodeIdentity(retained) !== stableNodeIdentity(opened)) {
                throw new Error(`synchronization target changed while it was synchronized: ${target}`);
            }
        } finally { await handle.close(); }
    }
}
async function verifyOrigin(origin, root, inventoryPath, expectedInventorySha256, repoRoot, datadirAuthorityPath) {
    const verified = await verifyStaticOriginInventory({ origin, root, inventoryPath, expectedInventorySha256 });
    if (origin === 'public') await verifyPublicBuild(root);
    else if (origin === 'identity_signer') await verifySignerBuild(root);
    else {
        const runtime = await verifyRuntimeCorpus(root, {
            datadirAuthorityPath,
            expectedContract: await verifyRuntimeSourceContract(repoRoot),
        });
        return { ...verified, latest: runtime.latest };
    }
    return verified;
}

function runtimePhysicalInventory(inventory, headersArtifact, directoryMode, fileMode) {
    const directories = new Set(['.']);
    const files = inventory.artifacts.map(item => {
        addPathParents(directories, item.path);
        return {
            artifact: {
                byte_length: item.byte_length,
                media_type: item.media_type,
                sha256: item.sha256,
            },
            path: item.path,
            unix_mode: fileMode,
        };
    });
    files.push({ artifact: headersArtifact, path: '_headers', unix_mode: fileMode });
    files.sort((left, right) => utf8Order(left.path, right.path));
    return {
        directories: [...directories].sort(utf8Order).map(path => ({ path, unix_mode: directoryMode })),
        files,
    };
}

function validateRuntimeInventoryDocument(inventory, label) {
    exactKeys(inventory, ['artifacts', 'cargo_lock_sha256', 'origin', 'schema_version', 'source_commit'], label);
    if (inventory.schema_version !== 1 || inventory.origin !== 'runtime'
        || !COMMIT.test(inventory.source_commit) || !DIGEST.test(inventory.cargo_lock_sha256)
        || !Array.isArray(inventory.artifacts) || inventory.artifacts.length < 1
        || inventory.artifacts.length > MAX_TREE_ENTRIES) {
        throw new Error(`${label} has invalid fixed authority`);
    }
    let previous;
    for (const [index, artifact] of inventory.artifacts.entries()) {
        exactKeys(artifact, ['byte_length', 'media_type', 'path', 'sha256'], `${label}.artifacts[${index}]`);
        validateRelativePath(artifact.path, `${label}.artifacts[${index}].path`);
        if (artifact.path === '_headers' || !Number.isSafeInteger(artifact.byte_length)
            || artifact.byte_length < 1 || artifact.byte_length > MAX_REGULAR_FILE_BYTES
            || !DIGEST.test(artifact.sha256) || typeof artifact.media_type !== 'string'
            || artifact.media_type.length < 1 || artifact.media_type.length > 128
            || /[\u0000-\u001f\u007f]/u.test(artifact.media_type)
            || (previous !== undefined && utf8Order(previous, artifact.path) >= 0)) {
            throw new Error(`${label} has invalid or unsorted artifact authority at ${artifact.path}`);
        }
        previous = artifact.path;
    }
}

async function validateBoundedRuntimeOrigin({
    root, inventoryPath, expectedInventorySha256, directoryMode, fileMode, label,
}) {
    if (!DIGEST.test(expectedInventorySha256)) throw new Error(`${label} approved inventory SHA-256 is invalid`);
    const inventoryFile = await stableRegularFile(inventoryPath, `${label} inventory`);
    if (sha256(inventoryFile.bytes) !== expectedInventorySha256) {
        throw new Error(`${label} inventory differs from its independently approved digest`);
    }
    const inventory = parseCanonicalBytes(inventoryFile.bytes, `${label} inventory`);
    validateRuntimeInventoryDocument(inventory, `${label} inventory`);
    const headers = await stableRegularFile(resolve(root, '_headers'), `${label} headers`);
    const tree = await physicalTree(root, label);
    verifyPhysicalTree(tree, runtimePhysicalInventory(
        inventory,
        { byte_length: headers.bytes.length, media_type: 'text/plain', sha256: sha256(headers.bytes) },
        directoryMode,
        fileMode,
    ), label);
    return { identity: physicalTreeIdentity(tree), inventory, inventoryBytes: inventoryFile.bytes, tree };
}

function validateExactRuntimeHandoffTree(handoffTree, runtimeTree, inventoryFile, label) {
    const expectedDirectories = new Set(['.', 'inventories', 'runtime']);
    for (const path of runtimeTree.directories.keys()) {
        if (path !== '.') expectedDirectories.add(`runtime/${path}`);
    }
    const expectedFiles = new Set(['inventories/runtime.json']);
    for (const path of runtimeTree.files.keys()) expectedFiles.add(`runtime/${path}`);
    if (!sameJson([...handoffTree.directories.keys()], [...expectedDirectories].sort(utf8Order))
        || !sameJson([...handoffTree.files.keys()], [...expectedFiles].sort(utf8Order))) {
        throw new Error(`${label} must contain exactly runtime/ and inventories/runtime.json`);
    }
    for (const [path, facts] of handoffTree.directories) {
        if (Number(facts.mode & 0o7777n) !== 0o550) throw new Error(`${label} directory must be sealed mode 0550: ${path}`);
    }
    for (const [path, file] of handoffTree.files) {
        if (Number(file.facts.mode & 0o7777n) !== 0o440) throw new Error(`${label} file must be sealed mode 0440: ${path}`);
    }
    const carriedInventory = handoffTree.files.get('inventories/runtime.json');
    if (!carriedInventory.bytes.equals(inventoryFile)) throw new Error(`${label} runtime inventory changed during exact closure proof`);
}

function physicalTreeIdentity(tree) {
    return canonical({
        directories: [...tree.directories].map(([path, facts]) => [path, stableNodeIdentity(facts)]),
        files: [...tree.files].map(([path, file]) => [path, stableNodeIdentity(file.facts)]),
    });
}

function assertSameTreeIdentity(before, after, label) {
    if (!sameJson(physicalTreeIdentity(before), physicalTreeIdentity(after))) {
        throw new Error(`${label} changed while it was copied`);
    }
}

function sortedChildDirectories(tree) {
    return [...tree.directories.keys()].filter(path => path !== '.').sort((left, right) => {
        const depth = left.split('/').length - right.split('/').length;
        return depth === 0 ? utf8Order(left, right) : depth;
    });
}

export async function copyBoundedTree(source, destination, { label = 'bounded tree copy' } = {}) {
    const snapshot = await physicalTree(source, `${label} source`);
    await requireAbsent(destination, `${label} destination`);
    await mkdir(destination, { mode: 0o700 });
    const output = await openUnaliasedDirectory(destination, `${label} destination`);
    try {
        for (const path of sortedChildDirectories(snapshot)) {
            await mkdir(resolve(output.capPath, path), { mode: 0o700 });
        }
        for (const [path, file] of snapshot.files) {
            await writeFile(resolve(output.capPath, path), file.bytes, { flag: 'wx', mode: 0o600 });
        }
        const copied = await physicalTree(output.capPath, `${label} destination`);
        if (!sameJson([...snapshot.directories.keys()], [...copied.directories.keys()])
            || !sameJson([...snapshot.files.keys()], [...copied.files.keys()])) {
            throw new Error(`${label} destination path closure differs from its retained source`);
        }
        for (const [path, file] of snapshot.files) {
            if (!file.bytes.equals(copied.files.get(path).bytes)) {
                throw new Error(`${label} destination differs from its retained source at ${path}`);
            }
        }
        assertSameTreeIdentity(snapshot, await physicalTree(source, `${label} retained source`), label);
    } finally {
        await output.handle.close();
    }
}

async function copyBoundedFile(source, destination, label) {
    const admitted = await stableRegularFile(source, `${label} source`);
    await writeFile(destination, admitted.bytes, { flag: 'wx', mode: 0o600 });
    const copied = await stableRegularFile(destination, `${label} destination`);
    const retained = await stableRegularFile(source, `${label} retained source`);
    if (!copied.bytes.equals(admitted.bytes)
        || stableNodeIdentity(retained.facts) !== stableNodeIdentity(admitted.facts)) {
        throw new Error(`${label} source changed while it was copied`);
    }
}

async function openApprovedRuntimeCapability(root) {
    const handoff = await openUnaliasedDirectory(root, 'approved WASM/static handoff');
    try {
        const runtime = await openDirectoryEntry(handoff, 'runtime', 'approved runtime origin');
        try {
            const inventories = await openDirectoryEntry(handoff, 'inventories', 'approved runtime inventories');
            return { handoff, inventories, runtime };
        } catch (error) {
            await runtime.handle.close();
            throw error;
        }
    } catch (error) {
        await handoff.handle.close();
        throw error;
    }
}

async function closeApprovedRuntimeCapability(capability) {
    await capability.inventories.handle.close();
    await capability.runtime.handle.close();
    await capability.handoff.handle.close();
}

async function validateApprovedRuntimeHandoff({
    datadirAuthorityPath, expectedInventorySha256, repoRoot, root,
    sourceCommit, cargoLockSha256, verifyOriginImpl, capability,
}) {
    if (!DIGEST.test(expectedInventorySha256)) throw new Error('approved runtime inventory SHA-256 is invalid');
    let retained = capability;
    if (retained === undefined) {
        retained = await openApprovedRuntimeCapability(root);
    }
    await requireRetainedDirectoryPath(retained.handoff, 'approved WASM/static handoff');
    const handoffRoot = retained.handoff.capPath;
    const runtimeRoot = retained.runtime.capPath;
    const inventoriesRoot = retained.inventories.capPath;
    for (const [directory, label] of [
        [retained.handoff, 'approved WASM/static handoff'],
        [retained.runtime, 'approved runtime origin'],
        [retained.inventories, 'approved runtime inventories'],
    ]) {
        const facts = await directory.handle.stat({ bigint: true });
        if (Number(facts.mode & 0o7777n) !== 0o550) throw new Error(`${label} must be sealed mode 0550`);
    }
    const inventoryPath = resolve(inventoriesRoot, 'runtime.json');
    const bounded = await validateBoundedRuntimeOrigin({
        directoryMode: 0o550,
        expectedInventorySha256,
        fileMode: 0o440,
        inventoryPath,
        label: 'approved runtime origin',
        root: runtimeRoot,
    });
    const handoffTree = await physicalTree(handoffRoot, 'approved WASM/static handoff');
    validateExactRuntimeHandoffTree(
        handoffTree,
        bounded.tree,
        bounded.inventoryBytes,
        'approved WASM/static handoff',
    );
    const verified = await verifyOriginImpl(
        'runtime', runtimeRoot, inventoryPath, expectedInventorySha256,
        repoRoot, datadirAuthorityPath,
    );
    if (verified.inventory.source_commit !== sourceCommit
        || verified.inventory.cargo_lock_sha256 !== cargoLockSha256
        || verified.latest?.commit !== sourceCommit) {
        throw new Error('approved runtime inventory/latest source authority differs from the materialization receipt');
    }
    if (!sameJson(verified.inventory, bounded.inventory)) {
        throw new Error('legacy runtime verifier differs from the bounded approved runtime authority');
    }
    return {
        identity: canonical({
            handoff: physicalTreeIdentity(handoffTree),
            runtime: bounded.identity,
        }),
        inventory: verified.inventory,
        inventoryBytes: bounded.inventoryBytes,
        inventoryPath,
        capability: retained,
        root: handoffRoot,
        runtimeRoot,
    };
}

function validateDemo(demo) {
    exactKeys(demo, [
        'content_manifest_sha256', 'content_manifest_url', 'datadir_byte_length',
        'datadir_sha256', 'datadir_url', 'native_content_sha256',
    ], 'datadir deployment Demo authority');
    for (const field of ['content_manifest_sha256', 'datadir_sha256', 'native_content_sha256']) {
        if (!DIGEST.test(demo[field])) throw new Error(`datadir deployment ${field} is invalid`);
    }
    if (!Number.isSafeInteger(demo.datadir_byte_length) || demo.datadir_byte_length < 1) throw new Error('datadir deployment byte length is invalid');
    for (const field of ['content_manifest_url', 'datadir_url']) {
        if (typeof demo[field] !== 'string' || !demo[field].startsWith(`https://${PUBLIC_HOST}/datadirs/`)) {
            throw new Error(`datadir deployment ${field} is outside the exact public datadir origin`);
        }
    }
}

function validateDatadirReceipt(receipt, bytes) {
    exactKeys(receipt, [
        'authority_sha256', 'demo', 'inventory_sha256',
        'public_root_url', 'route_pattern', 'schema_version', 'source_commit',
        'worker_name', 'worker_version_id',
    ], 'datadir deployment receipt');
    if (receipt.schema_version !== 1 || !COMMIT.test(receipt.source_commit)
        || !DIGEST.test(receipt.inventory_sha256)
        || !DIGEST.test(receipt.authority_sha256) || !VERSION.test(receipt.worker_version_id)
        || receipt.worker_name !== 'robinhood-datadir-assets'
        || receipt.route_pattern !== `${PUBLIC_HOST}/datadirs/*`
        || receipt.public_root_url !== `https://${PUBLIC_HOST}/datadirs/`) {
        throw new Error('datadir deployment receipt has invalid fixed authority');
    }
    validateDemo(receipt.demo);
    if (!bytes.equals(canonicalBytes(receipt, false))) throw new Error('datadir deployment receipt is not canonical compact JSON');
    return canonical({
        authority_sha256: receipt.authority_sha256,
        deployment_receipt_sha256: sha256(bytes),
        inventory_sha256: receipt.inventory_sha256,
        public_root_url: receipt.public_root_url,
        route_pattern: receipt.route_pattern,
        worker_name: receipt.worker_name,
        worker_version_id: receipt.worker_version_id,
    });
}

function validateDatadirAuthority(authority, bytes, receipt) {
    exactKeys(authority, [
        'cargo_lock_sha256', 'demo', 'inventory_sha256', 'public_root_url',
        'route_pattern', 'schema_version', 'source_commit', 'worker_name',
    ], 'datadir release authority');
    if (!bytes.equals(canonicalBytes(authority, false))) throw new Error('datadir release authority is not canonical compact JSON');
    if (authority.schema_version !== 1 || !COMMIT.test(authority.source_commit)
        || !DIGEST.test(authority.cargo_lock_sha256) || !DIGEST.test(authority.inventory_sha256)
        || authority.worker_name !== 'robinhood-datadir-assets'
        || authority.route_pattern !== `${PUBLIC_HOST}/datadirs/*`
        || authority.public_root_url !== `https://${PUBLIC_HOST}/datadirs/`) {
        throw new Error('datadir release authority has invalid fixed authority');
    }
    validateDemo(authority.demo);
    const authorityReceiptProjection = canonical({
        demo: receipt.demo,
        inventory_sha256: receipt.inventory_sha256,
        public_root_url: receipt.public_root_url,
        route_pattern: receipt.route_pattern,
        schema_version: receipt.schema_version,
        source_commit: receipt.source_commit,
        worker_name: receipt.worker_name,
    });
    const expectedProjection = canonical({
        demo: authority.demo,
        inventory_sha256: authority.inventory_sha256,
        public_root_url: authority.public_root_url,
        route_pattern: authority.route_pattern,
        schema_version: authority.schema_version,
        source_commit: authority.source_commit,
        worker_name: authority.worker_name,
    });
    if (JSON.stringify(authorityReceiptProjection) !== JSON.stringify(expectedProjection)
        || sha256(bytes) !== receipt.authority_sha256) {
        throw new Error('datadir release authority differs from its installed deployment receipt');
    }
}

function validateDeploymentExposure(exposure, routes) {
    exactKeys(exposure, [
        'backend_api_manifest_root', 'backend_api_route', 'cloudflare_routes',
        'cloudflare_zone', 'identity_signer_origin', 'identity_signer_static_root',
        'operator_private_paths', 'public_origin', 'public_static_root', 'schema_version',
    ], 'materialized deployment exposure');
    if (exposure.schema_version !== 3 || exposure.public_origin !== `https://${PUBLIC_HOST}`
        || exposure.identity_signer_origin !== `https://identity.${PUBLIC_HOST}`
        || exposure.public_static_root !== 'cloudflare-public'
        || exposure.identity_signer_static_root !== 'cloudflare-identity-signer'
        || exposure.backend_api_manifest_root !== 'backend/manifests'
        || exposure.backend_api_route !== '/api*' || exposure.cloudflare_zone !== 'phiresky.xyz'
        || canonicalBytes(exposure.cloudflare_routes, false).compare(canonicalBytes(routes, false)) !== 0) {
        throw new Error('materialized deployment exposure differs from the exact operator routes');
    }
    const privatePaths = [
        'backend/publication-v3.json', 'deployment', 'private', 'publication-lock-v3.json',
        'publication-lock-v3.sha256', 'publication-manifest-v3.json', 'publication-manifest-v3.sha256',
    ];
    if (canonicalBytes(exposure.operator_private_paths, false).compare(canonicalBytes(privatePaths, false)) !== 0) {
        throw new Error('materialized deployment exposure has invalid private-path authority');
    }
}

function carriedMaterializationPath(path) { return `${MATERIALIZATION_AUTHORITY_DIRECTORY}/${path}`; }

function validateArtifactBinding(binding, expectedPath, label) {
    exactKeys(binding, ['artifact', 'path'], label);
    if (binding.path !== expectedPath) throw new Error(`${label} uses an invalid authority path`);
    validateArtifactRef(binding.artifact, `${label}.artifact`);
}

function validateManifestShape(manifest) {
    exactKeys(manifest, [
        'cargo_lock_sha256', 'datadir', 'materialization', 'origins',
        'publication_lock_sha256', 'publication_manifest_sha256',
        'publication_schema_version', 'routes_sha256', 'runtime', 'schema_version',
        'source_commit', 'source_tree_sha1',
    ], 'operator deployment manifest');
    if (manifest.schema_version !== 2 || manifest.publication_schema_version !== 3
        || !COMMIT.test(manifest.source_commit) || !COMMIT.test(manifest.source_tree_sha1)) {
        throw new Error('operator deployment V2 schema/source authority is invalid');
    }
    for (const field of [
        'cargo_lock_sha256', 'publication_lock_sha256', 'publication_manifest_sha256', 'routes_sha256',
    ]) {
        if (!DIGEST.test(manifest[field])) throw new Error(`operator deployment ${field} is invalid`);
    }
    exactKeys(manifest.datadir, [
        'authority_sha256', 'deployment_receipt_sha256', 'inventory_sha256',
        'public_root_url', 'route_pattern', 'worker_name', 'worker_version_id',
    ], 'operator deployment datadir authority');
    if (![manifest.datadir.authority_sha256, manifest.datadir.deployment_receipt_sha256, manifest.datadir.inventory_sha256]
        .every(value => DIGEST.test(value)) || !VERSION.test(manifest.datadir.worker_version_id)
        || manifest.datadir.worker_name !== 'robinhood-datadir-assets'
        || manifest.datadir.route_pattern !== `${PUBLIC_HOST}/datadirs/*`
        || manifest.datadir.public_root_url !== `https://${PUBLIC_HOST}/datadirs/`) {
        throw new Error('operator deployment datadir authority is invalid');
    }
    exactKeys(manifest.origins, Object.keys(ORIGINS), 'operator deployment origins');
    for (const [origin, expected] of Object.entries(ORIGINS)) {
        const item = manifest.origins[origin];
        exactKeys(item, ['directory', 'inventory', 'inventory_sha256'], `${origin} origin`);
        if (item.directory !== expected.bundleDirectory || item.inventory !== expected.inventory || !DIGEST.test(item.inventory_sha256)) {
            throw new Error(`${origin} origin uses invalid bundle authority`);
        }
    }
    exactKeys(manifest.runtime, ['directory', 'inventory', 'inventory_sha256'], 'runtime origin');
    if (manifest.runtime.directory !== RUNTIME.bundleDirectory || manifest.runtime.inventory !== RUNTIME.inventory
        || !DIGEST.test(manifest.runtime.inventory_sha256)) throw new Error('runtime origin uses invalid bundle authority');
    exactKeys(manifest.materialization, [
        'approved_receipt_sha256', 'origin_inventories', 'receipt', 'sidecar',
    ], 'operator deployment materialization');
    if (!DIGEST.test(manifest.materialization.approved_receipt_sha256)) {
        throw new Error('operator deployment materialization digest is invalid');
    }
    validateArtifactBinding(
        manifest.materialization.receipt,
        carriedMaterializationPath(MATERIALIZATION_RECEIPT),
        'operator deployment materialization receipt',
    );
    validateArtifactBinding(
        manifest.materialization.sidecar,
        carriedMaterializationPath(MATERIALIZATION_SIDECAR),
        'operator deployment materialization sidecar',
    );
    exactKeys(
        manifest.materialization.origin_inventories,
        MATERIALIZATION_ORIGIN_ORDER,
        'operator deployment materialization origin inventories',
    );
    for (const origin of MATERIALIZATION_ORIGIN_ORDER) {
        validateArtifactBinding(
            manifest.materialization.origin_inventories[origin],
            carriedMaterializationPath(MATERIALIZATION_ORIGINS[origin].inventoryPath),
            `operator deployment ${origin} materialization inventory`,
        );
    }
}
function hardenedGitEnvironment(ambientEnvironment) {
    const environment = { ...ambientEnvironment };
    for (const key of Object.keys(environment)) {
        if (key.startsWith('GIT_')) delete environment[key];
    }
    environment.GIT_CONFIG_GLOBAL = '/dev/null';
    environment.GIT_CONFIG_NOSYSTEM = '1';
    environment.GIT_OPTIONAL_LOCKS = '0';
    environment.LANG = 'C';
    environment.LC_ALL = 'C';
    return environment;
}

function hardenedGitArguments(repoRoot, args) {
    return [
        '--no-replace-objects',
        `--work-tree=${repoRoot}`,
        '-c', 'core.fsmonitor=false',
        '-c', 'core.fileMode=true',
        '-c', 'core.hooksPath=/dev/null',
        '-c', 'core.ignoreCase=false',
        '-c', 'core.symlinks=true',
        '-c', 'core.untrackedCache=false',
        ...args,
    ];
}

async function exactGitRevision({ environment, execFileImpl, repoRoot, revision }) {
    const result = await execFileImpl('/usr/bin/git', hardenedGitArguments(repoRoot, [
        'rev-parse', '--verify', '--end-of-options', revision,
    ]), { cwd: repoRoot, encoding: 'utf8', env: environment, maxBuffer: 1024 * 1024, timeout: 30_000 });
    if (!/^[0-9a-f]{40}\n$/u.test(result.stdout) || result.stderr !== '') {
        throw new Error(`operator checkout git returned ambiguous authority for ${revision}`);
    }
    return result.stdout.slice(0, -1);
}

async function exactGitBlob({ environment, execFileImpl, repoRoot, revision }) {
    const result = await execFileImpl('/usr/bin/git', hardenedGitArguments(repoRoot, [
        'cat-file', 'blob', revision,
    ]), { cwd: repoRoot, encoding: null, env: environment, maxBuffer: MAX_REGULAR_FILE_BYTES, timeout: 30_000 });
    const stdout = Buffer.isBuffer(result.stdout) ? result.stdout : Buffer.from(result.stdout);
    const stderr = Buffer.isBuffer(result.stderr) ? result.stderr : Buffer.from(result.stderr ?? '');
    if (stdout.length < 1 || stdout.length > MAX_REGULAR_FILE_BYTES || stderr.length !== 0) {
        throw new Error(`operator checkout git returned ambiguous tracked blob authority for ${revision}`);
    }
    return stdout;
}

export async function readExactOperatorCheckout(
    repoRoot,
    execFileImpl = execFile,
    ambientEnvironment = process.env,
) {
    const environment = hardenedGitEnvironment(ambientEnvironment);
    const root = await openUnaliasedDirectory(repoRoot, 'operator tooling checkout');
    try {
        const pinnedRoot = root.capPath;
        const childPinnedRoot = `/proc/${process.pid}/fd/${root.handle.fd}/.`;
        const sourceCommit = await exactGitRevision({
            environment, execFileImpl, repoRoot: childPinnedRoot, revision: 'HEAD^{commit}',
        });
        const sourceTreeSha1 = await exactGitRevision({
            environment, execFileImpl, repoRoot: childPinnedRoot, revision: `${sourceCommit}^{tree}`,
        });
        try {
            await execFileImpl('/usr/bin/git', hardenedGitArguments(childPinnedRoot, [
                'diff', '--quiet', '--no-ext-diff', '--no-textconv', '--ignore-submodules=none',
                sourceCommit, '--',
            ]), { cwd: childPinnedRoot, encoding: 'utf8', env: environment, maxBuffer: 1024 * 1024, timeout: 30_000 });
        } catch (error) {
            throw new Error('operator tooling checkout has tracked changes or could not be proven clean', { cause: error });
        }
        const trackedCargoLock = await exactGitBlob({
            environment,
            execFileImpl,
            repoRoot: childPinnedRoot,
            revision: `${sourceCommit}:Cargo.lock`,
        });
        const cargoLock = await stableRegularFile(resolve(pinnedRoot, 'Cargo.lock'), 'operator tooling Cargo.lock');
        if (!trackedCargoLock.equals(cargoLock.bytes)) {
            throw new Error('operator tooling Cargo.lock differs from the exact commit');
        }
        const deploymentConfigSha256 = {};
        for (const file of DEPLOYMENT_CONFIG_FILES) {
            const tracked = await exactGitBlob({
                environment,
                execFileImpl,
                repoRoot: childPinnedRoot,
                revision: `${sourceCommit}:wasm-www/deploy/${file}`,
            });
            const local = await stableRegularFile(
                resolve(pinnedRoot, 'wasm-www/deploy', file),
                `operator tracked deployment config ${file}`,
            );
            if (!tracked.equals(local.bytes)) throw new Error(`operator deployment config differs from exact commit: ${file}`);
            deploymentConfigSha256[file] = sha256(tracked);
        }
        await requireRetainedDirectoryPath(root, 'operator tooling checkout');
        return { cargoLockSha256: sha256(trackedCargoLock), deploymentConfigSha256, sourceCommit, sourceTreeSha1 };
    } finally {
        await root.handle.close();
    }
}
export async function assembleOperatorDeploymentBundle({
    materializationRoot, runtimeRoot, expectedMaterializationReceiptSha256,
    expectedRuntimeInventorySha256, routesPath,
    output, repoRoot,
    verifyOriginImpl = verifyOrigin, verifyPublicBuildImpl = verifyPublicBuild,
    verifySignerBuildImpl = verifySignerBuild, checkoutImpl = readExactOperatorCheckout,
    processNodeVersion = process.version,
    installImpl = atomicInstallNoreplace, syncParentImpl = syncParentDirectory,
    syncTreeImpl = syncTree, copyImpl = copyBoundedTree, copyFileImpl = copyBoundedFile,
}) {
    requireExactNodeVersion(processNodeVersion);
    const materializationCapability = await openUnaliasedDirectory(
        materializationRoot,
        'Cloudflare Publication materialization',
    );
    try {
    const materialization = await validateCloudflarePublicationMaterialization({
        capability: materializationCapability,
        expectedReceiptSha256: expectedMaterializationReceiptSha256,
        root: materializationRoot,
    });
    const checkout = await checkoutImpl(repoRoot);
    if (checkout.sourceCommit !== materialization.receipt.source_commit) {
        throw new Error('operator tooling checkout is not the exact materialization source commit');
    }
    if (checkout.sourceTreeSha1 !== materialization.receipt.source_tree_sha1) {
        throw new Error('operator tooling checkout is not the exact clean materialization source tree');
    }
    if (checkout.cargoLockSha256 !== materialization.receipt.cargo_lock_sha256) {
        throw new Error('operator tooling Cargo.lock differs from the approved materialization');
    }
    const destination = resolve(output);
    const parent = await openUnaliasedDirectory(dirname(destination), 'operator deployment bundle parent');
    let runtimeCapability;
    try {
    const pinnedDestination = resolve(parent.capPath, basename(destination));
    await requireAbsent(pinnedDestination, 'operator deployment bundle output');
    const roots = {
        deployment_authority: await requireRealDirectory(resolve(materialization.root, 'deployment'), 'materialized deployment authority origin'),
        identity_signer: await requireRealDirectory(resolve(materialization.root, ORIGINS.identity_signer.materializationDirectory), 'materialized identity signer origin'),
        public: await requireRealDirectory(resolve(materialization.root, ORIGINS.public.materializationDirectory), 'materialized public origin'),
    };
    runtimeCapability = await openApprovedRuntimeCapability(runtimeRoot);
    const runtime = await validateApprovedRuntimeHandoff({
        capability: runtimeCapability,
        cargoLockSha256: materialization.receipt.cargo_lock_sha256,
        datadirAuthorityPath: resolve(roots.deployment_authority, 'datadir-authority.json'),
        expectedInventorySha256: expectedRuntimeInventorySha256,
        repoRoot,
        root: runtimeRoot,
        sourceCommit: materialization.receipt.source_commit,
        verifyOriginImpl,
    });
    roots.runtime = runtime.runtimeRoot;
    const receipt = await parseJson(resolve(roots.runtime, 'wasm/datadir-deployment.json'), 'runtime datadir deployment receipt', true);
    const materializedReceipt = await parseJson(
        resolve(roots.deployment_authority, 'datadir-deployment.json'),
        'materialized datadir deployment receipt',
        true,
    );
    if (!receipt.bytes.equals(materializedReceipt.bytes)) {
        throw new Error('runtime datadir deployment receipt differs byte-for-byte from the approved materialization');
    }
    const datadir = validateDatadirReceipt(receipt.value, receipt.bytes);
    const materializedAuthorityPath = resolve(roots.deployment_authority, 'datadir-authority.json');
    const authority = await parseJson(materializedAuthorityPath, 'materialized datadir release authority', true);
    validateDatadirAuthority(authority.value, authority.bytes, receipt.value);
    const routesBytes = (await stableRegularFile(routesPath, 'operator route authority')).bytes;
    const routeAuthority = JSON.parse(routesBytes.toString('utf8'));
    validateOperatorRouteAuthority({
        datadirWorker: 'robinhood-datadir-assets', expectedRoutes: routeAuthority.routes,
        publicHost: PUBLIC_HOST, publicWorker: 'robinhood-public-site', runtimeWorker: 'robinhood-runtime-assets',
    });
    const exposure = await parseJson(resolve(roots.deployment_authority, 'exposure-v3.json'), 'materialized deployment exposure', true);
    validateDeploymentExposure(exposure.value, routeAuthority.routes);
    const stagingCreated = await mkdtemp(resolve(
        parent.capPath,
        `.${basename(destination)}.assembling-`,
    ));
    const staging = await openDirectoryEntry(parent, basename(stagingCreated), 'operator deployment bundle staging root');
    try {
        await mkdir(resolve(staging.capPath, 'origins'));
        await mkdir(resolve(staging.capPath, 'inventories'));
        await mkdir(resolve(staging.capPath, MATERIALIZATION_AUTHORITY_DIRECTORY), { recursive: true });
        for (const [origin, paths] of Object.entries(ORIGINS)) {
            await copyImpl(roots[origin], resolve(staging.capPath, paths.bundleDirectory), {
                label: `materialized ${origin} origin copy`,
            });
        }
        await copyImpl(roots.runtime, resolve(staging.capPath, RUNTIME.bundleDirectory), {
            label: 'approved runtime origin copy',
        });
        await mkdir(resolve(staging.capPath, MATERIALIZATION_AUTHORITY_DIRECTORY, 'inventories'));
        for (const path of [MATERIALIZATION_RECEIPT, MATERIALIZATION_SIDECAR]) {
            await copyFileImpl(
                resolve(materialization.root, path),
                resolve(staging.capPath, MATERIALIZATION_AUTHORITY_DIRECTORY, path),
                `materialization authority ${path} copy`,
            );
        }
        for (const origin of MATERIALIZATION_ORIGIN_ORDER) {
            const path = MATERIALIZATION_ORIGINS[origin].inventoryPath;
            await copyFileImpl(
                resolve(materialization.root, path),
                resolve(staging.capPath, MATERIALIZATION_AUTHORITY_DIRECTORY, path),
                `materialization authority ${path} copy`,
            );
        }
        await copyImpl(roots.deployment_authority, resolve(staging.capPath, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment'), {
            label: 'materialized deployment authority copy',
        });
        await copyFileImpl(
            runtime.inventoryPath,
            resolve(staging.capPath, RUNTIME.inventory),
            'approved runtime inventory copy',
        );
        await requireRetainedDirectoryPath(materializationCapability, 'Cloudflare Publication materialization');
        await validateCloudflarePublicationMaterialization({
            capability: materializationCapability,
            expectedReceiptSha256: expectedMaterializationReceiptSha256,
            root: materializationRoot,
        });
        const runtimeAfterCopy = await validateApprovedRuntimeHandoff({
            capability: runtimeCapability,
            cargoLockSha256: materialization.receipt.cargo_lock_sha256,
            datadirAuthorityPath: resolve(roots.deployment_authority, 'datadir-authority.json'),
            expectedInventorySha256: expectedRuntimeInventorySha256,
            repoRoot,
            root: runtimeRoot,
            sourceCommit: materialization.receipt.source_commit,
            verifyOriginImpl,
        });
        if (!sameJson(runtimeAfterCopy.identity, runtime.identity)
            || !runtimeAfterCopy.inventoryBytes.equals(runtime.inventoryBytes)) {
            throw new Error('approved runtime handoff changed while it was copied');
        }
        const originBindings = Object.fromEntries(materialization.receipt.origins.map(binding => [binding.origin, binding]));
        const bind = (path, artifact) => ({ artifact, path });
        const manifest = canonical({
            cargo_lock_sha256: materialization.receipt.cargo_lock_sha256,
            datadir,
            materialization: {
                approved_receipt_sha256: expectedMaterializationReceiptSha256,
                origin_inventories: Object.fromEntries(MATERIALIZATION_ORIGIN_ORDER.map(origin => [origin, bind(
                    carriedMaterializationPath(MATERIALIZATION_ORIGINS[origin].inventoryPath),
                    originBindings[origin].inventory,
                )])),
                receipt: bind(carriedMaterializationPath(MATERIALIZATION_RECEIPT), materialization.receiptArtifact),
                sidecar: bind(carriedMaterializationPath(MATERIALIZATION_SIDECAR), materialization.sidecarArtifact),
            },
            origins: Object.fromEntries(Object.entries(ORIGINS).map(([origin, paths]) => [origin, {
                directory: paths.bundleDirectory,
                inventory: paths.inventory,
                inventory_sha256: originBindings[origin].inventory.sha256,
            }])),
            publication_lock_sha256: materialization.receipt.publication_lock_sha256,
            publication_manifest_sha256: materialization.receipt.publication_manifest_sha256,
            publication_schema_version: materialization.receipt.publication_schema_version,
            routes_sha256: sha256(routesBytes),
            runtime: {
                directory: RUNTIME.bundleDirectory,
                inventory: RUNTIME.inventory,
                inventory_sha256: expectedRuntimeInventorySha256,
            },
            schema_version: 2,
            source_commit: materialization.receipt.source_commit,
            source_tree_sha1: materialization.receipt.source_tree_sha1,
        });
        const manifestBytes = canonicalBytes(manifest);
        await writeFile(resolve(staging.capPath, 'deployment-v2.json'), manifestBytes, { flag: 'wx' });
        await makeReadOnly(staging.capPath);
        await validateOperatorDeploymentBundle({
            bundle: staging.capPath,
            checkoutImpl,
            expectedManifestSha256: sha256(manifestBytes),
            repoRoot,
            routesPath,
            verifyOriginImpl,
            verifyPublicBuildImpl,
            verifySignerBuildImpl,
        });
        await syncTreeImpl(staging.capPath);
        await requireAbsent(pinnedDestination, 'operator deployment bundle output');
        const installOutcome = await installImpl(staging.path, pinnedDestination, { parentCapability: parent });
        const validateInstalled = async () => {
            try {
                try {
                    await requireRetainedDirectoryPath(parent, 'operator deployment bundle parent');
                } catch (error) {
                    throw new OperatorBundleInstallStateUncertain(destination, error);
                }
            } catch (error) {
                throw new OperatorBundleInstallStateUncertain(destination, error);
            }
            const destinationFacts = await optionalLstat(pinnedDestination, { bigint: true });
            if (!sameInode(destinationFacts, staging.identity)) throw new OperatorBundleInstallStateUncertain(destination);
            try {
                const result = await validateOperatorDeploymentBundle({
                    bundle: staging.capPath,
                    checkoutImpl,
                    expectedManifestSha256: sha256(manifestBytes),
                    repoRoot,
                    routesPath,
                    verifyOriginImpl,
                    verifyPublicBuildImpl,
                    verifySignerBuildImpl,
                });
                await requireRetainedDirectoryPath(parent, 'operator deployment bundle parent');
                const retainedDestination = await optionalLstat(pinnedDestination, { bigint: true });
                if (!sameInode(retainedDestination, staging.identity)) {
                    throw new OperatorBundleInstallStateUncertain(destination);
                }
                return result;
            } catch (error) {
                if (error instanceof OperatorBundleInstallStateUncertain) throw error;
                const retainedDestination = await optionalLstat(pinnedDestination, { bigint: true });
                if (!sameInode(retainedDestination, staging.identity)) {
                    throw new OperatorBundleInstallStateUncertain(destination, error);
                }
                throw new OperatorBundleInstalledInvalid(destination, error);
            }
        };
        await validateInstalled();
        try {
            await syncParentImpl(parent.handle);
        } catch (error) {
            try {
                await validateInstalled();
            } catch (validationError) {
                throw new OperatorBundleInstallStateUncertain(destination, new AggregateError([error, validationError]));
            }
            const cause = installOutcome?.commandError === undefined
                ? error
                : new AggregateError([installOutcome.commandError, error], 'install command and parent synchronization both failed');
            throw new OperatorBundleInstalledButParentSyncFailed(destination, cause);
        }
        const installed = await validateInstalled();
        if (installOutcome?.commandError !== undefined) {
            throw new OperatorBundleInstalledCommandError(destination, installOutcome.commandError);
        }
        return { ...installed, root: destination };
    } catch (error) {
        try {
            await removeOwnedStage(staging);
        } catch (cleanupError) {
            throw new AggregateError([error, cleanupError], 'operator deployment bundle assembly and secure staging cleanup both failed');
        }
        throw error;
    } finally {
        await staging.handle.close();
    }
    } finally {
        if (runtimeCapability !== undefined) await closeApprovedRuntimeCapability(runtimeCapability);
        await parent.handle.close();
    }
    } finally {
        await materializationCapability.handle.close();
    }
}

function sameJson(left, right) {
    return canonicalBytes(left, false).equals(canonicalBytes(right, false));
}

function validateManifestMaterializationBinding(manifest, materialization) {
    const receipt = materialization.receipt;
    for (const key of [
        'cargo_lock_sha256', 'publication_lock_sha256', 'publication_manifest_sha256',
        'publication_schema_version', 'source_commit', 'source_tree_sha1',
    ]) {
        if (manifest[key] !== receipt[key]) throw new Error(`deployment manifest ${key} differs from its materialization receipt`);
    }
    if (manifest.materialization.approved_receipt_sha256 !== materialization.receiptSha256
        || !sameJson(manifest.materialization.receipt.artifact, materialization.receiptArtifact)
        || !sameJson(manifest.materialization.sidecar.artifact, materialization.sidecarArtifact)) {
        throw new Error('deployment manifest materialization receipt authority is substituted');
    }
    const bindings = Object.fromEntries(receipt.origins.map(binding => [binding.origin, binding]));
    for (const origin of MATERIALIZATION_ORIGIN_ORDER) {
        const artifact = manifest.materialization.origin_inventories[origin].artifact;
        if (!sameJson(artifact, bindings[origin].inventory)) {
            throw new Error(`deployment manifest ${origin} inventory authority is substituted`);
        }
        if (ORIGINS[origin] !== undefined
            && manifest.origins[origin].inventory_sha256 !== artifact.sha256) {
            throw new Error(`deployment manifest ${origin} origin differs from its materialization inventory`);
        }
    }
}

function addPathParents(directories, path) {
    let parent = dirname(path).split(sep).join('/');
    while (parent !== '.') {
        directories.add(parent);
        parent = dirname(parent).split(sep).join('/');
    }
    directories.add('.');
}

function addInventoryClosure(expectedFiles, expectedDirectories, prefix, inventory) {
    for (const directory of inventory.directories) {
        expectedDirectories.add(directory.path === '.' ? prefix : `${prefix}/${directory.path}`);
    }
    for (const file of inventory.files) expectedFiles.add(`${prefix}/${file.path}`);
}

async function validateSealedClosure(root, expectedFiles, expectedDirectories, label) {
    for (const path of expectedFiles) addPathParents(expectedDirectories, path);
    const tree = await physicalTree(root, label);
    const actualFiles = new Set(tree.files.keys());
    const actualDirectories = new Set(tree.directories.keys());
    const missingFiles = [...expectedFiles].filter(path => !actualFiles.has(path));
    const extraFiles = [...actualFiles].filter(path => !expectedFiles.has(path));
    const missingDirectories = [...expectedDirectories].filter(path => !actualDirectories.has(path));
    const extraDirectories = [...actualDirectories].filter(path => !expectedDirectories.has(path));
    if ([missingFiles, extraFiles, missingDirectories, extraDirectories].some(items => items.length > 0)) {
        throw new Error(`${label} closure mismatch; missing files [${missingFiles.join(', ')}], extra files [${extraFiles.join(', ')}], missing directories [${missingDirectories.join(', ')}], extra directories [${extraDirectories.join(', ')}]`);
    }
    for (const [path, facts] of tree.directories) {
        if (Number(facts.mode & 0o7777n) !== 0o555) throw new Error(`${label} directory is not sealed mode 0555: ${path}`);
    }
    for (const [path, file] of tree.files) {
        if (Number(file.facts.mode & 0o7777n) !== 0o444) throw new Error(`${label} file is not sealed mode 0444: ${path}`);
    }
    return tree;
}

export async function validateOperatorDeploymentBundle({
    bundle,
    expectedManifestSha256,
    repoRoot,
    routesPath,
    verifyOriginImpl = verifyOrigin,
    verifyPublicBuildImpl = verifyPublicBuild,
    verifySignerBuildImpl = verifySignerBuild,
    checkoutImpl = readExactOperatorCheckout,
    processNodeVersion = process.version,
}) {
    requireExactNodeVersion(processNodeVersion);
    if (!DIGEST.test(expectedManifestSha256)) throw new Error('expected deployment manifest SHA-256 is invalid');
    const root = await requireRealDirectory(resolve(bundle), 'operator deployment bundle');
    const manifestValue = await parseJson(resolve(root, 'deployment-v2.json'), 'operator deployment V2 manifest');
    if (sha256(manifestValue.bytes) !== expectedManifestSha256) throw new Error('operator deployment manifest SHA-256 mismatch');
    validateManifestShape(manifestValue.value);
    const checkout = await checkoutImpl(repoRoot);
    if (checkout.sourceCommit !== manifestValue.value.source_commit) {
        throw new Error('operator deployment checkout is not the exact bundle source commit');
    }
    if (checkout.sourceTreeSha1 !== manifestValue.value.source_tree_sha1) {
        throw new Error('operator deployment checkout is not the exact clean bundle source tree');
    }
    if (!manifestValue.bytes.equals(canonicalBytes(manifestValue.value))) throw new Error('operator deployment manifest is not canonical JSON');
    const routeAuthorityFile = await stableRegularFile(routesPath, 'operator route authority');
    if (sha256(routeAuthorityFile.bytes) !== manifestValue.value.routes_sha256) throw new Error('operator deployment route authority differs from the bundle');
    if (checkout.cargoLockSha256 !== manifestValue.value.cargo_lock_sha256) throw new Error('operator deployment Cargo.lock differs from the bundle');
    const materialization = await validateCarriedMaterialization({
        authorityRoot: resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY),
        expectedReceiptSha256: manifestValue.value.materialization.approved_receipt_sha256,
        mappedOrigins: {
            deployment_authority: resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment'),
            identity_signer: resolve(root, ORIGINS.identity_signer.bundleDirectory),
            public: resolve(root, ORIGINS.public.bundleDirectory),
        },
    });
    validateManifestMaterializationBinding(manifestValue.value, materialization);
    await verifyPublicBuildImpl(resolve(root, ORIGINS.public.bundleDirectory));
    await verifySignerBuildImpl(resolve(root, ORIGINS.identity_signer.bundleDirectory));
    const boundedRuntime = await validateBoundedRuntimeOrigin({
        directoryMode: 0o555,
        expectedInventorySha256: manifestValue.value.runtime.inventory_sha256,
        fileMode: 0o444,
        inventoryPath: resolve(root, RUNTIME.inventory),
        label: 'operator deployment bundle runtime origin',
        root: resolve(root, RUNTIME.bundleDirectory),
    });
    const runtime = await verifyOriginImpl(
        'runtime', resolve(root, RUNTIME.bundleDirectory), resolve(root, RUNTIME.inventory),
        manifestValue.value.runtime.inventory_sha256,
        repoRoot,
        resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-authority.json'),
    );
    if (!sameJson(runtime.inventory, boundedRuntime.inventory)
        || runtime.inventory.source_commit !== manifestValue.value.source_commit
        || runtime.inventory.cargo_lock_sha256 !== manifestValue.value.cargo_lock_sha256
        || runtime.latest?.commit !== manifestValue.value.source_commit) {
        throw new Error('runtime inventory source authority differs from the deployment manifest');
    }
    const receipt = await parseJson(resolve(root, RUNTIME.bundleDirectory, 'wasm/datadir-deployment.json'), 'runtime datadir deployment receipt', true);
    const datadir = validateDatadirReceipt(receipt.value, receipt.bytes);
    const materializedReceipt = (await stableRegularFile(
        resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-deployment.json'),
        'carried datadir deployment receipt',
    )).bytes;
    if (!receipt.bytes.equals(materializedReceipt)) throw new Error('runtime datadir receipt differs from carried materialization authority');
    const authority = await parseJson(resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-authority.json'), 'datadir release authority', true);
    validateDatadirAuthority(authority.value, authority.bytes, receipt.value);
    if (JSON.stringify(datadir) !== JSON.stringify(manifestValue.value.datadir)) throw new Error('runtime datadir receipt differs from the deployment manifest');
    const routeAuthority = JSON.parse(routeAuthorityFile.bytes.toString('utf8'));
    validateOperatorRouteAuthority({
        datadirWorker: 'robinhood-datadir-assets', expectedRoutes: routeAuthority.routes,
        publicHost: PUBLIC_HOST, publicWorker: 'robinhood-public-site', runtimeWorker: 'robinhood-runtime-assets',
    });
    const exposure = await parseJson(resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/exposure-v3.json'), 'carried deployment exposure', true);
    validateDeploymentExposure(exposure.value, routeAuthority.routes);
    const expectedFiles = new Set(['deployment-v2.json', RUNTIME.inventory]);
    const expectedDirectories = new Set(['.']);
    const authorityInventory = materializationAuthorityInventory(
        materialization.receipt, materialization.originInventories,
        materialization.receiptArtifact, materialization.sidecarArtifact,
    );
    addInventoryClosure(expectedFiles, expectedDirectories, MATERIALIZATION_AUTHORITY_DIRECTORY, authorityInventory);
    for (const [origin, paths] of Object.entries(ORIGINS)) {
        addInventoryClosure(expectedFiles, expectedDirectories, paths.bundleDirectory, materialization.originInventories[origin]);
    }
    for (const artifact of runtime.inventory.artifacts) expectedFiles.add(`${RUNTIME.bundleDirectory}/${artifact.path}`);
    expectedFiles.add(`${RUNTIME.bundleDirectory}/_headers`);
    const tree = await validateSealedClosure(root, expectedFiles, expectedDirectories, 'operator deployment bundle');
    for (const path of tree.files.keys()) {
        if (containsForbiddenStaticPath(path)) throw new Error(`operator deployment bundle contains forbidden private/Full/datadir path ${path}`);
    }
    return {
        checkout,
        manifest: manifestValue.value,
        manifestSha256: expectedManifestSha256,
        root,
        routes: canonical(routeAuthority.routes),
    };
}

async function sealTree(root, directoryMode, fileMode, label) {
    const tree = await physicalTree(root, `${label} input`);
    for (const [path, file] of tree.files) {
        const target = resolve(root, path);
        const before = await lstat(target, { bigint: true });
        if (stableNodeIdentity(before) !== stableNodeIdentity(file.facts)) throw new Error(`${label} file changed before sealing: ${path}`);
        await chmod(target, fileMode);
        const after = await lstat(target, { bigint: true });
        if (inodeIdentity(after) !== inodeIdentity(before) || Number(after.mode & 0o7777n) !== fileMode) {
            throw new Error(`${label} file changed while sealing: ${path}`);
        }
    }
    const directories = [...tree.directories.entries()].sort(([left], [right]) => {
        const depth = right.split('/').length - left.split('/').length;
        return depth === 0 ? utf8Order(right, left) : depth;
    });
    for (const [path, facts] of directories) {
        const target = path === '.' ? root : resolve(root, path);
        const before = await lstat(target, { bigint: true });
        if (stableNodeIdentity(before) !== stableNodeIdentity(facts)) throw new Error(`${label} directory changed before sealing: ${path}`);
        await chmod(target, directoryMode);
        const after = await lstat(target, { bigint: true });
        if (inodeIdentity(after) !== inodeIdentity(before) || Number(after.mode & 0o7777n) !== directoryMode) {
            throw new Error(`${label} directory changed while sealing: ${path}`);
        }
    }
}

async function makeReadOnly(root) { await sealTree(root, 0o555, 0o444, 'read-only release tree'); }
async function makeOwnerReadOnly(root) { await sealTree(root, 0o500, 0o400, 'private Wrangler snapshot'); }

function wranglerSnapshotAuthorityFromStageTree(tree, label, checkout) {
    const origin = WRANGLER_SNAPSHOT_ORIGINS[label];
    if (origin === undefined) throw new Error(`invalid Wrangler snapshot origin ${label}`);
    const directories = new Set(['.']);
    const files = new Map();
    for (const [path] of tree.directories) {
        if (path.startsWith(`${origin}/`)) directories.add(path.slice(origin.length + 1));
    }
    for (const [path, file] of tree.files) {
        if (path.startsWith(`${origin}/`)) {
            const relative = path.slice(origin.length + 1);
            files.set(relative, {
                byteLength: file.bytes.length,
                sha256: sha256(file.bytes),
            });
        }
    }
    const configName = `wrangler-${label}.json`;
    const configSha256 = checkout.deploymentConfigSha256?.[configName];
    const config = tree.files.get(`deploy/${configName}`);
    if (!DIGEST.test(configSha256) || config === undefined
        || sha256(config.bytes) !== configSha256 || files.size === 0) {
        throw new Error(`Wrangler ${label} snapshot authority is incomplete`);
    }
    return canonical({
        configByteLength: config.bytes.length,
        configName,
        configSha256,
        directories: [...directories].sort(utf8Order),
        files: Object.fromEntries([...files.entries()].sort(([left], [right]) => utf8Order(left, right))),
        origin,
    });
}

export async function validateOperatorWranglerSnapshot({ label, snapshot, authority }) {
    const origin = WRANGLER_SNAPSHOT_ORIGINS[label];
    if (origin === undefined) throw new Error(`invalid Wrangler snapshot origin ${label}`);
    exactKeys(authority, [
        'configByteLength', 'configName', 'configSha256', 'directories', 'files', 'origin',
    ], `Wrangler ${label} snapshot authority`);
    if (authority.origin !== origin || authority.configName !== `wrangler-${label}.json`
        || !Number.isSafeInteger(authority.configByteLength) || authority.configByteLength < 1
        || authority.configByteLength > MAX_REGULAR_FILE_BYTES
        || !DIGEST.test(authority.configSha256) || !Array.isArray(authority.directories)
        || authority.directories.length < 1 || authority.directories.length > MAX_TREE_ENTRIES
        || authority.files === null || typeof authority.files !== 'object' || Array.isArray(authority.files)) {
        throw new Error(`Wrangler ${label} snapshot authority is invalid`);
    }
    const snapshotTree = await physicalTree(snapshot, `Wrangler ${label} snapshot`);
    const expectedFiles = new Map();
    const expectedDirectories = new Set(['.', 'deploy', origin]);
    let previousDirectory;
    for (const path of authority.directories) {
        validateRelativePath(path, `Wrangler ${label} authority directory`, true);
        if ((previousDirectory !== undefined && utf8Order(previousDirectory, path) >= 0)
            || (previousDirectory === undefined && path !== '.')) {
            throw new Error(`Wrangler ${label} authority directories are not exact sorted closure`);
        }
        previousDirectory = path;
        if (path !== '.') expectedDirectories.add(`${origin}/${path}`);
    }
    const authorityFiles = Object.entries(authority.files);
    if (authorityFiles.length < 1 || authorityFiles.length > MAX_TREE_ENTRIES) {
        throw new Error(`Wrangler ${label} authority file count is invalid`);
    }
    let previousFile;
    let totalBytes = 0;
    for (const [path, artifact] of authorityFiles) {
        validateRelativePath(path, `Wrangler ${label} authority file`);
        if (previousFile !== undefined && utf8Order(previousFile, path) >= 0) {
            throw new Error(`Wrangler ${label} authority files are not exact sorted closure`);
        }
        previousFile = path;
        exactKeys(artifact, ['byteLength', 'sha256'], `Wrangler ${label} authority file ${path}`);
        if (!Number.isSafeInteger(artifact.byteLength) || artifact.byteLength < 1
            || artifact.byteLength > MAX_REGULAR_FILE_BYTES || !DIGEST.test(artifact.sha256)) {
            throw new Error(`Wrangler ${label} authority file is invalid: ${path}`);
        }
        totalBytes += artifact.byteLength;
        if (totalBytes > MAX_TREE_BYTES) throw new Error(`Wrangler ${label} authority exceeds bounded byte total`);
        expectedFiles.set(`${origin}/${path}`, artifact);
    }
    expectedFiles.set(`deploy/${authority.configName}`, {
        byteLength: authority.configByteLength,
        sha256: authority.configSha256,
    });
    if (!sameJson([...snapshotTree.files.keys()], [...expectedFiles.keys()].sort(utf8Order))
        || !sameJson([...snapshotTree.directories.keys()], [...expectedDirectories].sort(utf8Order))) {
        throw new Error(`Wrangler ${label} snapshot closure differs from its approved authority`);
    }
    for (const [path, expected] of expectedFiles) {
        const actual = snapshotTree.files.get(path);
        if (sha256(actual.bytes) !== expected.sha256
            || (expected.byteLength !== undefined && actual.bytes.length !== expected.byteLength)
            || Number(actual.facts.mode & 0o7777n) !== 0o400) {
            throw new Error(`Wrangler ${label} snapshot differs from its approved authority at ${path}`);
        }
    }
    for (const [path, facts] of snapshotTree.directories) {
        if (Number(facts.mode & 0o7777n) !== 0o500) throw new Error(`Wrangler ${label} snapshot directory is not owner-read-only: ${path}`);
    }
    return { configPath: `deploy/${authority.configName}`, root: snapshot };
}

export async function createOperatorWranglerSnapshot({ label, stage, authority, copyImpl = copyBoundedTree }) {
    const origin = WRANGLER_SNAPSHOT_ORIGINS[label];
    if (origin === undefined) throw new Error(`invalid Wrangler snapshot origin ${label}`);
    const parent = await openUnaliasedDirectory(PRIVATE_TEMP_ROOT, `Wrangler ${label} snapshot parent`);
    let created;
    let capability;
    try {
        created = await mkdtemp(resolve(parent.capPath, `robinhood-wrangler-${label}-`));
        capability = await openDirectoryEntry(parent, basename(created), `Wrangler ${label} snapshot`);
        await mkdir(resolve(capability.capPath, 'sealed'), { mode: 0o700 });
        await mkdir(resolve(capability.capPath, 'sealed/deploy'), { mode: 0o700 });
        await mkdir(resolve(capability.capPath, 'work'), { mode: 0o700 });
        await copyImpl(resolve(stage, origin), resolve(capability.capPath, 'sealed', origin), {
            label: `Wrangler ${label} origin snapshot copy`,
        });
        const configName = `wrangler-${label}.json`;
        const config = await stableRegularFile(resolve(stage, 'deploy', configName), `Wrangler ${label} config`);
        await writeFile(resolve(capability.capPath, 'sealed/deploy', configName), config.bytes, { flag: 'wx' });
        await makeOwnerReadOnly(resolve(capability.capPath, 'sealed'));
        const sealedPath = resolve(capability.capPath, 'sealed');
        await validateOperatorWranglerSnapshot({ authority, label, snapshot: sealedPath });
        return {
            ...capability,
            configPath: `../sealed/deploy/${configName}`,
            cwd: `/proc/${process.pid}/fd/${capability.handle.fd}/work`,
            label,
            sealedPath,
        };
    } catch (error) {
        try {
            if (capability !== undefined) {
                await clearOwnedDirectory(capability.handle, `Wrangler ${label} snapshot`);
                await capability.handle.close();
                await rmdir(created);
            } else if (created !== undefined) {
                await rmdir(created);
            }
            await parent.handle.close();
        } catch (cleanupError) {
            throw new AggregateError([error, cleanupError], `Wrangler ${label} snapshot creation and cleanup failed`);
        }
        throw error;
    }
}

export async function removeOperatorWranglerSnapshot(snapshot) {
    try {
        await removeOwnedStage(snapshot);
    } finally {
        await snapshot.handle.close();
        await snapshot.parent.handle.close();
    }
}

export async function validateOperatorDeploymentStage({
    stage, expectedManifestSha256, repoRoot, routesPath, deploymentConfigDirectory,
    verifyOriginImpl = verifyOrigin, verifyPublicBuildImpl = verifyPublicBuild,
    verifySignerBuildImpl = verifySignerBuild, checkoutImpl = readExactOperatorCheckout,
    processNodeVersion = process.version,
}) {
    requireExactNodeVersion(processNodeVersion);
    if (!DIGEST.test(expectedManifestSha256)) throw new Error('expected deployment manifest SHA-256 is invalid');
    const root = await requireRealDirectory(resolve(stage), 'operator deployment stage');
    const manifestValue = await parseJson(resolve(root, 'deployment-v2.json'), 'staged operator deployment V2 manifest');
    if (sha256(manifestValue.bytes) !== expectedManifestSha256
        || !manifestValue.bytes.equals(canonicalBytes(manifestValue.value))) {
        throw new Error('staged operator deployment manifest authority differs');
    }
    validateManifestShape(manifestValue.value);
    const checkout = await checkoutImpl(repoRoot);
    if (checkout.sourceCommit !== manifestValue.value.source_commit
        || checkout.sourceTreeSha1 !== manifestValue.value.source_tree_sha1
        || checkout.cargoLockSha256 !== manifestValue.value.cargo_lock_sha256
        || sha256((await stableRegularFile(routesPath, 'operator route authority')).bytes) !== manifestValue.value.routes_sha256) {
        throw new Error('staged operator deployment checkout/Cargo/routes authority differs');
    }
    const materialization = await validateCarriedMaterialization({
        authorityRoot: resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY),
        expectedReceiptSha256: manifestValue.value.materialization.approved_receipt_sha256,
        mappedOrigins: {
            deployment_authority: resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment'),
            identity_signer: resolve(root, 'signer-dist'),
            public: resolve(root, 'dist'),
        },
    });
    validateManifestMaterializationBinding(manifestValue.value, materialization);
    await verifyPublicBuildImpl(resolve(root, 'dist'));
    await verifySignerBuildImpl(resolve(root, 'signer-dist'));
    const boundedRuntime = await validateBoundedRuntimeOrigin({
        directoryMode: 0o555,
        expectedInventorySha256: manifestValue.value.runtime.inventory_sha256,
        fileMode: 0o444,
        inventoryPath: resolve(root, RUNTIME.inventory),
        label: 'staged operator deployment runtime origin',
        root: resolve(root, 'runtime-dist'),
    });
    const runtime = await verifyOriginImpl(
        'runtime', resolve(root, 'runtime-dist'), resolve(root, RUNTIME.inventory),
        manifestValue.value.runtime.inventory_sha256, repoRoot,
        resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-authority.json'),
    );
    if (!sameJson(runtime.inventory, boundedRuntime.inventory)
        || runtime.inventory.source_commit !== manifestValue.value.source_commit
        || runtime.inventory.cargo_lock_sha256 !== manifestValue.value.cargo_lock_sha256
        || runtime.latest?.commit !== manifestValue.value.source_commit) {
        throw new Error('staged runtime inventory source authority differs from the deployment manifest');
    }
    const receipt = await parseJson(resolve(root, 'runtime-dist/wasm/datadir-deployment.json'), 'staged runtime datadir receipt', true);
    const materializedReceipt = (await stableRegularFile(
        resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-deployment.json'),
        'staged datadir deployment receipt',
    )).bytes;
    if (!receipt.bytes.equals(materializedReceipt)
        || !sameJson(validateDatadirReceipt(receipt.value, receipt.bytes), manifestValue.value.datadir)) {
        throw new Error('staged runtime datadir receipt differs from carried authority');
    }
    const authority = await parseJson(resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/datadir-authority.json'), 'staged datadir authority', true);
    validateDatadirAuthority(authority.value, authority.bytes, receipt.value);
    const routeAuthority = JSON.parse((await stableRegularFile(routesPath, 'operator route authority')).bytes.toString('utf8'));
    validateOperatorRouteAuthority({
        datadirWorker: 'robinhood-datadir-assets', expectedRoutes: routeAuthority.routes,
        publicHost: PUBLIC_HOST, publicWorker: 'robinhood-public-site', runtimeWorker: 'robinhood-runtime-assets',
    });
    const exposure = await parseJson(resolve(root, MATERIALIZATION_AUTHORITY_DIRECTORY, 'deployment/exposure-v3.json'), 'staged deployment exposure', true);
    validateDeploymentExposure(exposure.value, routeAuthority.routes);
    for (const file of DEPLOYMENT_CONFIG_FILES) {
        const stagedConfig = await stableRegularFile(resolve(root, 'deploy', file), `staged deployment config ${file}`);
        if (sha256(stagedConfig.bytes) !== checkout.deploymentConfigSha256?.[file]) {
            throw new Error(`staged deployment config differs from exact tracked commit: ${file}`);
        }
    }
    const expectedFiles = new Set(['deployment-v2.json', RUNTIME.inventory]);
    const expectedDirectories = new Set(['.']);
    const authorityInventory = materializationAuthorityInventory(
        materialization.receipt, materialization.originInventories,
        materialization.receiptArtifact, materialization.sidecarArtifact,
    );
    addInventoryClosure(expectedFiles, expectedDirectories, MATERIALIZATION_AUTHORITY_DIRECTORY, authorityInventory);
    addInventoryClosure(expectedFiles, expectedDirectories, 'dist', materialization.originInventories.public);
    addInventoryClosure(expectedFiles, expectedDirectories, 'signer-dist', materialization.originInventories.identity_signer);
    for (const artifact of runtime.inventory.artifacts) expectedFiles.add(`runtime-dist/${artifact.path}`);
    expectedFiles.add('runtime-dist/_headers');
    for (const file of DEPLOYMENT_CONFIG_FILES) expectedFiles.add(`deploy/${file}`);
    expectedDirectories.add('deploy');
    const tree = await validateSealedClosure(root, expectedFiles, expectedDirectories, 'operator deployment stage');
    for (const path of tree.files.keys()) {
        if (containsForbiddenStaticPath(path)) throw new Error(`operator deployment stage contains forbidden private/Full/datadir path ${path}`);
    }
    return {
        checkout,
        manifest: manifestValue.value,
        manifestSha256: expectedManifestSha256,
        root,
        routes: canonical(routeAuthority.routes),
        wranglerSnapshotAuthorities: Object.fromEntries(
            Object.keys(WRANGLER_SNAPSHOT_ORIGINS).map(label => [
                label,
                wranglerSnapshotAuthorityFromStageTree(tree, label, checkout),
            ]),
        ),
    };
}

export async function stageOperatorDeploymentBundle({
    bundle, expectedManifestSha256, stage, repoRoot, routesPath,
    deploymentConfigDirectory, verifyOriginImpl = verifyOrigin,
    verifyPublicBuildImpl = verifyPublicBuild, verifySignerBuildImpl = verifySignerBuild,
    checkoutImpl = readExactOperatorCheckout,
    processNodeVersion = process.version,
    installImpl = atomicInstallNoreplace, syncParentImpl = syncParentDirectory,
    syncTreeImpl = syncTree, copyImpl = copyBoundedTree, copyFileImpl = copyBoundedFile,
}) {
    requireExactNodeVersion(processNodeVersion);
    const output = resolve(stage);
    const parent = await openUnaliasedDirectory(dirname(output), 'operator deployment stage parent');
    let bundleCapability;
    let temporary;
    try {
        bundleCapability = await openUnaliasedDirectory(bundle, 'operator deployment bundle');
        const pinnedOutput = resolve(parent.capPath, basename(output));
        await requireAbsent(pinnedOutput, 'operator deployment stage');
        const verified = await validateOperatorDeploymentBundle({
            bundle: bundleCapability.capPath,
            checkoutImpl,
            expectedManifestSha256,
            processNodeVersion,
            repoRoot,
            routesPath,
            verifyOriginImpl,
            verifyPublicBuildImpl,
            verifySignerBuildImpl,
        });
        const created = await mkdtemp(resolve(parent.capPath, `.${basename(output)}.staging-`));
        temporary = await openDirectoryEntry(parent, basename(created), 'operator deployment private stage');
        try {
            for (const [origin, destination] of [['public', 'dist'], ['identity_signer', 'signer-dist']]) {
                await copyImpl(resolve(verified.root, ORIGINS[origin].bundleDirectory), resolve(temporary.capPath, destination), {
                    label: `staged ${origin} origin copy`,
                });
            }
            await copyImpl(resolve(verified.root, RUNTIME.bundleDirectory), resolve(temporary.capPath, 'runtime-dist'), {
                label: 'staged runtime origin copy',
            });
            await copyImpl(resolve(verified.root, 'inventories'), resolve(temporary.capPath, 'inventories'), {
                label: 'staged runtime inventory directory copy',
            });
            await copyImpl(resolve(verified.root, 'authorities'), resolve(temporary.capPath, 'authorities'), {
                label: 'staged materialization authority copy',
            });
            await copyFileImpl(
                resolve(verified.root, 'deployment-v2.json'),
                resolve(temporary.capPath, 'deployment-v2.json'),
                'staged deployment manifest copy',
            );
            await mkdir(resolve(temporary.capPath, 'deploy'));
            for (const file of DEPLOYMENT_CONFIG_FILES) {
                const source = await stableRegularFile(resolve(deploymentConfigDirectory, file), `tracked deployment config ${file}`);
                if (sha256(source.bytes) !== verified.checkout.deploymentConfigSha256?.[file]) {
                    throw new Error(`deployment config differs from exact tracked commit: ${file}`);
                }
                await writeFile(resolve(temporary.capPath, 'deploy', file), source.bytes, { flag: 'wx' });
            }
            await validateOperatorDeploymentBundle({
                bundle: bundleCapability.capPath,
                checkoutImpl,
                expectedManifestSha256,
                processNodeVersion,
                repoRoot,
                routesPath,
                verifyOriginImpl,
                verifyPublicBuildImpl,
                verifySignerBuildImpl,
            });
            await makeReadOnly(temporary.capPath);
            await validateOperatorDeploymentStage({
                deploymentConfigDirectory,
                expectedManifestSha256,
                processNodeVersion,
                checkoutImpl,
                repoRoot,
                routesPath,
                stage: temporary.capPath,
                verifyOriginImpl,
                verifyPublicBuildImpl,
                verifySignerBuildImpl,
            });
            await syncTreeImpl(temporary.capPath);
            await requireAbsent(pinnedOutput, 'operator deployment stage');
            const installOutcome = await installImpl(temporary.path, pinnedOutput, { parentCapability: parent });
            const validateInstalled = async () => {
                try {
                    await requireRetainedDirectoryPath(parent, 'operator deployment stage parent');
                } catch (error) {
                    throw new OperatorBundleInstallStateUncertain(output, error);
                }
                const facts = await optionalLstat(pinnedOutput, { bigint: true });
                if (!sameInode(facts, temporary.identity)) throw new OperatorBundleInstallStateUncertain(output);
                let result;
                try {
                    result = await validateOperatorDeploymentStage({
                        deploymentConfigDirectory,
                        expectedManifestSha256,
                        processNodeVersion,
                        checkoutImpl,
                        repoRoot,
                        routesPath,
                        stage: pinnedOutput,
                        verifyOriginImpl,
                        verifyPublicBuildImpl,
                        verifySignerBuildImpl,
                    });
                } catch (error) {
                    const retained = await optionalLstat(pinnedOutput, { bigint: true });
                    if (!sameInode(retained, temporary.identity)) {
                        throw new OperatorBundleInstallStateUncertain(output, error);
                    }
                    throw new OperatorBundleInstalledInvalid(output, error);
                }
                const retained = await optionalLstat(pinnedOutput, { bigint: true });
                if (!sameInode(retained, temporary.identity)) throw new OperatorBundleInstallStateUncertain(output);
                return result;
            };
            await validateInstalled();
            try {
                await syncParentImpl(parent.handle);
            } catch (error) {
                await validateInstalled();
                const cause = installOutcome?.commandError === undefined
                    ? error
                    : new AggregateError([installOutcome.commandError, error]);
                throw new OperatorBundleInstalledButParentSyncFailed(output, cause);
            }
            const staged = await validateInstalled();
            if (installOutcome?.commandError !== undefined) {
                throw new OperatorBundleInstalledCommandError(output, installOutcome.commandError);
            }
            try {
                await requireRetainedDirectoryPath(parent, 'operator deployment stage parent');
            } catch (error) {
                throw new OperatorBundleInstallStateUncertain(output, error);
            }
            return { ...staged, root: output, stage: output };
        } catch (error) {
            try {
                await removeOwnedStage(temporary);
            } catch (cleanupError) {
                throw new AggregateError([error, cleanupError], 'operator deployment staging and secure cleanup both failed');
            }
            throw error;
        } finally {
            await temporary.handle.close();
        }
    } finally {
        if (bundleCapability !== undefined) await bundleCapability.handle.close();
        await parent.handle.close();
    }
}

async function main() {
    const [command, ...args] = process.argv.slice(2);
    if (command === 'assemble' && args.length === 6) {
        const [
            materializationRoot, runtimeRoot, output,
            expectedMaterializationReceiptSha256, expectedRuntimeInventorySha256,
            repoRoot,
        ] = args;
        const result = await assembleOperatorDeploymentBundle({
            expectedMaterializationReceiptSha256,
            expectedRuntimeInventorySha256,
            materializationRoot,
            output,
            repoRoot,
            routesPath: resolve(repoRoot, 'wasm-www/deploy/public-routes.json'),
            runtimeRoot,
        });
        console.log(`assembled operator deployment bundle ${result.manifestSha256}`);
        return;
    }
    if (command === 'verify' && args.length === 3) {
        const [bundle, expectedManifestSha256, repoRoot] = args;
        const result = await validateOperatorDeploymentBundle({
            bundle, expectedManifestSha256, repoRoot,
            routesPath: resolve(repoRoot, 'wasm-www/deploy/public-routes.json'),
        });
        console.log(`verified operator deployment bundle ${result.manifestSha256}`);
        return;
    }
    throw new Error('usage: operator-deployment-bundle.mjs assemble MATERIALIZATION WASM_STATIC OUTPUT APPROVED_MATERIALIZATION_RECEIPT_SHA256 APPROVED_RUNTIME_INVENTORY_SHA256 REPO_ROOT | verify BUNDLE EXPECTED_MANIFEST_SHA256 REPO_ROOT');
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
}
