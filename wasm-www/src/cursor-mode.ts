/** Browser capture is explicit and follows the browser's actual lock state. */
export function installCursorMode(canvas: HTMLCanvasElement, button: HTMLButtonElement, status: HTMLElement): () => void {
    const document = canvas.ownerDocument;
    const refresh = () => {
        const captured = document.pointerLockElement === canvas;
        button.textContent = captured ? 'Release cursor' : 'Capture cursor';
        button.setAttribute('aria-pressed', String(captured));
        status.textContent = captured ? 'Edge scrolling on · Esc releases cursor' : 'Edge scrolling off';
    };
    const failed = () => {
        refresh();
        status.textContent = 'Cursor capture failed. Click Capture cursor to retry.';
    };
    const toggle = async () => {
        if (document.pointerLockElement === canvas) document.exitPointerLock();
        else {
            try { canvas.focus(); await canvas.requestPointerLock(); }
            catch { failed(); }
        }
    };
    const captureDrag = (event: PointerEvent) => {
        if (event.button === 0 && document.pointerLockElement === null) canvas.setPointerCapture(event.pointerId);
    };
    button.addEventListener('click', toggle);
    canvas.addEventListener('pointerdown', captureDrag);
    document.addEventListener('pointerlockchange', refresh);
    document.addEventListener('pointerlockerror', failed);
    refresh();
    return () => {
        button.removeEventListener('click', toggle);
        canvas.removeEventListener('pointerdown', captureDrag);
        document.removeEventListener('pointerlockchange', refresh);
        document.removeEventListener('pointerlockerror', failed);
        if (document.pointerLockElement === canvas) document.exitPointerLock();
    };
}
