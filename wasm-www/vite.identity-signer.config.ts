import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const signerRoot = fileURLToPath(new URL('.', import.meta.url));
const signerOutput = fileURLToPath(new URL('./signer-dist', import.meta.url));

export default defineConfig({
    root: signerRoot,
    base: '/',
    build: {
        outDir: signerOutput,
        emptyOutDir: true,
        target: 'es2022',
        sourcemap: false,
        rollupOptions: {
            input: resolve(signerRoot, 'identity-signer/index.html'),
        },
    },
});
