import { brotliCompressSync, constants } from 'node:zlib';
import { readFile, writeFile } from 'node:fs/promises';

/** Offline packaging: the HTTP sidecar is decoded by the browser while compiling. */
export async function writeBrotliWasm(wasmPath) {
    const compressed = brotliCompressSync(await readFile(wasmPath), {
        params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
    });
    await writeFile(`${wasmPath}.br`, compressed, { flag: 'wx' });
}
