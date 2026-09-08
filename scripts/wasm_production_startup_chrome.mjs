#!/usr/bin/env node
// Run `pnpm --dir wasm-www build:site` first. This serves that unchanged production
// shell and the actual /wasm/<hash> and /datadirs/... URL layout locally.
// Example: node scripts/wasm_production_startup_chrome.mjs --pkg /tmp/pkg \
//   --datadir /tmp/corpus --output /tmp/startup --mbit 16
// Repeat warm loads with --repeat-replay FILE (repeatable; same package build).
// Default mission=auto exercises normal production demo launch (correct demo team).
// Explicit Dem_Lei_MP takes a different forced-mission path with a different team.
// Endpoint: bootstrap and screenshot-after-two-rAF plus 500ms capture settle, NOT physical presentation.
// --require-present also waits for Rust's first mission present-return marker.
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { resolve, join, extname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { gzipSync, brotliDecompressSync } from 'node:zlib';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';
import { SharedBandwidth } from './startup_throttle.mjs';
const { values } = parseArgs({ options: {
    pkg: { type: 'string' }, datadir: { type: 'string' }, output: { type: 'string' },
    site: { type: 'string', default: 'wasm-www/dist' }, core: { type: 'string', default: 'assets/core-datadir' },
    mbit: { type: 'string', default: '16' }, chrome: { type: 'string', default: 'google-chrome' },
    mission: { type: 'string', default: 'auto' }, 'require-present': { type: 'boolean', default: false },
    trace: { type: 'boolean', default: false }, 'cpu-profile': { type: 'boolean', default: false },
    'http-wasm-br': { type: 'string' }, 'http-admission-br': { type: 'string' },
    replay: { type: 'string' },
    'repeat-replay': { type: 'string', multiple: true, default: [] },
    query: { type: 'string', multiple: true, default: [] },
} });
if (!values.pkg || !values.datadir || !values.output) throw new Error('--pkg, --datadir and --output are required');
const pkg = resolve(values.pkg), datadir = resolve(values.datadir), site = resolve(values.site), core = resolve(values.core);
const outputBase = resolve(values.output);
let output = outputBase;
const replayContent = values.replay ? (await readFile(resolve(values.replay), 'utf8')).trim() : undefined;
const replayBuild = replayContent?.match(/^rhrec-([0-9a-f]{12})-/)?.[1];
if (replayContent !== undefined && !replayBuild) throw new Error('--replay must contain a compact rhrec replay');
if (replayContent !== undefined && values.mission !== 'auto') throw new Error('Replay header must select the mission; omit --mission');
// Reuse one origin and Chrome profile: query changes must not change asset identity.
// Each repeat is an actual admitted replay, not a synthetic asset-only request.
const replayRuns = [{ path: values.replay, content: replayContent }];
for (const path of values['repeat-replay']) {
    if (!replayBuild) throw new Error('--repeat-replay requires --replay');
    const content = (await readFile(resolve(path), 'utf8')).trim();
    if (content.match(/^rhrec-([0-9a-f]{12})-/)?.[1] !== replayBuild) {
        throw new Error('Repeated replay must use the supplied package build identity');
    }
    replayRuns.push({ path, content });
}
if (replayRuns.length > 1 && values.trace) throw new Error('--trace cannot be combined with --repeat-replay');
const repeatResults = [];
const rate = values.mbit === 'unlimited' ? null : Number(values.mbit) * 1_000_000 / 8;
const throttle = rate === null ? null : new SharedBandwidth(rate);
const records = [], logs = [], errors = [];
const cache = new Map();
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const httpWasmBr = values['http-wasm-br'] ? await readFile(resolve(values['http-wasm-br'])) : undefined;
if (httpWasmBr && !brotliDecompressSync(httpWasmBr).equals(await readFile(join(pkg, 'robin_bg.wasm')))) {
    throw new Error('--http-wasm-br does not decode to the supplied package WASM');
}
const httpAdmissionBr = values['http-admission-br'] ? await readFile(resolve(values['http-admission-br'])) : undefined;
if (httpAdmissionBr && !brotliDecompressSync(httpAdmissionBr).equals(await readFile(join(pkg, 'replay_admission_bg.wasm')))) {
    throw new Error('--http-admission-br does not decode to the supplied package admission WASM');
}
const hash = replayBuild ?? '000000000000'; // Replay builds retain their real envelope identity.
const runtimePrefix = `/wasm/${hash}/`;
const dataPrefix = '/datadirs/demo-leicester/';
const preload = [];
const { readdir } = await import('node:fs/promises');
preload.push({ path: 'Data/Interface/Fonts/arial.ttf', url: 'Data/Interface/Fonts/arial.ttf' });
for (const name of (await readdir(join(core, 'Data/Interface/UI'))).sort()) {
    if (name.endsWith('.png')) preload.push({ path: `Data/Interface/UI/${name}`, url: `Data/Interface/UI/${name}` });
}
function category(path) {
    if (path.endsWith('.wasm.gz') || path.endsWith('.wasm.br') || path.endsWith('.wasm')) return 'wasm';
    if (path.endsWith('.rhdata.zst')) return 'boot';
    if (path.includes('/audio/')) return 'audio';
    if (path.includes('/terrain/')) return 'terrain';
    if (path.endsWith('.rhmission.zst')) return 'mission-parts';
    if (path.includes('/Data/Interface/')) return 'interface';
    if (path.endsWith('.js')) return 'javascript';
    return 'shell-and-other';
}
function safePath(root, path) {
    const result = resolve(root, path);
    if (!result.startsWith(root + sep)) throw new Error('path outside fixture');
    return result;
}
async function asset(path) {
    if (cache.has(path)) return cache.get(path);
    let body, type = 'application/octet-stream', encoding;
    if (path === '/wasm/latest.json') {
        body = Buffer.from(JSON.stringify({ short: hash })); type = 'application/json';
    } else if (path === runtimePrefix + 'preload-assets.json') {
        body = Buffer.from(JSON.stringify(preload)); type = 'application/json';
    } else {
        let file;
        if (path.startsWith(runtimePrefix)) {
            const suffix = path.slice(runtimePrefix.length);
            file = safePath(suffix.startsWith('Data/') ? core : pkg, suffix);
            if (suffix === 'robin_bg.wasm.gz') body = execFileSync('gzip', ['-9', '-n', '-c', join(pkg, 'robin_bg.wasm')], { maxBuffer: 256 * 1024 * 1024 });
            if (suffix === 'robin_bg.wasm' && httpWasmBr) { body = httpWasmBr; encoding = 'br'; }
            if (suffix === 'replay_admission_bg.wasm' && httpAdmissionBr) { body = httpAdmissionBr; encoding = 'br'; }
        } else if (path.startsWith(dataPrefix)) {
            const suffix = path.slice(dataPrefix.length);
            file = safePath(datadir, suffix === 'v8-web-opus-q80.rhdata.zst' ? 'Data/datadir.bin' : `Data/${suffix}`);
        } else file = safePath(site, path === '/' ? 'index.html' : path.slice(1));
        body ??= await readFile(file);
        type = ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.png': 'image/png', '.svg': 'image/svg+xml' })[extname(file)] ?? type;
    }
    // Explicit .wasm.{gz,br} siblings are raw compressed objects. Ordinary
    // .wasm optionally uses a captured, hash-verified HTTP Brotli response.
    // JS/HTML/CSS use HTTP content encoding; other compressed assets stay intact.
    if (['text/html', 'text/javascript', 'text/css', 'application/json', 'image/svg+xml'].includes(type)) {
        body = gzipSync(body, { level: 9 }); encoding = 'gzip';
    }
    const result = { body, type, encoding }; cache.set(path, result); return result;
}
// Precompute compression before navigation, keeping server CPU out of startup.
await asset('/'); await asset('/wasm/latest.json'); await asset(runtimePrefix + 'robin_bg.wasm.gz');
for (const path of ['robin.js', 'preload-assets.json']) await asset(runtimePrefix + path);
const server = createServer(async (req, res) => {
    const path = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    const record = { path, requestedAt: performance.now(), bytes: 0, chunks: [] };
    records.push(record);
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    const immutable = path.startsWith(runtimePrefix) || path.startsWith(dataPrefix) || path.startsWith('/assets/');
    res.setHeader('Cache-Control', immutable ? 'public, max-age=31536000, immutable' : 'public, max-age=0, must-revalidate');
    try {
        const { body, type, encoding } = await asset(path);
        record.payloadBytes = body.length;
        res.setHeader('Content-Type', type); res.setHeader('Content-Length', body.length);
        if (encoding) res.setHeader('Content-Encoding', encoding);
        const onChunk = (bytes, at) => { record.bytes += bytes; record.chunks.push({ bytes, at }); };
        if (throttle) await throttle.send(res, body, onChunk);
        else { res.end(body); onChunk(body.length, performance.now()); }
        record.finishedAt = performance.now();
    } catch (error) {
        record.error = String(error); res.writeHead(404); res.end(String(error));
    }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const profile = await mkdtemp(join(tmpdir(), 'robin-production-startup-'));
let browser, socket;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let browserErrors = '';
let bootstrapEpoch, presentEpoch, replayState;
try {
    browser = spawn(values.chrome, ['--headless=new', `--user-data-dir=${profile}`, '--no-first-run', '--enable-unsafe-swiftshader', '--autoplay-policy=no-user-gesture-required', '--remote-debugging-port=0', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
    browser.stderr.on('data', data => { browserErrors += data; });
    let target;
    for (let i = 0; i < 300 && !target; i++) {
        try {
            const port = (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0];
            target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === 'page');
        } catch {}
        if (!target) await sleep(50);
    }
    if (!target) throw new Error('Chrome debugger unavailable: ' + browserErrors);
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    let id = 0;
    const pending = new Map();
    const send = (method, params = {}) => new Promise((resolve, reject) => {
        pending.set(++id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params }));
    });
    let finishTrace;
    const traceComplete = new Promise(resolve => { finishTrace = resolve; });
    socket.addEventListener('message', event => {
        const message = JSON.parse(event.data);
        if (message.id) {
            const request = pending.get(message.id); pending.delete(message.id);
            if (message.error) request.reject(new Error(JSON.stringify(message.error))); else request.resolve(message.result);
        } else if (message.method === 'Tracing.tracingComplete') {
            finishTrace(message.params);
        } else if (message.method === 'Runtime.consoleAPICalled') {
            const p = message.params;
            const line = p.args.map(arg => arg.value ?? arg.description ?? '').join(' ').replaceAll('%c', '');
            logs.push({ epochMs: p.timestamp, line });
            console.log(line);
            if (line.includes('mission bootstrap: total elapsed_ms')) bootstrapEpoch ??= p.timestamp;
            if (line.includes('startup timing: first mission present returned')) presentEpoch ??= p.timestamp;
        } else if (message.method === 'Runtime.exceptionThrown') errors.push(message.params);
    });
    await send('Runtime.enable'); await send('Page.enable');
    await send('Emulation.setDeviceMetricsOverride', { width: 1024, height: 768, deviceScaleFactor: 1, mobile: false });
    await send('Emulation.setHardwareConcurrencyOverride', { hardwareConcurrency: 4 });
    for (const [runIndex, replayRun] of replayRuns.entries()) {
        if (runIndex > 0) {
            await send('Page.navigate', { url: 'about:blank' });
            await sleep(500); // Dispose the prior game's workers before resetting observations.
        }
        output = replayRuns.length === 1 ? outputBase : `${outputBase}-${runIndex}`;
        records.length = 0; logs.length = 0; errors.length = 0;
        bootstrapEpoch = undefined; presentEpoch = undefined; replayState = undefined;
        const query = new URLSearchParams({ mission: values.mission, 'wasm-threads': '4', 'wasm-log': 'info' });
        if (values.mission === 'auto') query.delete('mission');
        for (const value of values.query) { const at = value.indexOf('='); if (at < 1) throw new Error('--query requires KEY=VALUE'); query.set(value.slice(0, at), value.slice(at + 1)); }
        if (replayContent !== undefined) { query.set('replay', replayRun.content); query.set('paused', '0'); }
        if (values.trace) await send('Tracing.start', {
            categories: 'devtools.timeline,blink.user_timing,v8,gpu,disabled-by-default-v8.cpu_profiler',
            transferMode: 'ReturnAsStream',
        });
        if (values['cpu-profile']) {
            await send('Profiler.enable');
            await send('Profiler.start');
        }

        await send('Page.navigate', { url: `http://127.0.0.1:${server.address().port}/?${query}` });
        const deadline = Date.now() + 180000;
        while ((!bootstrapEpoch || ((values['require-present'] || replayContent !== undefined) && !presentEpoch)) && Date.now() < deadline && !errors.length) await sleep(20);
        if (!bootstrapEpoch || ((values['require-present'] || replayContent !== undefined) && !presentEpoch)) throw new Error('Startup did not reach required endpoint: ' + JSON.stringify(errors));
        if (replayContent !== undefined) {
            if (!logs.some(({ line }) => line.includes('Loaded replay (decoded):'))) {
                throw new Error('Bootstrap completed without decoded replay playback');
            }
            const state = await send('Runtime.evaluate', {
                expression: 'globalThis.robinRpc("state")', awaitPromise: true, returnByValue: true,
            });
            if (state.exceptionDetails) throw new Error('Replay state RPC failed: ' + JSON.stringify(state.exceptionDetails));
            replayState = state.result.value;
            if (!replayState?.replay) throw new Error('Replay playback state is missing: ' + JSON.stringify(replayState));
        }
        const probe = await send('Runtime.evaluate', { expression: `new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve({timeOrigin:performance.timeOrigin, screenshotRequestAt:performance.now(), canvas:{width:document.querySelector('#canvas').width,height:document.querySelector('#canvas').height}, resources:performance.getEntriesByType('resource').map(e=>e.toJSON()), marks:performance.getEntriesByType('mark').map(e=>e.toJSON())}))))`, awaitPromise: true, returnByValue: true });
        const page = probe.result.value;
        await sleep(500);
        const screenshotStart = performance.now();
        const screenshot = await send('Page.captureScreenshot', { format: 'png', fromSurface: true });
        const screenshotEnd = performance.now();
        await writeFile(output + '.png', Buffer.from(screenshot.data, 'base64'));
        if (errors.length) throw new Error('Browser exception during startup/capture: ' + JSON.stringify(errors));
        if (values['cpu-profile']) {
            const { profile } = await send('Profiler.stop');
            await writeFile(output + '.cpuprofile', JSON.stringify(profile));
        }
        if (values.trace) {
            await send('Tracing.end');
            let timer;
            const { stream } = await Promise.race([
                traceComplete,
                new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Chrome trace completion timed out')), 30000); }),
            ]).finally(() => clearTimeout(timer));
            if (!stream) throw new Error('Chrome trace completed without a stream');
            const chunks = [];
            for (;;) {
                const chunk = await send('IO.read', { handle: stream });
                chunks.push(Buffer.from(chunk.data, chunk.base64Encoded ? 'base64' : 'utf8'));
                if (chunk.eof) break;
            }
            await send('IO.close', { handle: stream });
            await writeFile(output + '.trace.json', Buffer.concat(chunks));
        }
        const navigationServerAt = page.timeOrigin - performance.timeOrigin;
        const result = {
            inputs: { wasmSha256: sha256(await readFile(join(pkg, 'robin_bg.wasm'))), wasmGzipSha256: sha256((await asset(runtimePrefix + 'robin_bg.wasm.gz')).body), bootSha256: sha256(await readFile(join(datadir, 'Data/datadir.bin'))), siteIndexSha256: sha256(await readFile(join(site, 'index.html'))) },
            httpAdmissionBrotli: httpAdmissionBr ? { path: resolve(values['http-admission-br']), bytes: httpAdmissionBr.length, sha256: sha256(httpAdmissionBr), rawSha256: sha256(await readFile(join(pkg, 'replay_admission_bg.wasm'))), caveat: 'Supplied encoded fixture is verified against admission package bytes; retain capture provenance separately.' } : null,
            httpWasmBrotli: httpWasmBr ? { path: resolve(values['http-wasm-br']), bytes: httpWasmBr.length, sha256: sha256(httpWasmBr), caveat: 'Supplied encoded fixture is verified against package bytes; retain capture provenance separately.' } : null,
            replay: replayContent === undefined ? null : { path: resolve(replayRun.path), sha256: sha256(Buffer.from(replayRun.content)), build: replayBuild, state: replayState },
            pkg, datadir, site, mission: values.mission, query: [...query], browser: await send('Browser.getVersion'),
            diagnostics: { trace: values.trace, cpuProfile: values['cpu-profile'], caveat: 'Optional profiling adds overhead; use uninstrumented runs for timing comparisons.' },
            network: { mbit: rate === null ? 'unlimited' : Number(values.mbit), scope: throttle ? 'single shared server queue for all response payloads including worker fetches' : 'unshaped loopback responses', chunkBytes: throttle ? 16384 : null, latencyMs: 0, compression: 'gzip -9 -n CLI for raw explicit wasm.gz sibling; Node gzip level9 HTTP encoding for text', cache: runIndex === 0 ? 'fresh browser profile; normal intra-navigation HTTP caching' : 'same browser profile and origin; normal HTTP cache reuse', caveat: 'HTTP/1.1 loopback, no TCP overhead or packet loss; cumulative deadlines avoid per-chunk timer-rounding loss'  },
            endpoints: { bootstrapMs: bootstrapEpoch - page.timeOrigin, firstMissionPresentReturnedMs: presentEpoch ? presentEpoch - page.timeOrigin : null, afterTwoRafMs: page.screenshotRequestAt, screenshotRequestMs: screenshotStart - navigationServerAt, screenshotCompleteMs: screenshotEnd - navigationServerAt, screenshotSettleMs: 500, screenshotServerDurationMs: screenshotEnd - screenshotStart, caveat: 'Screenshot after bootstrap, two animation callbacks and 500ms settle is an inspectable image, not a physical display presentation timestamp. present returned is submission-side only.' },
            page, errors, logs: logs.map(({ epochMs, line }) => ({ pageMs: epochMs - page.timeOrigin, line })),
            requests: records.map(record => ({ ...record, requestedAt: record.requestedAt - navigationServerAt, finishedAt: record.finishedAt === undefined ? null : record.finishedAt - navigationServerAt, category: category(record.path), chunks: record.chunks.map(chunk => ({ ...chunk, at: chunk.at - navigationServerAt })) })),
        };
        result.payloadBytesAtBootstrap = {};
        for (const request of result.requests) {
            const bytes = request.chunks.filter(chunk => chunk.at <= result.endpoints.bootstrapMs).reduce((n, chunk) => n + chunk.bytes, 0);
            result.payloadBytesAtBootstrap[request.category] = (result.payloadBytesAtBootstrap[request.category] ?? 0) + bytes;
        }
        result.bytesAtBootstrap = Object.values(result.payloadBytesAtBootstrap).reduce((sum, n) => sum + n, 0);
        await writeFile(output + '.json', JSON.stringify(result, null, 2));
        console.log(JSON.stringify(result.endpoints));
        repeatResults.push({ runIndex, output, replayPath: replayRun.path,
            endpoints: result.endpoints, payloadBytesAtBootstrap: result.payloadBytesAtBootstrap,
            bytesAtBootstrap: result.bytesAtBootstrap, serverRequests: result.requests.length,
            resources: result.page.resources.map(({ name, transferSize, encodedBodySize, decodedBodySize }) => ({ name, transferSize, encodedBodySize, decodedBodySize })),
        });
    }
    if (replayRuns.length > 1) await writeFile(outputBase + '.repeat.json', JSON.stringify(repeatResults, null, 2));
} catch (error) {
    await writeFile(output + '.failure.json', JSON.stringify({ error: String(error), errors, logs, records, browserErrors, bootstrapEpoch, presentEpoch, replayState, replayPath: values.replay ? resolve(values.replay) : null }, null, 2));
    throw error;
} finally {
    socket?.close();
    if (browser && browser.exitCode === null) { browser.kill('SIGKILL'); await new Promise(resolve => browser.once('exit', resolve)); }
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
