export const DEFAULT_NETWORK_DEADLINE_MS = 30_000;

export class NetworkDeadlineError extends Error {
    constructor(label: string) {
        super(`${label} exceeded the browser network deadline.`);
        this.name = 'NetworkDeadlineError';
    }
}

/**
 * One absolute deadline shared by response headers and every streamed body
 * chunk. `race` is intentional: test doubles and broken fetch adapters may
 * ignore AbortSignal even though real browser fetches observe it.
 */
export class NetworkDeadline {
    readonly signal: AbortSignal;
    readonly #controller = new AbortController();
    readonly #parent: AbortSignal | undefined;
    readonly #timer: ReturnType<typeof setTimeout>;
    readonly #label: string;
    #timedOut = false;

    constructor(
        parent: AbortSignal | undefined,
        label: string,
        timeoutMs: number = DEFAULT_NETWORK_DEADLINE_MS,
    ) {
        if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
            throw new Error('Network deadline must be a positive integer number of milliseconds.');
        }
        this.signal = this.#controller.signal;
        this.#parent = parent;
        this.#label = label;
        if (parent?.aborted === true) {
            this.#controller.abort(parent.reason);
        } else {
            parent?.addEventListener('abort', this.#abortFromParent, { once: true });
        }
        this.#timer = setTimeout(() => {
            this.#timedOut = true;
            this.#controller.abort(new NetworkDeadlineError(this.#label));
        }, timeoutMs);
    }

    get timedOut(): boolean { return this.#timedOut; }

    async race<T>(operation: PromiseLike<T>): Promise<T> {
        if (this.signal.aborted) throw abortReason(this.signal, this.#label);
        return await new Promise<T>((resolve, reject) => {
            const onAbort = (): void => { reject(abortReason(this.signal, this.#label)); };
            this.signal.addEventListener('abort', onAbort, { once: true });
            void Promise.resolve(operation).then(
                value => {
                    this.signal.removeEventListener('abort', onAbort);
                    resolve(value);
                },
                error => {
                    this.signal.removeEventListener('abort', onAbort);
                    reject(error);
                },
            );
        });
    }

    cancelBody(body: ReadableStream<Uint8Array> | null, reason?: unknown): void {
        if (body === null) return;
        void body.cancel(reason).catch(() => {
            // Best-effort release after a timeout/abort; retain the original error.
        });
    }

    dispose(): void {
        clearTimeout(this.#timer);
        this.#parent?.removeEventListener('abort', this.#abortFromParent);
    }

    readonly #abortFromParent = (): void => {
        this.#controller.abort(this.#parent?.reason);
    };
}

function abortReason(signal: AbortSignal, label: string): unknown {
    if (signal.reason !== undefined) return signal.reason;
    return new DOMException(`${label} was aborted.`, 'AbortError');
}
