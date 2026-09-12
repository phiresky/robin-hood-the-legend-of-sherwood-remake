import type { RobinWasmModule } from './boot-lifecycle.js';
import type { RobinRpc } from './replay.js';

export function createRpcClient(wasm: Pick<RobinWasmModule, 'rh_rpc'>): RobinRpc {
    const rhRpc = wasm.rh_rpc;
    if (rhRpc === undefined) throw new Error('wasm module does not export rh_rpc');
    return async <T = unknown>(method: string, params: unknown = null): Promise<T> => {
        try {
            return await rhRpc<T>({ method, params });
        } catch (error) {
            // wasm-bindgen forwards Rust's JsValue::from_str errors as strings.
            throw error instanceof Error ? error : new Error(String(error), { cause: error });
        }
    };
}
