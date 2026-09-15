// Run a wasm32-wasip1 command binary under node with the filesystem root
// preopened, e.g. for serial-wasm decode timing of bench examples:
//   node scripts/wasi_run.mjs <file.wasm> [args...]
import { readFileSync } from 'node:fs';
import { WASI } from 'node:wasi';

const [wasmPath, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: 'preview1', args: [wasmPath, ...args], env: process.env, preopens: { '/': '/' } });
const module = await WebAssembly.compile(readFileSync(wasmPath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
