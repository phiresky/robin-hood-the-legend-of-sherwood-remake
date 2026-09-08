export async function requestFullContentFolder(signal: AbortSignal, root: Pick<Document, 'querySelector'> = document): Promise<FileList> {
    const gate = root.querySelector<HTMLElement>('#multiplayer-content-gate');
    const input = root.querySelector<HTMLInputElement>('#multiplayer-content-files');
    const choose = root.querySelector<HTMLButtonElement>('#multiplayer-content-choose');
    const status = root.querySelector<HTMLElement>('#multiplayer-content-status');
    if (gate === null || input === null || choose === null || status === null) {
        throw new Error('Full multiplayer content picker is unavailable');
    }
    signal.throwIfAborted();
    gate.hidden = false;
    status.textContent = 'Choose the exact local Full web-content export. Nothing is uploaded.';
    return await new Promise<FileList>((resolve, reject) => {
        const cleanup = (): void => {
            choose.removeEventListener('click', clicked);
            input.removeEventListener('change', changed);
            signal.removeEventListener('abort', aborted);
        };
        const aborted = (): void => { cleanup(); gate.hidden = true; reject(signal.reason); };
        const clicked = (): void => input.click();
        const changed = (): void => {
            const files = input.files;
            if (files === null || files.length === 0) {
                status.textContent = 'No folder selected. Choose the required Full content folder.';
                return;
            }
            cleanup();
            status.textContent = `Authenticating ${files.length} local files; nothing is uploaded…`;
            resolve(files);
        };
        signal.addEventListener('abort', aborted, { once: true });
        input.addEventListener('change', changed);
        choose.addEventListener('click', clicked);
        choose.focus();
    });
}
