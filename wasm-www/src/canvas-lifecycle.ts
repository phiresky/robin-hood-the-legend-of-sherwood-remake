export type CanvasEnvironment = {
    viewport: () => { readonly fullscreen: boolean; readonly innerWidth: number; readonly innerHeight: number; readonly devicePixelRatio: number };
    subscribe: (sync: () => void) => () => void;
};

function browserEnvironment(canvas: HTMLCanvasElement): CanvasEnvironment {
    return {
        viewport: () => ({ fullscreen: document.fullscreenElement === canvas, innerWidth: window.innerWidth,
            innerHeight: window.innerHeight, devicePixelRatio: window.devicePixelRatio }),
        subscribe: sync => {
            const observer = new ResizeObserver(sync);
            observer.observe(canvas);
            window.addEventListener('resize', sync, { passive: true });
            document.addEventListener('fullscreenchange', sync);
            return () => {
                observer.disconnect();
                window.removeEventListener('resize', sync);
                document.removeEventListener('fullscreenchange', sync);
            };
        },
    };
}

/**
 * Keep CSS layout pixels separate from the WebGPU backing store. The canvas
 * fits the browser viewport in CSS while its drawable size follows device
 * pixels, allowing winit to report native-resolution resize events on HiDPI
 * and fullscreen displays.
 */
export function installCanvasBackingStore(
    canvas: HTMLCanvasElement, environment: CanvasEnvironment = browserEnvironment(canvas),
): { readonly sync: () => void; readonly dispose: () => void } {
    let disposed = false;
    const sync = (): void => {
        if (disposed) return;
        const { fullscreen, innerWidth, innerHeight, devicePixelRatio } = environment.viewport();
        const availableWidth = Math.max(1, innerWidth - (fullscreen ? 0 : 16));
        const availableHeight = Math.max(1, innerHeight - (fullscreen ? 0 : 16));
        const availableAspect = availableWidth / availableHeight;
        const targetAspect = fullscreen
            ? availableAspect
            : Math.min(16 / 9, Math.max(4 / 3, availableAspect));
        const cssWidth = availableAspect >= targetAspect
            ? availableHeight * targetAspect
            : availableWidth;
        const cssHeight = availableAspect >= targetAspect
            ? availableHeight
            : availableWidth / targetAspect;
        canvas.style.width = `${Math.round(cssWidth)}px`;
        canvas.style.height = `${Math.round(cssHeight)}px`;

        const bounds = canvas.getBoundingClientRect();
        const scale = devicePixelRatio || 1;
        const width = Math.max(1, Math.round(bounds.width * scale));
        const height = Math.max(1, Math.round(bounds.height * scale));
        if (canvas.width !== width) canvas.width = width;
        if (canvas.height !== height) canvas.height = height;
    };
    const unsubscribe = environment.subscribe(sync);
    sync();
    return {
        sync,
        dispose: () => { if (!disposed) { disposed = true; unsubscribe(); } },
    };
}
