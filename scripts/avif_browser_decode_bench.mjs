// Headless-Chrome AVIF decode benchmark + exactness dump (web codec study).
//
//   node scripts/avif_browser_decode_bench.mjs <avif-dir> <out-dir> [--chrome BIN] [--repeats N]
//
// Serves every <avif-dir>/*.avif over loopback, then in the page times each
// browser decode path over the whole set and posts the decoded pixels of the
// last repeat back as <out-dir>/<method>/<stem>.rgba (straight RGBA8, width x
// height of the image) for scoring with
// `rle_jxl_tuning_bench score <src-png-dir> <out-dir>/<method>`.
//
// Methods:
//   bitmap_serial / bitmap_concurrent  createImageBitmap(premultiplyAlpha:none,
//                                      colorSpaceConversion:none) only — the
//                                      decode cost of a GPU-texture-only path
//   bitmap_gl        bitmap -> WebGL2 texImage2D (no premultiply) -> readPixels
//   decoder_rgba     ImageDecoder -> VideoFrame.copyTo({format:'RGBA'})
//   decoder_raw      ImageDecoder -> VideoFrame.copyTo() raw planes; RGB
//                    reconstructed in JS (identity matrix, or BT.601 full range)
//   canvas2d         bitmap -> OffscreenCanvas 2D -> getImageData (premultiplies)
import { createServer } from 'node:http';
import { readFileSync, readdirSync, mkdirSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join, basename } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';

const args = process.argv.slice(2);
const positional = [];
let chromeBin = 'google-chrome';
// --firefox BIN runs the same page in headless Firefox instead of Chrome.
let firefoxBin = null;
let repeats = 3;
const extraFlags = [];
for (let i = 0; i < args.length; i++) {
    if (args[i] === '--chrome') chromeBin = args[++i];
    else if (args[i] === '--firefox') firefoxBin = args[++i];
    else if (args[i] === '--repeats') repeats = Number(args[++i]);
    else if (args[i] === '--flag') extraFlags.push(args[++i]);
    else positional.push(args[i]);
}
const [avifDir, outDir] = positional;
if (!avifDir || !outDir) {
    console.error('usage: node scripts/avif_browser_decode_bench.mjs <avif-dir> <out-dir> [--chrome BIN] [--repeats N]');
    process.exit(2);
}
const stems = readdirSync(avifDir).filter((f) => f.endsWith('.avif')).map((f) => basename(f, '.avif')).sort();

const page = `<!DOCTYPE html><meta charset="utf-8"><script type="module">
const REPEATS = ${repeats};
const log = (m) => fetch('/log', { method: 'POST', body: String(m) });
try {
const stems = await (await fetch('/list')).json();
const bufs = await Promise.all(stems.map(async (s) => new Uint8Array(await (await fetch('/f/' + s)).arrayBuffer())));
const blobs = bufs.map((b) => new Blob([b], { type: 'image/avif' }));
const bmpOpts = { premultiplyAlpha: 'none', colorSpaceConversion: 'none' };
const results = {};
const pixels = {};

const gl = new OffscreenCanvas(1, 1).getContext('webgl2', { premultipliedAlpha: false });
function glRead(bmp) {
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, bmp);
    const fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    const out = new Uint8Array(bmp.width * bmp.height * 4);
    gl.readPixels(0, 0, bmp.width, bmp.height, gl.RGBA, gl.UNSIGNED_BYTE, out);
    gl.deleteFramebuffer(fb);
    gl.deleteTexture(tex);
    return { w: bmp.width, h: bmp.height, rgba: out };
}

function planesToRgba(frame, buf, layout) {
    const w = frame.codedWidth, h = frame.codedHeight;
    const fmt = frame.format;
    const m = frame.colorSpace.matrix;
    const full = frame.colorSpace.fullRange;
    // Chrome hands AVIF frames back already converted to packed RGB; only a
    // decoder exposing I444(A) planes gives YUV access.
    if (fmt === 'RGBA' || fmt === 'RGBX' || fmt === 'BGRA' || fmt === 'BGRX') {
        const src = new Uint8Array(buf.buffer, layout[0].offset, w * h * 4);
        const out = new Uint8Array(w * h * 4);
        const bgr = fmt[0] === 'B';
        const hasAlpha = fmt[3] === 'A';
        for (let i = 0; i < w * h * 4; i += 4) {
            out[i] = src[i + (bgr ? 2 : 0)]; out[i + 1] = src[i + 1]; out[i + 2] = src[i + (bgr ? 0 : 2)];
            out[i + 3] = hasAlpha ? src[i + 3] : 255;
        }
        return { w, h, rgba: out, fmt, matrix: m, fullRange: full };
    }
    if (!(fmt === 'I444A' || fmt === 'I444')) throw new Error('raw format ' + fmt);
    const Y = new Uint8Array(buf.buffer, layout[0].offset, w * h);
    const U = new Uint8Array(buf.buffer, layout[1].offset, w * h);
    const V = new Uint8Array(buf.buffer, layout[2].offset, w * h);
    const A = fmt === 'I444A' ? new Uint8Array(buf.buffer, layout[3].offset, w * h) : null;
    const out = new Uint8Array(w * h * 4);
    const clamp = (v) => (v < 0 ? 0 : v > 255 ? 255 : v);
    for (let i = 0; i < w * h; i++) {
        let r, g, b;
        if (m === 'rgb') { g = Y[i]; b = U[i]; r = V[i]; }
        else {
            if (!full) throw new Error('limited range not handled');
            const y = Y[i], cb = U[i] - 128, cr = V[i] - 128;
            r = clamp(Math.round(y + 1.402 * cr));
            g = clamp(Math.round(y - 0.344136 * cb - 0.714136 * cr));
            b = clamp(Math.round(y + 1.772 * cb));
        }
        out[i * 4] = r; out[i * 4 + 1] = g; out[i * 4 + 2] = b; out[i * 4 + 3] = A ? A[i] : 255;
    }
    return { w, h, rgba: out, fmt, matrix: m, fullRange: full };
}

const methods = {
    bitmap_serial: async () => { const o = []; for (const b of blobs) { const bmp = await createImageBitmap(b, bmpOpts); o.push(null); bmp.close(); } return o; },
    bitmap_concurrent: async () => { const bmps = await Promise.all(blobs.map((b) => createImageBitmap(b, bmpOpts))); bmps.forEach((b) => b.close()); return bmps.map(() => null); },
    bitmap_gl: async () => { const o = []; for (const b of blobs) { const bmp = await createImageBitmap(b, bmpOpts); o.push(glRead(bmp)); bmp.close(); } return o; },
    decoder_rgba: async () => {
        const o = [];
        for (const data of bufs) {
            const dec = new ImageDecoder({ data, type: 'image/avif', premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
            const { image } = await dec.decode();
            const out = new Uint8Array(image.allocationSize({ format: 'RGBA' }));
            await image.copyTo(out, { format: 'RGBA' });
            o.push({ w: image.codedWidth, h: image.codedHeight, rgba: out });
            image.close(); dec.close();
        }
        return o;
    },
    decoder_raw: async () => {
        const o = [];
        for (const data of bufs) {
            const dec = new ImageDecoder({ data, type: 'image/avif', premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
            const { image } = await dec.decode();
            const out = new Uint8Array(image.allocationSize());
            const layout = await image.copyTo(out);
            o.push({ frame: { codedWidth: image.codedWidth, codedHeight: image.codedHeight, format: image.format, colorSpace: image.colorSpace.toJSON() }, out, layout });
            image.close(); dec.close();
        }
        return o;
    },
    canvas2d: async () => {
        const o = [];
        for (const b of blobs) {
            const bmp = await createImageBitmap(b, bmpOpts);
            const c = new OffscreenCanvas(bmp.width, bmp.height).getContext('2d', { willReadFrequently: true });
            c.drawImage(bmp, 0, 0);
            o.push({ w: bmp.width, h: bmp.height, rgba: new Uint8Array(c.getImageData(0, 0, bmp.width, bmp.height).data.buffer) });
            bmp.close();
        }
        return o;
    },
};

for (const [name, fn] of Object.entries(methods)) {
    try {
        const times = [];
        let last;
        for (let r = 0; r < REPEATS; r++) {
            const t0 = performance.now();
            last = await fn();
            times.push(performance.now() - t0);
        }
        results[name] = { best_ms: Math.min(...times), times };
        if (name === 'decoder_raw') {
            results[name].formats = [...new Set(last.map((x) => x.frame.format + '/' + x.frame.colorSpace.matrix + '/' + x.frame.colorSpace.fullRange))];
            const t0 = performance.now();
            last = last.map((x) => planesToRgba(x.frame, x.out, x.layout));
            results[name].js_convert_ms = performance.now() - t0;
        }
        for (let i = 0; i < stems.length; i++) {
            if (!last[i]) continue;
            await fetch('/px/' + name + '/' + stems[i] + '?w=' + last[i].w + '&h=' + last[i].h, { method: 'POST', body: last[i].rgba });
        }
    } catch (e) {
        results[name] = { error: String(e && e.stack || e) };
    }
}
await fetch('/done', { method: 'POST', body: JSON.stringify({ ua: navigator.userAgent, images: stems.length, bytes: bufs.reduce((a, b) => a + b.length, 0), results }) });
} catch (e) { await fetch('/done', { method: 'POST', body: JSON.stringify({ fatal: String(e && e.stack || e) }) }); }
</script>`;

let chrome;
const profile = mkdtempSync(join(tmpdir(), 'avif-bench-chrome-'));
const readBody = (req) => new Promise((res) => { const c = []; req.on('data', (d) => c.push(d)); req.on('end', () => res(Buffer.concat(c))); });
const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://x');
    if (url.pathname === '/') { res.writeHead(200, { 'content-type': 'text/html' }); res.end(page); return; }
    if (url.pathname === '/list') { res.end(JSON.stringify(stems)); return; }
    if (url.pathname.startsWith('/f/')) { res.writeHead(200, { 'content-type': 'image/avif' }); res.end(readFileSync(join(avifDir, decodeURIComponent(url.pathname.slice(3)) + '.avif'))); return; }
    const body = await readBody(req);
    if (url.pathname === '/log') { console.log('page:', body.toString()); res.end(); return; }
    if (url.pathname.startsWith('/px/')) {
        const [, , method, stem] = url.pathname.split('/');
        const w = Number(url.searchParams.get('w'));
        const h = Number(url.searchParams.get('h'));
        mkdirSync(join(outDir, method), { recursive: true });
        // No flip for bitmap_gl: without UNPACK_FLIP_Y the ImageBitmap's top
        // row is texture row 0, which readPixels on the texture FBO returns first.
        if (body.length !== w * h * 4) throw new Error(`${method}/${stem}: ${body.length} bytes for ${w}x${h}`);
        writeFileSync(join(outDir, method, stem + '.rgba'), body);
        res.end();
        return;
    }
    if (url.pathname === '/done') {
        res.end();
        console.log(JSON.stringify(JSON.parse(body.toString()), null, 1));
        // Chrome keeps writing its profile until it exits; clean up after that.
        chrome.once('exit', () => rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }));
        chrome.kill();
        server.close();
        return;
    }
    res.writeHead(404); res.end();
});
server.listen(0, '127.0.0.1', () => {
    const { port } = server.address();
    const url = `http://127.0.0.1:${port}/`;
    chrome = firefoxBin
        ? spawn(firefoxBin, ['--headless', '--no-remote', '--profile', profile, ...extraFlags, url], { stdio: ['ignore', 'ignore', 'inherit'] })
        : spawn(chromeBin, ['--headless=new', `--user-data-dir=${profile}`, '--no-first-run', '--enable-unsafe-swiftshader', ...extraFlags, url], { stdio: ['ignore', 'ignore', 'inherit'] });
});
