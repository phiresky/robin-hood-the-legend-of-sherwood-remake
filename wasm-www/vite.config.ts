import { defineConfig } from 'vite';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve, sep } from 'node:path';

const DEFAULT_LOCAL_BINARIES_ROOT = fileURLToPath(
    new URL('../../../../binaries/', import.meta.url),
);
const LOCAL_BINARIES_ROOT = resolve(
    process.env.ROBIN_LOCAL_BINARIES_ROOT ?? DEFAULT_LOCAL_BINARIES_ROOT,
);

function contentType(path: string): string {
    if (path.endsWith('.json')) {
        return 'application/json';
    }
    if (path.endsWith('.js')) {
        return 'application/javascript';
    }
    if (path.endsWith('.wasm')) {
        return 'application/wasm';
    }
    return 'application/octet-stream';
}

// Single-threaded wasm-bindgen build — no pthreads, no
// SharedArrayBuffer, no COOP/COEP requirement.  The page can be hosted
// from any static origin (GitHub Pages, CDN, etc.) without crossOrigin
// isolation.
export default defineConfig({
    plugins: [{
        name: 'local-binaries-pages',
        configureServer(server) {
            server.middlewares.use((req, res, next) => {
                const pathname = req.url?.split('?', 1)[0] ?? '';
                if (!pathname.startsWith('/wasm/') && !pathname.startsWith('/datadirs/')) {
                    next();
                    return;
                }

                const decodedPath = decodeURIComponent(pathname);
                const filePath = resolve(LOCAL_BINARIES_ROOT, `.${decodedPath}`);
                if (!filePath.startsWith(`${LOCAL_BINARIES_ROOT}${sep}`)) {
                    res.statusCode = 403;
                    res.end('forbidden');
                    return;
                }
                if (!existsSync(filePath) || !statSync(filePath).isFile()) {
                    res.statusCode = 404;
                    res.end('not found');
                    return;
                }

                res.setHeader('content-type', contentType(filePath));
                createReadStream(filePath).pipe(res);
            });
        },
    }],
    // `'mpa'` disables Vite's HTML-fallback middleware so missing
    // static files return a real 404 instead of `index.html` with
    // status 200.  Keeps `fetch('./data/Data/datadir.bin')` from
    // resolving to HTML when the symlink is missing.
    appType: 'mpa',
    server: {
        // Vite's dev-mode FS protection refuses to follow symlinks
        // out of the project root by default, which breaks
        // `public/data` (a symlink to the converted shipping
        // datadir) and `pkg/robin.wasm` (the wasm-bindgen output
        // tree).  Allow the whole repo so symlinked artefacts
        // resolve from anywhere.
        fs: {
            allow: ['..', '../..', '../../..', '../../../..'],
        },
    },
    build: {
        target: 'es2022',
        sourcemap: true,
    },
});
