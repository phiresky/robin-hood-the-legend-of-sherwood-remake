import { validateReplayModule, type ReplayValidationRequest, type ReplayValidatorModule } from './replay-worker.ts';

type ValidateReply =
    | { readonly status: 'accepted' }
    | { readonly status: 'rejected'; readonly error: string };

self.addEventListener('message', (event: MessageEvent<ReplayValidationRequest>) => {
    void (async (): Promise<void> => {
        try {
            await validateReplayModule(event.data, {
                importModule: url => import(/* @vite-ignore */ url) as Promise<ReplayValidatorModule>,
                fetchModule: url => fetch(url, { cache: 'force-cache' }),
            });
            self.postMessage({ status: 'accepted' } satisfies ValidateReply);
        } catch (error) {
            self.postMessage({
                status: 'rejected',
                error: error instanceof Error ? error.message : String(error),
            } satisfies ValidateReply);
        }
    })();
});
