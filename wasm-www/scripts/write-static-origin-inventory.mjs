import { createHash } from 'node:crypto';
import { lstat, mkdir, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { dirname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const ORIGINS = new Set(['public', 'runtime', 'datadir', 'identity_signer']);

function utf8Order(left, right) {
    return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function mediaType(path) {
    if (path.endsWith('.html')) return 'text/html';
    if (path.endsWith('.js') || path.endsWith('.mjs')) return 'text/javascript';
    if (path.endsWith('.wasm')) return 'application/wasm';
    if (path.endsWith('.json')) return 'application/json';
    if (path.endsWith('.gz')) return 'application/gzip';
    if (path.endsWith('.zst')) return 'application/zstd';
    if (path.endsWith('.opus')) return 'audio/ogg; codecs=opus';
    if (path.endsWith('.bin')) return 'application/octet-stream';
    if (path.endsWith('.css')) return 'text/css';
    if (path.endsWith('.png')) return 'image/png';
    if (path.endsWith('.svg')) return 'image/svg+xml';
    if (path.endsWith('.ico')) return 'image/x-icon';
    if (path.endsWith('.ttf')) return 'font/ttf';
    if (path.endsWith('.woff2')) return 'font/woff2';
    if (path.endsWith('.txt')) return 'text/plain';
    throw new Error(`static output has no closed media-type policy: ${path}`);
}

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.entries(value)
            .sort(([left], [right]) => utf8Order(left, right))
            .map(([key, item]) => [key, canonical(item)]));
    }
    return value;
}

export async function buildStaticOriginInventory({ origin, root, sourceCommit, cargoLockSha256 }) {
    if (!ORIGINS.has(origin)) throw new Error('origin must be public, runtime, datadir, or identity_signer');
    if (!/^[0-9a-f]{40}$/u.test(sourceCommit) || !/^[0-9a-f]{64}$/u.test(cargoLockSha256)) {
        throw new Error('source commit and Cargo.lock digest must be canonical lowercase hashes');
    }
    const rootFacts = await lstat(root).catch(() => undefined);
    if (rootFacts === undefined || !rootFacts.isDirectory() || rootFacts.isSymbolicLink()) {
        throw new Error(`static origin is not a real directory: ${resolve(root)}`);
    }
    const realRoot = await realpath(root);
    const artifacts = [];
    async function visit(directory) {
        const entries = (await readdir(directory, { withFileTypes: true }))
            .sort((left, right) => utf8Order(left.name, right.name));
        for (const entry of entries) {
            const absolute = resolve(directory, entry.name);
            const path = relative(realRoot, absolute).split(sep).join('/');
            const facts = await lstat(absolute);
            if (facts.isSymbolicLink()) throw new Error(`symlink is forbidden in static output: ${path}`);
            if (facts.isDirectory()) await visit(absolute);
            else if (facts.isFile()) {
                if (path === '_headers') continue;
                if (path.endsWith('.map')) throw new Error(`source map is forbidden in static output: ${path}`);
                artifacts.push({
                    byte_length: facts.size,
                    media_type: mediaType(path),
                    path,
                    sha256: createHash('sha256').update(await readFile(absolute)).digest('hex'),
                });
            } else throw new Error(`non-regular static output is forbidden: ${path}`);
        }
    }
    await visit(realRoot);
    if (artifacts.length === 0) throw new Error(`static origin is empty: ${realRoot}`);
    artifacts.sort((left, right) => utf8Order(left.path, right.path));
    return canonical({
        artifacts,
        cargo_lock_sha256: cargoLockSha256,
        origin,
        schema_version: 1,
        source_commit: sourceCommit,
    });
}

export async function writeStaticOriginInventory({ origin, root, sourceCommit, cargoLockSha256, output }) {
    const realRoot = await realpath(root);
    const outputPath = resolve(output);
    if (outputPath === realRoot || outputPath.startsWith(`${realRoot}${sep}`)) {
        throw new Error('inventory output must be outside the inventoried origin');
    }
    const document = await buildStaticOriginInventory({ origin, root, sourceCommit, cargoLockSha256 });
    await mkdir(dirname(outputPath), { recursive: true });
    await writeFile(outputPath, JSON.stringify(document), { flag: 'wx' });
    return document;
}

async function main() {
    const [origin, root, sourceCommit, cargoLockSha256, output, extra] = process.argv.slice(2);
    if ([origin, root, sourceCommit, cargoLockSha256, output].some(value => value === undefined)
        || extra !== undefined) {
        throw new Error('usage: write-static-origin-inventory.mjs ORIGIN ROOT SOURCE_COMMIT CARGO_LOCK_SHA256 OUTPUT');
    }
    const document = await writeStaticOriginInventory({ origin, root, sourceCommit, cargoLockSha256, output });
    console.log(`wrote ${document.origin} inventory (${document.artifacts.length} artifacts)`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
