// End-to-end browser check for the streaming shipping-mission installer.
//
// Boots the REAL game build (wasm-www/pkg, from scripts/build-wasm-threads.sh)
// in headless Chrome behind a loopback COOP/COEP server, launches straight
// into a mission via the `?mission=` query, and relays the page's console to
// stdout. Success is the game's own "activated shipping mission" install
// line; the run also prints the wall time from wasm_boot to installation, so
// fetch+decode overlap improvements show up directly.
//
//   node scripts/wasm_mission_install_chrome.mjs <converted-datadir-root> \
//       [--mission H01_Lin_VL] [--pkg wasm-www/pkg] [--serial] [--chrome BIN]
//
// --serial withholds the COOP/COEP headers, so crossOriginIsolated is false
// and the game exercises the no-worker-pool fallback of the same build.
import { createServer } from 'node:http';
import { readFileSync, existsSync, mkdtempSync, rmSync, readdirSync } from 'node:fs';
import { join, resolve, extname, normalize } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';

const args = process.argv.slice(2);
const positional = [];
let mission = 'H01_Lin_VL';
let pkgDir = 'wasm-www/pkg';
let chromeBin = 'google-chrome';
let isolated = true;
for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--mission') mission = args[++i];
    else if (arg === '--pkg') pkgDir = args[++i];
    else if (arg === '--chrome') chromeBin = args[++i];
    else if (arg === '--serial') isolated = false;
    else positional.push(arg);
}
const [root] = positional;
if (!root) {
    console.error(
        'usage: node scripts/wasm_mission_install_chrome.mjs <converted-datadir-root> ' +
        '[--mission NAME] [--pkg DIR] [--serial] [--chrome BIN]',
    );
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
<body>
<canvas id="canvas" width="1024" height="768"></canvas>
<script>
// Relay the console (tracing-wasm writes there) to the harness.
const relay = [];
let relayTimer = null;
const post = (line) => {
    relay.push(line);
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
    const glue = await import('/pkg/robin.js');
    await glue.default({ module_or_path: '/pkg/robin_bg.wasm' });
    for (const path of preloadPaths) {
        const resp = await fetch('/core/' + path);
        if (!resp.ok) throw new Error('preload ' + path + ': HTTP ' + resp.status);
        glue.wasm_preload_asset(path, new Uint8Array(await resp.arrayBuffer()));
    }
    const datadir = new Uint8Array(await (await fetch('/data/Data/datadir.bin')).arrayBuffer());
    console.log('harness: boot t0');
    glue.wasm_boot(datadir, '/data/Data');
} catch (e) {
    console.error('harness boot failed: ' + (e && e.stack || e));
}
</script>`;

let bootAt = null;
let done = false;
const server = createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    if (isolated) {
        res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
        res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    }
    res.setHeader('Cache-Control', 'no-store');
    if (req.method === 'POST' && url.pathname === '/log') {
        let body = '';
        req.on('data', (c) => { body += c; });
        req.on('end', () => {
            res.end('ok');
            for (const line of JSON.parse(body)) {
                console.log(`[page] ${line}`);
                if (line.includes('boot t0')) bootAt = performance.now();
                if (line.includes('activated shipping mission') && bootAt !== null && !done) {
                    done = true;
                    const secs = ((performance.now() - bootAt) / 1000).toFixed(1);
                    console.log(`RESULT: mission installed ${secs}s after wasm_boot`);
                    // Give trailing logs a moment, then finish.
                    setTimeout(() => finish(0), 1500);
                }
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
    if (filePath === null || !existsSync(filePath)) {
        res.writeHead(404, { 'Content-Type': 'text/plain' });
        res.end(`not found: ${url.pathname}`);
        return;
    }
    res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] ?? 'application/octet-stream' });
    res.end(readFileSync(filePath));
});

let chrome = null;
let profile = null;
function finish(code) {
    chrome?.kill('SIGKILL');
    server.close();
    if (profile !== null) rmSync(profile, { recursive: true, force: true });
    process.exit(code);
}

server.listen(0, '127.0.0.1', () => {
    const { port } = server.address();
    profile = mkdtempSync(join(tmpdir(), 'robin-e2e-chrome-'));
    const query = new URLSearchParams({ mission, 'wasm-log': 'info' });
    chrome = spawn(chromeBin, [
        '--headless=new',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        '--enable-unsafe-swiftshader',
        '--autoplay-policy=no-user-gesture-required',
        `http://127.0.0.1:${port}/?${query}`,
    ], { stdio: ['ignore', 'ignore', 'pipe'] });
    let chromeErr = '';
    chrome.stderr.on('data', (c) => { chromeErr += c; });
    console.log(`[e2e: mission=${mission} isolated=${isolated} pkg=${pkgDir}]`);
    setTimeout(() => {
        console.error('[timeout] mission did not install within 10 minutes');
        console.error(chromeErr.slice(-2000));
        finish(1);
    }, 10 * 60 * 1000).unref();
});
