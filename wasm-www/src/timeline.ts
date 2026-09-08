import type { RobinRpc } from './replay.js';

const FPS = 25;
const POLL_INTERVAL_MS = 500;

type ReplayStatus = {
    readonly frame: number;
    readonly total: number;
    readonly paused: boolean;
};

type StateReply = {
    readonly frame: number;
    readonly replay?: ReplayStatus | null;
};

export function installTimeline(container: HTMLElement, rpc: RobinRpc): void {
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

    let scrubbing = false;
    scrub.addEventListener('pointerdown', () => { scrubbing = true; });
    const endScrub = (): void => { scrubbing = false; };
    scrub.addEventListener('pointerup', endScrub);
    scrub.addEventListener('pointercancel', endScrub);
    scrub.addEventListener('blur', endScrub);

    scrub.addEventListener('input', () => {
        const frame = Number(scrub.value);
        current.textContent = formatTime(frame);
        void rpc('go-to-frame', { frame }).catch((e: unknown) => {
            console.warn('timeline: go-to-frame failed:', e);
        });
    });

    playPause.addEventListener('click', () => {
        const paused = playPause.dataset.paused !== 'true';
        playPause.dataset.paused = String(paused);
        playPause.textContent = paused ? 'Play' : 'Pause';
        void rpc('set-paused', { paused }).catch((e: unknown) => {
            console.warn('timeline: set-paused failed:', e);
        });
    });

    let liveMax = 0;
    const intervalId = window.setInterval(poll, POLL_INTERVAL_MS);

    function poll(): void {
        void (async (): Promise<void> => {
            try {
                const reply = await rpc<StateReply>('state');
                const replay = reply.replay ?? null;
                if (replay === null) {
                    liveMax = Math.max(liveMax, reply.frame);
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
                if (e instanceof Error && e.message.includes('unknown method: state')) {
                    window.clearInterval(intervalId);
                    container.style.display = 'none';
                    return;
                }
                if (e instanceof Error && e.message.includes('engine not ready')) {
                    container.style.display = 'none';
                    return;
                }
                console.warn('timeline: state poll failed:', e);
            }
        })();
    }

    poll();
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
