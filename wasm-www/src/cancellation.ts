/** Stop waiting for APIs (dynamic import, WebCrypto, File) without native cancellation.
 * Their already-started work can finish, but its result cannot advance an aborted operation.
 */
export async function withAbort<T>(signal: AbortSignal, operation: () => Promise<T>): Promise<T> {
    signal.throwIfAborted();
    let abort!: () => void;
    try {
        return await new Promise<T>((resolve, reject) => {
            abort = () => reject(signal.reason);
            signal.addEventListener('abort', abort, { once: true });
            Promise.resolve().then(() => { signal.throwIfAborted(); return operation(); }).then(resolve, reject);
        });
    } finally {
        signal.removeEventListener('abort', abort);
    }
}
