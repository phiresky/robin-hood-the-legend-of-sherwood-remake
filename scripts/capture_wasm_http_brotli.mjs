#!/usr/bin/env node
// Capture Wrangler's actual local HTTP encoding, without deploying anything.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer, get } from 'node:http';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { brotliDecompressSync } from 'node:zlib';
import { setTimeout as delay } from 'node:timers/promises';

const { values } = parseArgs({ options: {
    worktree: { type: 'string' }, pkg: { type: 'string' }, wasm: { type: 'string' },
    output: { type: 'string' }, 'timeout-ms': { type: 'string', default: '60000' },
} });
if (!values.worktree || !values.output || Boolean(values.pkg) === Boolean(values.wasm)) {
    throw new Error('Usage: node scripts/capture_wasm_http_brotli.mjs --worktree REPO (--pkg PACKAGE | --wasm RAW_WASM) --output FILE.br [--timeout-ms 60000]');
}
const timeoutMs = Number(values['timeout-ms']);
if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) throw new Error('--timeout-ms must be a positive integer');
const tree = resolve(values.worktree);
const input = values.wasm ? resolve(values.wasm) : resolve(values.pkg, 'robin_bg.wasm');
const output = resolve(values.output);
const headerPath = join(tree, 'wasm-www/deploy/runtime-headers.txt');
const wranglerRoot = join(tree, 'wasm-www/node_modules/wrangler');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const [raw, runtimeHeaders, runtimeConfig, packageJson, wranglerPackage] = await Promise.all([
    readFile(input), readFile(headerPath),
    readFile(join(tree, 'wasm-www/deploy/wrangler-runtime.json'), 'utf8').then(JSON.parse),
    readFile(join(tree, 'wasm-www/package.json'), 'utf8').then(JSON.parse),
    readFile(join(wranglerRoot, 'package.json'), 'utf8').then(JSON.parse),
]);
const pinnedVersion = packageJson.devDependencies?.wrangler ?? packageJson.dependencies?.wrangler;
if (pinnedVersion !== wranglerPackage.version) {
    throw new Error(`Wrangler must match the exact repository pin: declared ${pinnedVersion}, installed ${wranglerPackage.version}`);
}
if (typeof runtimeConfig.compatibility_date !== 'string') throw new Error('Runtime configuration has no compatibility_date');
if (!raw.subarray(0, 4).equals(Buffer.from([0, 97, 115, 109]))) throw new Error('Input is not a raw WebAssembly module');

const controller = new AbortController();
const { signal } = controller;
const cancel = name => controller.abort(new Error(`Capture cancelled by ${name}`));
const onInterrupt = () => cancel('SIGINT');
const onTerminate = () => cancel('SIGTERM');
process.on('SIGINT', onInterrupt);
process.on('SIGTERM', onTerminate);
const timeout = setTimeout(() => controller.abort(new Error(`Capture exceeded ${timeoutMs} ms`)), timeoutMs);
let scratch;
let child;
let childClosed;
let childFailure;
let logs = '';
let wroteOutput = false;
let wroteMetadata = false;

async function reservePort() {
    const server = createServer();
    await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
    });
    const port = server.address().port;
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    return port;
}
function request(port) {
    return new Promise((resolve, reject) => {
        const req = get({ hostname: '127.0.0.1', port, path: '/wasm/test/robin_bg.wasm',
            headers: { 'Accept-Encoding': 'br' }, signal,
        }, response => {
            const chunks = [];
            response.on('data', chunk => chunks.push(chunk));
            response.on('error', reject);
            response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks) }));
        });
        req.on('error', reject);
        req.setTimeout(10000, () => req.destroy(new Error('Local HTTP request timed out after 10000 ms')));
    });
}
function signalChild(name) {
    try {
        if (process.platform === 'win32') child.kill(name);
        else process.kill(-child.pid, name); // Include Wrangler's workerd descendants.
    } catch (error) { if (error.code !== 'ESRCH') throw error; }
}
async function stopChild() {
    if (!child || !childClosed) return;
    if (child.exitCode === null && child.signalCode === null) signalChild('SIGINT');
    let timer;
    try {
        await Promise.race([childClosed, new Promise(resolve => {
            timer = setTimeout(() => { signalChild('SIGKILL'); resolve(); }, 3000);
        })]);
    } finally { clearTimeout(timer); }
    // SIGKILL also closes the pipe handles before scratch removal.
    await childClosed;
}
try {
    signal.throwIfAborted();
    scratch = await mkdtemp(join(tmpdir(), 'robin-http-brotli-'));
    await mkdir(join(scratch, 'assets/wasm/test'), { recursive: true });
    await copyFile(input, join(scratch, 'assets/wasm/test/robin_bg.wasm'));
    await writeFile(join(scratch, 'assets/_headers'), runtimeHeaders);
    const config = { name: 'robin-http-brotli-capture', compatibility_date: runtimeConfig.compatibility_date,
        assets: { directory: './assets', html_handling: 'none' } };
    await writeFile(join(scratch, 'wrangler.json'), JSON.stringify(config));
    const port = await reservePort();
    signal.throwIfAborted();
    child = spawn(process.execPath, [join(wranglerRoot, 'bin/wrangler.js'), 'dev',
        '--config', join(scratch, 'wrangler.json'), '--local', '--ip', '127.0.0.1',
        '--port', String(port), '--inspector-port', '0'], {
        cwd: scratch, detached: process.platform !== 'win32',
        env: { ...process.env, CI: 'true', XDG_CONFIG_HOME: join(scratch, 'config'), WRANGLER_SEND_METRICS: 'false' },
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    childClosed = new Promise(resolve => {
        child.once('error', error => { childFailure = error; });
        child.once('close', (code, signal) => { childFailure ??= new Error(`Wrangler exited: code=${code}, signal=${signal}`); resolve(); });
    });
    const collect = chunk => { logs = (logs + chunk.toString()).slice(-1024 * 1024); };
    child.stdout.on('data', collect); child.stderr.on('data', collect);
    let response;
    while (!response) {
        signal.throwIfAborted();
        if (childFailure) throw childFailure;
        try { response = await request(port); }
        catch (error) {
            signal.throwIfAborted();
            if (childFailure) throw childFailure;
            if (error.code !== 'ECONNREFUSED') throw error;
            await delay(100, undefined, { signal });
        }
    }
    if (response.status !== 200 || response.headers['content-encoding'] !== 'br') {
        throw new Error(`Expected HTTP 200 Brotli: status=${response.status}, headers=${JSON.stringify(response.headers)}`);
    }
    const decoded = brotliDecompressSync(response.body);
    if (!decoded.equals(raw)) throw new Error('HTTP Brotli body does not decode to the exact input WASM');
    signal.throwIfAborted();
    const facts = { input, worktree: tree, capturedAt: new Date().toISOString(),
        rawBytes: raw.length, rawSha256: sha256(raw),
        encodedBytes: response.body.length, encodedSha256: sha256(response.body),
        decodedSha256: sha256(decoded), headers: response.headers,
        runtimeHeadersPath: headerPath, runtimeHeadersSha256: sha256(runtimeHeaders),
        node: process.version, wrangler: wranglerPackage.version, wranglerRepositoryPin: pinnedVersion,
        compatibilityDate: runtimeConfig.compatibility_date,
        method: 'Local Wrangler static-assets HTTP response with Accept-Encoding: br; no deployment',
    };
    // Exclusive creates prevent silently replacing an earlier measurement.
    await writeFile(output, response.body, { flag: 'wx' }); wroteOutput = true;
    await writeFile(output + '.json', JSON.stringify(facts, null, 2) + '\n', { flag: 'wx' }); wroteMetadata = true;
    console.log(JSON.stringify(facts, null, 2));
} catch (error) {
    if (wroteOutput && !wroteMetadata) await rm(output);
    throw new Error(`${signal.aborted ? signal.reason : error}\nWrangler diagnostics:\n${logs || '(no output)'}`, { cause: error });
} finally {
    clearTimeout(timeout);
    try { await stopChild(); }
    finally {
        if (scratch) await rm(scratch, { recursive: true, force: true });
        process.off('SIGINT', onInterrupt); process.off('SIGTERM', onTerminate);
    }
}
