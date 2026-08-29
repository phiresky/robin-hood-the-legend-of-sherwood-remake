// Headless-Chrome runner for crates/robin_assets/examples/wasm_decode_bench.rs.
//
// The node runner (scripts/wasm_decode_bench.mjs) cannot execute the THREADED
// build: wasm-bindgen-rayon spawns Web Workers, which node does not provide.
// This runner serves the wasm-bindgen output plus a converted datadir over a
// loopback HTTP server that sets the COOP/COEP headers required for
// SharedArrayBuffer, drives the bench in `google-chrome --headless=new`, and
// prints the same per-mission table.
//
//   node scripts/wasm_decode_bench_chrome.mjs <converted-datadir-root> \
//       <wasm-bindgen-out-dir> [--threads N] [--mission NAME] [--chrome BIN]
//
// --threads 0 (default) runs the serial `bench_mission` path — that also
// works on a plain single-threaded build, giving an apples-to-apples browser
// baseline. --threads N initializes the worker pool with N workers and runs
// `bench_mission_parallel` (threaded build only). --mission limits the run to
// one mission (e.g. H01_Lin_VL).
import { createServer } from 'node:http';
import { readFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { join, resolve, extname, normalize } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';

const args = process.argv.slice(2);
const positional = [];
let threads = 0;
let onlyMission = null;
let chromeBin = 'google-chrome';
for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--threads') threads = Number(args[++i]);
    else if (arg === '--mission') onlyMission = args[++i];
    else if (arg === '--chrome') chromeBin = args[++i];
    else positional.push(arg);
}
const [root, glueDir] = positional;
if (!root || !glueDir || !Number.isInteger(threads) || threads < 0) {
    console.error(
        'usage: node scripts/wasm_decode_bench_chrome.mjs <converted-datadir-root> ' +
        '<wasm-bindgen-out-dir> [--threads N] [--mission NAME] [--chrome BIN]',
    );
    process.exit(2);
}

const MIME = {
    '.js': 'text/javascript',
    '.mjs': 'text/javascript',
    '.wasm': 'application/wasm',
    '.html': 'text/html',
    '.json': 'application/json',
};

const page = `<!DOCTYPE html>
<meta charset="utf-8">
<script type="module">
const post = (path, body) => fetch(path, { method: 'POST', body: JSON.stringify(body) });
try {
    const params = new URLSearchParams(location.search);
    const threads = Number(params.get('threads') ?? '0');
    const only = params.get('mission');
    const glue = await import('/pkg/wasm_decode_bench.js');
    await glue.default({ module_or_path: '/pkg/wasm_decode_bench_bg.wasm' });
    if (threads > 0) {
        if (!crossOriginIsolated) throw new Error('page is not crossOriginIsolated');
        await glue.bench_init_threads(threads);
    }
    const datadir = new Uint8Array(await (await fetch('/data/Data/datadir.bin')).arrayBuffer());
    const missions = JSON.parse(glue.bench_init(datadir));
    for (const [mission, files] of Object.entries(missions)) {
        if (only && mission !== only) continue;
        const parts = await Promise.all(files.map(async (f) => {
            const resp = await fetch('/data/Data/' + f);
            if (!resp.ok) throw new Error('fetch ' + f + ': HTTP ' + resp.status);
            return new Uint8Array(await resp.arrayBuffer());
        }));
        const arr = parts.map((p) => p);
        const result = threads > 0
            ? await glue.bench_mission_parallel(arr)
            : glue.bench_mission(arr);
        await post('/result', { mission, result: JSON.parse(result) });
    }
    await post('/done', {});
} catch (e) {
    await post('/error', { message: String(e && e.stack || e) });
}
</script>`;

const totals = { part_bytes: 0, decode_ms: 0, materialize_ms: 0, vq_chunks: 0, vq_blob_bytes: 0, vq_sprites: 0 };
const mb = (n) => (n / 1e6).toFixed(1);
let missionsSeen = 0;
let finish = null;
const finished = new Promise((resolveDone) => { finish = resolveDone; });

const server = createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    const fail = (code, message) => {
        res.writeHead(code, { 'Content-Type': 'text/plain' });
        res.end(message);
    };
    // Required for crossOriginIsolated (SharedArrayBuffer / wasm threads).
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    res.setHeader('Cache-Control', 'no-store');
    if (req.method === 'POST') {
        let body = '';
        req.on('data', (chunk) => { body += chunk; });
        req.on('end', () => {
            res.end('ok');
            const payload = body.length > 0 ? JSON.parse(body) : {};
            if (url.pathname === '/result') {
                const r = payload.result;
                for (const key of Object.keys(totals)) totals[key] += r[key];
                missionsSeen += 1;
                console.log(
                    `${payload.mission.padEnd(28)}${mb(r.part_bytes).padStart(8)}` +
                    `${r.decode_ms.toFixed(0).padStart(11)}${r.materialize_ms.toFixed(0).padStart(16)}` +
                    `${String(r.vq_chunks).padStart(8)}${String(r.vq_sprites).padStart(9)}  fnv:${r.grids_fnv}`,
                );
            } else if (url.pathname === '/done') {
                finish(0);
            } else if (url.pathname === '/error') {
                console.error(`[page error] ${payload.message}`);
                finish(1);
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
        filePath = join(resolve(glueDir), normalize(url.pathname.slice(5)));
    } else if (url.pathname.startsWith('/data/')) {
        filePath = join(resolve(root), normalize(url.pathname.slice(6)));
    }
    if (filePath === null || !existsSync(filePath)) {
        return fail(404, `not found: ${url.pathname}`);
    }
    res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] ?? 'application/octet-stream' });
    res.end(readFileSync(filePath));
});

server.listen(0, '127.0.0.1', () => {
    const { port } = server.address();
    const query = new URLSearchParams({ threads: String(threads) });
    if (onlyMission !== null) query.set('mission', onlyMission);
    const profile = mkdtempSync(join(tmpdir(), 'robin-bench-chrome-'));
    const chrome = spawn(chromeBin, [
        '--headless=new',
        '--disable-gpu',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        `http://127.0.0.1:${port}/?${query}`,
    ], { stdio: ['ignore', 'ignore', 'pipe'] });
    let chromeErr = '';
    chrome.stderr.on('data', (chunk) => { chromeErr += chunk; });
    console.log(`[chrome bench: threads=${threads}${onlyMission ? ` mission=${onlyMission}` : ''}]`);
    console.log('mission                     parts-MB  decode-ms  materialize-ms  chunks  sprites');
    const timeout = setTimeout(() => {
        console.error('[timeout] no /done after 30 minutes');
        console.error(chromeErr.slice(-2000));
        finish(1);
    }, 30 * 60 * 1000);
    void finished.then((code) => {
        clearTimeout(timeout);
        console.log(
            `TOTAL (${missionsSeen} missions): ${mb(totals.part_bytes)} MB parts, ` +
            `${(totals.decode_ms / 1000).toFixed(1)} s zstd+bitcode, ` +
            `${(totals.materialize_ms / 1000).toFixed(1)} s sprite_codec materialize ` +
            `(${totals.vq_chunks} chunks, ${totals.vq_sprites} VQ sprites, ${mb(totals.vq_blob_bytes)} MB blobs)`,
        );
        chrome.kill('SIGKILL');
        server.close();
        rmSync(profile, { recursive: true, force: true });
        process.exit(code);
    });
});
