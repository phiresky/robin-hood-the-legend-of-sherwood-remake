/** Owns one route lifetime. Superseded work cannot clear a newer route's busy state. */
export class RouteController {
    #active: AbortController | undefined;

    get signal(): AbortSignal | undefined { return this.#active?.signal; }

    cancel(): void {
        this.#active?.abort();
        this.#active = undefined;
    }

    async run(
        render: (signal: AbortSignal) => Promise<void>,
        callbacks: { busy: (value: boolean) => void; error: (error: unknown) => void },
    ): Promise<void> {
        this.cancel();
        const active = new AbortController();
        this.#active = active;
        callbacks.busy(true);
        try {
            await render(active.signal);
        } catch (error) {
            if (!active.signal.aborted) callbacks.error(error);
        } finally {
            if (this.#active === active) callbacks.busy(false);
        }
    }
}
