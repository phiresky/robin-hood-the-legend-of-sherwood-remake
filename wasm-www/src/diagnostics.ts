/** Browser diagnostics share the native client's private VPS endpoint. */
export interface DiagnosticReport {
    schema_version: 1;
    kind: 'bug' | 'panic' | 'fatal_error';
    description: string;
    engine_commit: string;
    platform: string;
    occurred_at_unix_ms: number;
    backtrace: string | null;
    recent_log: string;
    attachments: Array<{ filename: string; content: string }>;
    warnings: string[];
}
const encoder = new TextEncoder();
const decoder = new TextDecoder();
const PREFIX = 'robin-diagnostic-v1:';
export function boundedText(text: string, limit: number): string {
    const bytes = encoder.encode(text);
    if (bytes.length <= limit) return text;
    // Avoid replacement characters pushing UTF-8 output past the byte limit.
    let end = limit;
    while (end > 0 && ((bytes[end] ?? 0) & 0xc0) === 0x80) end--;
    return decoder.decode(bytes.subarray(0, end));
}
export async function submitDiagnostic(body: string, fetcher: typeof fetch = fetch): Promise<string> {
    if (encoder.encode(body).length > 2 * 1024 * 1024) throw new Error('Report exceeds upload limit');
    const response = await fetcher('/api/v1/diagnostics', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body, cache: 'no-store', redirect: 'error', credentials: 'omit',
        signal: AbortSignal.timeout(15_000),
    });
    if (response.status !== 202) throw new Error(`Server returned HTTP ${response.status}`);
    if (response.body === null) throw new Error('Missing report receipt');
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
        for (;;) {
            const chunk = await reader.read();
            if (chunk.done) break;
            size += chunk.value.length;
            if (size > 4096) throw new Error('Report receipt exceeds limit');
            chunks.push(chunk.value);
        }
    } finally {
        await reader.cancel();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
    const receipt: unknown = JSON.parse(decoder.decode(bytes));
    const expected = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(body))), byte => byte.toString(16).padStart(2, '0')).join('');
    if (typeof receipt !== 'object' || receipt === null || !('schema_version' in receipt) || receipt.schema_version !== 1 || !('report_id' in receipt) || receipt.report_id !== expected) {
        throw new Error('Invalid report receipt');
    }
    return expected;
}

export function installDiagnostics(): { log: (line: string) => void; failure: (error: unknown) => void; setBuild: (build: string) => void } {
    const button = document.querySelector<HTMLButtonElement>('#report-bug');
    const dialog = document.querySelector<HTMLDialogElement>('#bug-report-dialog');
    const form = document.querySelector<HTMLFormElement>('#bug-report-form');
    const description = document.querySelector<HTMLTextAreaElement>('#bug-report-description');
    const status = document.querySelector<HTMLElement>('#bug-report-status');
    const send = document.querySelector<HTMLButtonElement>('#bug-report-send');
    if (button === null || dialog === null || form === null || description === null || status === null || send === null) {
        throw new Error('Missing bug report controls');
    }
    let build = 'unavailable-before-engine-load';
    let recentLog = '';
    let uploading = false;
    let automaticReports = 0;
    const showStatus = (message: string): void => { status.textContent = message; button.title = message; };
    const flush = async (): Promise<void> => {
        if (uploading) return;
        uploading = true;
        try {
            const storage = window.localStorage;
            const keys = Object.keys(storage).filter(key => key.startsWith(PREFIX)).sort().slice(0, 10);
            for (const key of keys) {
                const body = storage.getItem(key);
                if (body === null) continue;
                const id = await submitDiagnostic(body);
                storage.removeItem(key);
                showStatus(`Report submitted: ${id}`);
            }
        } catch (error) {
            showStatus(`Report remains queued: ${error instanceof Error ? error.message : String(error)}`);
        } finally { uploading = false; }
    };
    const queue = (kind: DiagnosticReport['kind'], detail: string, stack: string | null): void => {
        const report: DiagnosticReport = {
            schema_version: 1, kind, description: boundedText(detail, 16384),
            engine_commit: boundedText(build, 128), platform: 'wasm32-browser',
            occurred_at_unix_ms: Date.now(), backtrace: stack === null ? null : boundedText(stack, 128 * 1024),
            recent_log: recentLog, attachments: [], warnings: ['Browser replay attachment is not yet implemented.'],
        };
        const storage = window.localStorage;
        if (Object.keys(storage).filter(key => key.startsWith(PREFIX)).length >= 10) throw new Error('Report queue is full; retry when online');
        const body = JSON.stringify(report);
        storage.setItem(PREFIX + Date.now() + '-' + crypto.randomUUID(), body);
        showStatus('Report queued for submission.');
        void flush();
    };
    const failure = (error: unknown): void => {
        if (automaticReports >= 3) return;
        automaticReports++;
        try { queue('fatal_error', error instanceof Error ? error.message : String(error), error instanceof Error ? error.stack ?? null : null); }
        catch (failure) { showStatus(`Could not queue crash report: ${String(failure)}`); }
    };
    button.addEventListener('click', () => { dialog.showModal(); description.focus(); });
    document.querySelector('#bug-report-close')?.addEventListener('click', () => dialog.close());
    form.addEventListener('submit', event => {
        event.preventDefault();
        if (description.value.trim().length === 0) { description.focus(); return; }
        send.disabled = true;
        try { queue('bug', description.value.trim(), null); description.value = ''; }
        catch (error) { showStatus(`Could not queue report: ${String(error)}`); }
        finally { send.disabled = false; }
    });
    window.addEventListener('error', event => failure(event.error ?? event.message));
    window.addEventListener('unhandledrejection', event => failure(event.reason));
    window.addEventListener('online', () => { void flush(); });
    void flush();
    return {
        setBuild(value) { build = value; },
        failure,
        log(line) {
            // Keep the end of the log; cap JS storage as well as encoded bytes.
            recentLog = boundedText((recentLog + line + '\n').slice(-64 * 1024), 256 * 1024);
            // Rust wasm panic hooks write to console.error before wasm traps.
            if (line.includes('panicked at')) failure(line);
        },
    };
}
