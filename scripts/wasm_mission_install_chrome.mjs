// End-to-end browser check for the streaming shipping-mission installer.
//
// Boots the REAL game build (wasm-www/pkg, from scripts/build-wasm-threads.sh)
// in headless Chrome behind a loopback COOP/COEP server, launches straight
// into a mission via the `?mission=` query, and relays the page's console to
// stdout. Success is the game's own "activated shipping mission" install
// line; the run also prints the wall time from wasm_boot to installation, so
// fetch+decode overlap improvements show up directly. --wait-ingame also
// reports navigation-to-game, including module fetch/compile and preloads.
// Timestamps originate in the browser, excluding the console relay delay.
// --cpu-profile <file> records the main-thread Chrome CPU profile from
// navigation through the finish marker (use an unstripped wasm for names).
//
//   node scripts/wasm_mission_install_chrome.mjs <converted-datadir-root> \
//       [--mission H01_Lin_VL] [--pkg wasm-www/pkg] [--serial] [--chrome BIN]
//
// --verify-restart FILE drives real terminal UI clicks, checks two checkpoint
// restores by default, and exports the compact replay string to FILE.
// --restart-cycles N changes that count. This mode uses a fixed CDP viewport;
// ordinary startup measurements keep their original Chrome viewport.
// --timings FILE saves browser-clock log timestamps and Resource Timing entries.
// Worker-local resource entries are not included in the main-window buffer.
// --serial withholds the COOP/COEP headers, so crossOriginIsolated is false
// and the game exercises the no-worker-pool fallback of the same build.
import { createServer } from 'node:http';
import { readFileSync, existsSync, mkdtempSync, rmSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve, extname, normalize } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';

const args = process.argv.slice(2);
const positional = [];
let mission = 'H01_Lin_VL';
let pkgDir = 'wasm-www/pkg';
let chromeBin = 'google-chrome';
let isolated = true;
let waitIngame = false;
let waitAudio = false;
let cpuProfile = null;
let timingsFile = null;
let failedRequest = null;
let wasmLog = 'info';
let restartReplay = null;
let restartCycles = 2;
for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--mission') mission = args[++i];
    else if (arg === '--pkg') pkgDir = args[++i];
    else if (arg === '--chrome') chromeBin = args[++i];
    else if (arg === '--serial') isolated = false;
    else if (arg === '--wait-ingame') waitIngame = true;
    else if (arg === '--wait-audio') waitAudio = true;
    else if (arg === '--cpu-profile') cpuProfile = args[++i];
    else if (arg === '--timings') timingsFile = args[++i];
    else if (arg === '--fail-request') failedRequest = args[++i];
    else if (arg === '--wasm-log') wasmLog = args[++i];
    else if (arg === '--verify-restart') restartReplay = args[++i];
    else if (arg === '--restart-cycles') restartCycles = Number(args[++i]);
    else positional.push(arg);
}
const [root] = positional;
if (!root) {
    console.error(
        'usage: node scripts/wasm_mission_install_chrome.mjs <converted-datadir-root> ' +
        '[--mission NAME] [--pkg DIR] [--serial] [--chrome BIN] [--wait-ingame] [--wait-audio] [--cpu-profile FILE] [--timings FILE] [--fail-request URL_PATH] [--wasm-log LEVEL] [--verify-restart REPLAY_FILE] [--restart-cycles N]',
    );
    process.exit(2);
}

if (!Number.isSafeInteger(restartCycles) || restartCycles < 1) {
    console.error('--restart-cycles must be a positive integer');
    process.exit(2);
}

const MIME = {
    '.js': 'text/javascript',
    '.wasm': 'application/wasm',
    '.html': 'text/html',
    '.json': 'application/json',
    '.png': 'image/png',
    '.ttf': 'font/ttf',
};

// Same overlay set the publish workflow preloads.
const coreRoot = resolve('assets/core-datadir');
const preloadPaths = [
    'Data/Interface/Fonts/arial.ttf',
    ...readdirSync(join(coreRoot, 'Data/Interface/UI'))
        .filter((f) => f.endsWith('.png'))
        .map((f) => `Data/Interface/UI/${f}`),
];

const page = `<!DOCTYPE html>
<meta charset="utf-8">
<style>html,body { margin: 0; } canvas { display: block; width: 1024px; height: 768px; }</style>
<body>
<canvas id="canvas" width="1024" height="768"></canvas>
<script>
// Relay the console (tracing-wasm writes there) to the harness.
const scriptStartedAt = performance.now();
const relay = [];
const collectTimings = ${JSON.stringify(Boolean(timingsFile))};
const audioDecodes = [];
let restartRestored = false;
let restartFailed = false;
let terminalFailures = 0;
let restartCheckStarted = false;
if (collectTimings) {
    performance.setResourceTimingBufferSize(10000);
    const originalDecode = BaseAudioContext.prototype.decodeAudioData;
    BaseAudioContext.prototype.decodeAudioData = function (...args) {
        const span = { start: performance.now(), bytes: args[0].byteLength };
        audioDecodes.push(span);
        const result = originalDecode.apply(this, args);
        result.then(
            (buffer) => {
                span.end = performance.now();
                span.ok = true;
                span.pcmBytes = buffer.length * buffer.numberOfChannels * 4;
            },
            () => { span.end = performance.now(); span.ok = false; },
        );
        return result;
    };
}
let relayTimer = null;
const post = (line) => {
    if (${JSON.stringify(Boolean(restartReplay))}) {
        if (line.includes('Restart snapshot restored')) restartRestored = true;
        if (line.includes('Restart snapshot could not be restored')) restartFailed = true;
        if (line.includes('Engine tick returned: LevelFailed')) terminalFailures++;
        if (!restartCheckStarted && line.includes('mission bootstrap: total elapsed_ms')) {
            restartCheckStarted = true;
            setTimeout(() => globalThis.runRestartCheck().catch(error => console.error('restart verification failed: ' + error)), 0);
        }
    }
    relay.push({ line, pageMs: performance.now() });
    if (collectTimings && (line.includes('Recording replay') || line.includes('background mission audio warmup complete'))) {
        void fetch('/timings', {
            method: 'POST',
            body: JSON.stringify({
                reason: line.includes('Recording replay') ? 'recording' : 'audio complete',
                capturedAt: performance.now(),
                scriptStartedAt,
                navigation: performance.getEntriesByType('navigation').map((entry) => entry.toJSON()),
                audioDecodes,
                resources: performance.getEntriesByType('resource').map((entry) => entry.toJSON()),
            }),
        });
    }
    if (relayTimer === null) {
        relayTimer = setTimeout(() => {
            relayTimer = null;
            void fetch('/log', { method: 'POST', body: JSON.stringify(relay.splice(0)) });
        }, 50);
    }
};
for (const m of ['log', 'info', 'warn', 'error']) {
    const orig = console[m].bind(console);
    console[m] = (...a) => {
        orig(...a);
        post(m + ': ' + a.map((x) => {
            if (typeof x === 'string') return x.replaceAll('%c', '');
            try { return JSON.stringify(x); } catch { return String(x); }
        }).join(' '));
    };
}
addEventListener('error', (e) => post('pageerror: ' + e.message));
addEventListener('unhandledrejection', (e) => post('pageerror: ' + e.reason));
</script>
<script type="module">
const preloadPaths = ${JSON.stringify(preloadPaths)};
try {
    console.log('harness: isolated=' + crossOriginIsolated);
    console.log('startup: module import begin');
    const glue = await import('/pkg/robin.js');
    await glue.default({ module_or_path: '/pkg/robin_bg.wasm' });
    console.log('startup: module ready; core preloads begin');
    for (const path of preloadPaths) {
        const resp = await fetch('/core/' + path);
        if (!resp.ok) throw new Error('preload ' + path + ': HTTP ' + resp.status);
        glue.wasm_preload_asset(path, new Uint8Array(await resp.arrayBuffer()));
    }
    console.log('startup: core preloads complete; boot fetch begin');
    const datadir = new Uint8Array(await (await fetch('/data/Data/datadir.bin')).arrayBuffer());
    globalThis.runRestartCheck = async () => {
        const delay = (ms = 50) => new Promise(resolve => setTimeout(resolve, ms));
        const rpc = (method, params = null) => {
            let timer;
            const timeout = new Promise((_, reject) => {
                timer = setTimeout(() => reject(new Error('RPC timeout: ' + method)), 15000);
            });
            return Promise.race([glue.rh_rpc({ method, params }), timeout])
                .finally(() => clearTimeout(timer));
        };
        const cdp = async (method, params) => {
            const response = await fetch('/restart-input', {
                method: 'POST', body: JSON.stringify({ method, params }),
            });
            const result = await response.json();
            if (!response.ok) throw new Error(JSON.stringify(result));
        };
        const waitFor = async (predicate, label) => {
            const deadline = performance.now() + 15000;
            while (!predicate()) {
                if (restartFailed) throw new Error('Restart fell back to mission reconstruction');
                if (performance.now() >= deadline) throw new Error('Timed out waiting for ' + label);
                await delay();
            }
        };
        const evidence = [];
        for (let cycle = 0; cycle < ${JSON.stringify(restartCycles)}; cycle++) {
            restartRestored = false;
            const initial = await rpc('state');
            // A no-op seek dismisses startup popups without advancing simulation.
            // Manual forward ticks bypass the cooperative terminal UI, so force
            // defeat on a normal outer frame instead.
            await rpc('go-to-frame', { frame: initial.frame, auto_dismiss: true });
            await rpc('console', { command: 'LOOSE' });
            const previousFailures = terminalFailures;
            await rpc('set-paused', { paused: false });
            await waitFor(() => terminalFailures > previousFailures, 'natural terminal frame');
            await delay(2500); // Mission-state opening transition must finish first.
            await cdp('Input.dispatchKeyEvent', {
                type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13,
            });
            await delay(100);
            await cdp('Input.dispatchKeyEvent', {
                type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13,
            });
            await delay(1500); // Allow the final debrief page to take ownership.
            const before = await rpc('set-paused', { paused: true });
            // Restart's seal is at (122,392) in the centered 640x480 menu;
            // its center is (142,412). Use CSS pixels for CDP input.
            const rect = document.querySelector('canvas').getBoundingClientRect();
            const x = rect.left + rect.width / 2 - 178;
            const y = rect.top + rect.height / 2 + 172;
            await cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
            await cdp('Input.dispatchMouseEvent', {
                type: 'mousePressed', x, y, button: 'left', clickCount: 1,
            });
            await delay(100);
            await cdp('Input.dispatchMouseEvent', {
                type: 'mouseReleased', x, y, button: 'left', clickCount: 1,
            });
            await waitFor(() => restartRestored, 'UI Restart checkpoint restore');
            const restored = await rpc('state');
            if (restartFailed || restored.frame !== 0 || before.frame <= restored.frame) {
                throw new Error('Restart did not restore initial timeline: ' + JSON.stringify({ before, restored }));
            }
            // Leave the last restore paused for export; the next cycle explicitly
            // resumes and defeats the restored attempt to exercise its lifecycle.
            await rpc('set-paused', { paused: true });
            evidence.push({ before: before.frame, restored: restored.frame });
        }
        // Finish the final restored attempt so its new recorder contains an
        // admitted frame, rather than exporting a header with pending metadata.
        const finalFailures = terminalFailures;
        await rpc('go-to-frame', { frame: 0, auto_dismiss: true });
        await rpc('console', { command: 'LOOSE' });
        await rpc('set-paused', { paused: false });
        await waitFor(() => terminalFailures > finalFailures, 'final restored attempt completion');
        await rpc('set-paused', { paused: true });
        const replay = await rpc('get-replay');
        if (typeof replay.content !== 'string' || !replay.content.startsWith('rhrec-')) {
            throw new Error('Unexpected replay export shape');
        }
        const saved = await fetch('/restart-replay', { method: 'POST', body: replay.content });
        if (!saved.ok) throw new Error('Could not export Restart replay');
        console.log('harness: restart verified ' + JSON.stringify({ cycles: evidence }));
    };
    console.log('harness: boot t0');
    glue.wasm_boot(datadir, '/data/Data');
} catch (e) {
    console.error('harness boot failed: ' + (e && e.stack || e));
}
</script>`;

const timingLogs = [];
let resourceTimings = null;
const resourceSnapshots = [];
let audioExpected = false;
let audioDone = false;
let bootAt = null;
let done = false;
let activatedAt = null;
let inGameAt = null;
let bootstrapAt = null;
let restartVerified = false;
// Lazy character-chunk streaming: activation can precede the deferred
// sprite-decode tail. When the install announces a deferred tail, keep the
// page alive until the tail's completion line so its duration is measured.
let tailExpected = false;
let tailDone = false;
const maybeFinish = () => {
    if (done || activatedAt === null) return;
    // Mission install is the default finish line; --wait-ingame keeps the
    // run alive through session bootstrap so its PhaseTimer spans land too.
    if ((waitIngame || restartReplay) && inGameAt === null) return;
    if (restartReplay && !restartVerified) return;
    if (tailExpected && !tailDone) return;
    if (waitAudio && audioExpected && !audioDone) return;
    done = true;
    // Give trailing logs a moment, then finish.
    setTimeout(() => finish(0), 1500);
};
let devtoolsSend = null;
const server = createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    if (isolated) {
        res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
        res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    }
    res.setHeader('Cache-Control', 'no-store');
    if (restartReplay && req.method === 'POST' && url.pathname === '/restart-input') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', async () => {
            try {
                const { method, params } = JSON.parse(body);
                if (!['Input.dispatchKeyEvent', 'Input.dispatchMouseEvent'].includes(method)) {
                    throw new Error('Unsupported Restart input method');
                }
                if (devtoolsSend === null) throw new Error('Chrome input session is not ready');
                await devtoolsSend(method, params);
                res.end('{}');
            } catch (error) {
                res.statusCode = 500;
                res.end(JSON.stringify({ error: String(error) }));
            }
        });
        return;
    }
    if (restartReplay && req.method === 'POST' && url.pathname === '/restart-replay') {
        const chunks = [];
        req.on('data', chunk => chunks.push(chunk));
        req.on('end', () => {
            writeFileSync(restartReplay, Buffer.concat(chunks));
            res.end('ok');
        });
        return;
    }
    if (req.method === 'POST' && url.pathname === '/timings') {
        let body = '';
        req.on('data', (chunk) => { body += chunk; });
        req.on('end', () => {
            const snapshot = JSON.parse(body);
            resourceSnapshots.push(snapshot);
            if (snapshot.reason === 'recording') resourceTimings = snapshot;
            res.end('ok');
        });
        return;
    }
    if (req.method === 'POST' && url.pathname === '/log') {
        let body = '';
        req.on('data', (c) => { body += c; });
        req.on('end', () => {
            res.end('ok');
            for (const { line, pageMs } of JSON.parse(body)) {
                if (timingsFile) timingLogs.push({ line, pageMs });
                console.log(`[page] ${line}`);
                if (line.includes('background mission audio warmup started')) audioExpected = true;
                if (line.includes('background mission audio warmup complete')) audioDone = true;
                if (waitAudio && line.includes('background mission audio warmup failed')) {
                    void finish(1);
                    return;
                }
                if (restartReplay && (line.includes('restart verification failed:') || line.includes('panicked at') || line.startsWith('pageerror:'))) {
                    void finish(1);
                    return;
                }
                if (line.includes('harness: restart verified')) restartVerified = true;
                if (line.includes('boot t0')) bootAt = pageMs;
                const secs = () => ((pageMs - bootAt) / 1000).toFixed(3);
                if (line.includes('activated shipping mission') && bootAt !== null
                    && activatedAt === null) {
                    activatedAt = pageMs;
                    console.log(`RESULT: mission installed ${secs()}s after wasm_boot`);
                }
                if (line.includes('deferred sprite chunks')) tailExpected = true;
                if (line.includes('background sprite streaming complete') && bootAt !== null
                    && !tailDone) {
                    tailDone = true;
                    console.log(`RESULT: sprite streaming tail complete ${secs()}s after wasm_boot`);
                }
                if (line.includes('Recording replay') && bootAt !== null && inGameAt === null) {
                    inGameAt = pageMs;
                    console.log(`RESULT: in-game (recording replay) ${secs()}s after wasm_boot; ${(pageMs / 1000).toFixed(3)}s after navigation`);
                }
                if (line.includes('mission bootstrap: total elapsed_ms') && bootstrapAt === null) {
                    bootstrapAt = pageMs;
                    console.log(`RESULT: bootstrap complete ${(pageMs / 1000).toFixed(3)}s after navigation`);
                }
                maybeFinish();
            }
        });
        return;
    }
    let filePath = null;
    if (url.pathname === '/') {
        res.writeHead(200, { 'Content-Type': 'text/html' });
        res.end(page);
        return;
    }
    if (url.pathname.startsWith('/pkg/')) {
        filePath = join(resolve(pkgDir), normalize(url.pathname.slice(5)));
    } else if (url.pathname.startsWith('/data/')) {
        filePath = join(resolve(root), normalize(url.pathname.slice(6)));
    } else if (url.pathname.startsWith('/core/')) {
        filePath = join(coreRoot, normalize(url.pathname.slice(6)));
    }
    if (url.pathname === failedRequest || filePath === null || !existsSync(filePath)) {
        res.writeHead(404, { 'Content-Type': 'text/plain' });
        res.end(`not found: ${url.pathname}`);
        return;
    }
    res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] ?? 'application/octet-stream' });
    res.end(readFileSync(filePath));
});

// Attach before navigation for startup profiling or trusted Restart UI input.
async function startCpuProfile(profileDir, pageUrl) {
    const deadline = Date.now() + 15000;
    let target;
    while (Date.now() < deadline) {
        const portFile = join(profileDir, 'DevToolsActivePort');
        if (existsSync(portFile)) {
            const port = readFileSync(portFile, 'utf8').split('\n')[0];
            const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
            target = targets.find((entry) => entry.type === 'page');
            if (target) break;
        }
        await new Promise((resolve) => setTimeout(resolve, 50));
    }
    if (!target) throw new Error('Chrome did not expose a page for CPU profiling');
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
        socket.addEventListener('open', resolve, { once: true });
        socket.addEventListener('error', reject, { once: true });
    });
    let nextId = 0;
    const pending = new Map();
    socket.addEventListener('message', ({ data }) => {
        const message = JSON.parse(data);
        const request = pending.get(message.id);
        if (!request) return;
        pending.delete(message.id);
        if (message.error) request.reject(new Error(JSON.stringify(message.error)));
        else request.resolve(message.result);
    });
    socket.addEventListener('close', () => {
        for (const request of pending.values()) request.reject(new Error('Chrome debugger closed'));
        pending.clear();
    });
    const send = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++nextId;
        pending.set(id, { resolve, reject });
        socket.send(JSON.stringify({ id, method, params }));
    });
    devtoolsSend = send;
    if (cpuProfile) {
        await send('Profiler.enable');
        await send('Profiler.setSamplingInterval', { interval: 1000 });
        await send('Profiler.start');
    }
    if (restartReplay) {
        // The default headless viewport clips the Restart seal below its edge.
        await send('Emulation.setDeviceMetricsOverride', {
            width: 1024, height: 768, deviceScaleFactor: 1, mobile: false,
        });
    }
    await send('Page.navigate', { url: pageUrl });
    return async () => {
        if (!cpuProfile) {
            socket.close();
            return;
        }
        const { profile: result } = await send('Profiler.stop');
        writeFileSync(cpuProfile, JSON.stringify(result));
        socket.close();
        console.log(`CPU profile: ${cpuProfile}`);
    };
}

let chrome = null;
let profile = null;
let profileReady = null;
let finishing = false;
async function finish(code) {
    if (finishing) return;
    finishing = true;
    if (profileReady !== null) {
        try { await (await profileReady)(); }
        catch (error) { console.error('CPU profiling failed:', error); code = 1; }
    }
    if (chrome && chrome.exitCode === null && chrome.signalCode === null) {
        await new Promise((resolve) => {
            chrome.once('exit', resolve);
            chrome.kill('SIGKILL');
        });
    }
    if (timingsFile) {
        writeFileSync(timingsFile, JSON.stringify({
            mission, pkgDir: resolve(pkgDir), bootAt, activatedAt, inGameAt, bootstrapAt,
            ...(restartReplay ? { restartVerified, restartCycles } : {}),
            logs: timingLogs.sort((a, b) => a.pageMs - b.pageMs),
            resourceTimings, resourceSnapshots,
        }, null, 2));
        console.log(`Startup timings: ${timingsFile}`);
        if (code === 0 && waitIngame && resourceTimings === null) {
            console.error('Missing browser resource timings');
            code = 1;
        }
    }
    server.close();
    if (profile !== null) rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    process.exit(code);
}

server.listen(0, '127.0.0.1', () => {
    const { port } = server.address();
    profile = mkdtempSync(join(tmpdir(), 'robin-e2e-chrome-'));
    const query = new URLSearchParams({ mission, 'wasm-log': wasmLog });
    if (restartReplay) query.set('start-paused', 'true');
    const pageUrl = `http://127.0.0.1:${port}/?${query}`;
    chrome = spawn(chromeBin, [
        '--headless=new',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        '--enable-unsafe-swiftshader',
        '--autoplay-policy=no-user-gesture-required',
        ...((cpuProfile || restartReplay) ? ['--remote-debugging-port=0', 'about:blank'] : [pageUrl]),
    ], { stdio: ['ignore', 'ignore', 'pipe'] });
    if (cpuProfile || restartReplay) {
        profileReady = startCpuProfile(profile, pageUrl);
        profileReady.catch(() => finish(1));
    }
    let chromeErr = '';
    chrome.stderr.on('data', (c) => { chromeErr += c; });
    console.log(`[e2e: mission=${mission} isolated=${isolated} pkg=${pkgDir}]`);
    setTimeout(() => {
        console.error('[timeout] mission did not install within 10 minutes');
        console.error(chromeErr.slice(-2000));
        finish(1);
    }, 10 * 60 * 1000).unref();
});
