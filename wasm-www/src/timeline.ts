import type { RobinRpc } from './replay.js';

const FPS = 25;
const POLL_INTERVAL_MS = 500;

type ReplayStatus = {
    readonly frame: number;
    readonly total: number;
    readonly paused: boolean;
};

type StateReply = {
    readonly replay?: ReplayStatus | null;
};

/** The owner must dispose before replacing the runtime or removing its UI. */
export function installTimeline(container: HTMLElement, rpc: RobinRpc): () => void {
    container.replaceChildren();

    const current = document.createElement('span');
    current.className = 'timeline-time';
    current.textContent = '00:00';

    const playPause = document.createElement('button');
    playPause.type = 'button';
    playPause.textContent = 'Play';

    const scrub = document.createElement('input');
    scrub.type = 'range';
    scrub.min = '0';
    scrub.max = '0';
    scrub.step = '1';
    scrub.value = '0';
    scrub.className = 'timeline-scrub';

    const total = document.createElement('span');
    total.className = 'timeline-time';
    total.textContent = '00:00';

    container.append(playPause, current, scrub, total);

    let disposed = false;
    let polling = false;
    let seeking = false;
    let pendingSeek: number | undefined;
    // An interaction invalidates a state request sent before it.
    let revision = 0;
    let scrubbing = false;
    const startScrub = (): void => { scrubbing = true; };
    scrub.addEventListener('pointerdown', startScrub);
    const endScrub = (): void => { scrubbing = false; };
    scrub.addEventListener('pointerup', endScrub);
    scrub.addEventListener('pointercancel', endScrub);
    scrub.addEventListener('blur', endScrub);

    const seek = (): void => {
        if (disposed) return;
        const frame = Number(scrub.value);
        current.textContent = formatTime(frame);
        revision++;
        pendingSeek = frame;
        void drainSeeks();
    };
    scrub.addEventListener('input', seek);

    async function drainSeeks(): Promise<void> {
        if (seeking) return;
        seeking = true;
        try {
            while (!disposed && pendingSeek !== undefined) {
                const frame = pendingSeek;
                pendingSeek = undefined;
                try {
                    await rpc('go-to-frame', { frame, auto_dismiss: true });
                } catch (e) {
                    if (!disposed) console.warn('timeline: go-to-frame failed:', e);
                }
            }
        } finally {
            seeking = false;
        }
    }

    const togglePaused = (): void => {
        if (disposed) return;
        revision++;
        const paused = playPause.dataset.paused !== 'true';
        playPause.dataset.paused = String(paused);
        playPause.textContent = paused ? 'Play' : 'Pause';
        void rpc('set-paused', { paused }).catch((e: unknown) => {
            if (!disposed) console.warn('timeline: set-paused failed:', e);
        });
    };
    playPause.addEventListener('click', togglePaused);

    const intervalId = window.setInterval(poll, POLL_INTERVAL_MS);

    function poll(): void {
        if (disposed || polling || seeking) return;
        polling = true;
        const requestedRevision = revision;
        void (async (): Promise<void> => {
            try {
                const reply = await rpc<StateReply>('state');
                if (disposed || revision !== requestedRevision) return;
                const replay = reply.replay ?? null;
                if (replay === null) {
                    container.style.display = 'none';
                    return;
                }
                const frame = replay.frame;
                const totalFrames = replay.total;
                container.style.display = 'flex';
                scrub.max = String(Math.max(totalFrames, frame, 1));
                total.textContent = formatTime(totalFrames);
                playPause.dataset.paused = String(replay.paused);
                playPause.textContent = replay.paused ? 'Play' : 'Pause';
                if (!scrubbing) {
                    scrub.value = String(frame);
                    current.textContent = formatTime(frame);
                }
            } catch (e) {
                if (disposed || revision !== requestedRevision) return;
                const message = e instanceof Error ? e.message : String(e);
                if (message.includes('unknown method: state')) {
                    dispose();
                    return;
                }
                if (message.includes('engine not ready')) {
                    container.style.display = 'none';
                    return;
                }
                console.warn('timeline: state poll failed:', e);
            } finally {
                polling = false;
            }
        })();
    }

    function dispose(): void {
        if (disposed) return;
        disposed = true;
        pendingSeek = undefined;
        window.clearInterval(intervalId);
        scrub.removeEventListener('pointerdown', startScrub);
        scrub.removeEventListener('pointerup', endScrub);
        scrub.removeEventListener('pointercancel', endScrub);
        scrub.removeEventListener('blur', endScrub);
        scrub.removeEventListener('input', seek);
        playPause.removeEventListener('click', togglePaused);
        container.style.display = 'none';
    }

    poll();
    return dispose;
}

function formatTime(frames: number): string {
    const totalSeconds = Math.max(0, Math.floor(frames / FPS));
    const m = Math.floor(totalSeconds / 60);
    const s = totalSeconds % 60;
    return `${pad2(m)}:${pad2(s)}`;
}

function pad2(n: number): string {
    return n < 10 ? `0${n}` : String(n);
}
