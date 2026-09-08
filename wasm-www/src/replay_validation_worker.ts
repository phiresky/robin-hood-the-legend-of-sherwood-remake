type ValidateRequest = {
    readonly jsUrl: string;
    readonly wasmUrl: string;
    readonly compact: string;
};

type ReplayValidatorModule = {
    readonly default: (init?: {
        module_or_path?: string | URL | Request | Response | ArrayBuffer;
    }) => Promise<unknown>;
    readonly validate_compact_replay?: (compact: string) => void;
};

type ValidateReply =
    | { readonly status: 'accepted' }
    | { readonly status: 'rejected'; readonly error: string };

self.addEventListener('message', (event: MessageEvent<ValidateRequest>) => {
    void (async (): Promise<void> => {
        try {
            const module = await import(/* @vite-ignore */ event.data.jsUrl) as ReplayValidatorModule;
            await module.default({ module_or_path: event.data.wasmUrl });
            if (module.validate_compact_replay === undefined) {
                throw new Error('selected wasm build has no isolated replay validator');
            }
            module.validate_compact_replay(event.data.compact);
            self.postMessage({ status: 'accepted' } satisfies ValidateReply);
        } catch (error) {
            self.postMessage({
                status: 'rejected',
                error: error instanceof Error ? error.message : String(error),
            } satisfies ValidateReply);
        }
    })();
});
