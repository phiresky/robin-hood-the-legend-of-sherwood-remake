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
export const MAX_COMPRESSED_REPORT_BYTES = 20 * 1024 * 1024;
const MAX_DECODED_REPORT_BYTES = 256 * 1024 * 1024;
const MAX_LOG_BYTES = 32 * 1024 * 1024;

export interface QueuedDiagnostic { id: string; body: string }
export interface DiagnosticQueue {
    list(): Promise<QueuedDiagnostic[]>;
    put(report: QueuedDiagnostic): Promise<void>;
    remove(id: string): Promise<void>;
}

/** IndexedDB avoids localStorage's small quota for larger diagnostic reports. */
export function diagnosticQueue(factory: IDBFactory): DiagnosticQueue {
    const database = new Promise<IDBDatabase>((resolve, reject) => {
        const request = factory.open('robin-diagnostics', 1);
        request.onupgradeneeded = () => request.result.createObjectStore('reports', { keyPath: 'id' });
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
        request.onblocked = () => reject(new Error('Diagnostic storage upgrade is blocked'));
    });
    const operation = async <T>(mode: IDBTransactionMode, action: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> => {
        const db = await database;
        return new Promise((resolve, reject) => {
            const transaction = db.transaction('reports', mode);
            const request = action(transaction.objectStore('reports'));
            transaction.oncomplete = () => resolve(request.result);
            transaction.onabort = () => reject(transaction.error ?? new Error('Diagnostic storage transaction aborted'));
            transaction.onerror = () => reject(transaction.error);
        });
    };
    return {
        list: () => operation('readonly', store => store.getAll()),
        async put(report) { await operation('readwrite', store => store.put(report)); },
        async remove(id) { await operation('readwrite', store => store.delete(id)); },
    };
}

export async function compressDiagnostic(body: string): Promise<Blob> {
    const raw = new Blob([body]);
    if (raw.size > MAX_DECODED_REPORT_BYTES) throw new Error('Report exceeds decoded safety limit');
    const compressed = await new Response(raw.stream().pipeThrough(new CompressionStream('gzip'))).blob();
    if (compressed.size > MAX_COMPRESSED_REPORT_BYTES) throw new Error('Report exceeds 20 MiB compressed upload limit');
    return compressed;
}
export function boundedText(text: string, limit: number): string {
    const bytes = encoder.encode(text);
    if (bytes.length <= limit) return text;
    // Avoid replacement characters pushing UTF-8 output past the byte limit.
    let end = limit;
    while (end > 0 && ((bytes[end] ?? 0) & 0xc0) === 0x80) end--;
    return decoder.decode(bytes.subarray(0, end));
}
export async function submitDiagnostic(body: string, fetcher: typeof fetch = fetch): Promise<string> {
    const compressed = await compressDiagnostic(body);
    const response = await fetcher('/api/v1/diagnostics', {
        method: 'POST', headers: { 'Content-Type': 'application/json', 'Content-Encoding': 'gzip' },
        body: compressed, cache: 'no-store', redirect: 'error', credentials: 'omit',
        signal: AbortSignal.timeout(120_000),
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

export function installDiagnostics(queueStore?: DiagnosticQueue): { log: (line: string) => void; failure: (error: unknown) => void; setBuild: (build: string) => void } {
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
    const logLines: Array<{ text: string; bytes: number }> = [];
    let logHead = 0;
    let logBytes = 0;
    let uploading = false;
    let automaticReports = 0;
    // Open lazily inside the error-handled queue operations so denied storage
    // reports a failure without preventing the game from starting.
    const storage = (): DiagnosticQueue => queueStore ??= diagnosticQueue(window.indexedDB);
    const showStatus = (message: string): void => { status.textContent = message; button.title = message; };
    const flush = async (): Promise<void> => {
        if (uploading) return;
        uploading = true;
        try {
            const legacy = window.localStorage;
            const keys = Object.keys(legacy).filter(key => key.startsWith(PREFIX)).sort().slice(0, 10);
            for (const key of keys) {
                const body = legacy.getItem(key);
                if (body === null) continue;
                await storage().put({ id: key, body });
                legacy.removeItem(key);
            }
            for (const { id: key, body } of (await storage().list()).slice(0, 10)) {
                const id = await submitDiagnostic(body);
                await storage().remove(key);
                showStatus(`Report submitted: ${id}`);
            }
        } catch (error) {
            showStatus(`Report remains queued: ${error instanceof Error ? error.message : String(error)}`);
        } finally { uploading = false; }
    };
    const queue = async (kind: DiagnosticReport['kind'], detail: string, stack: string | null): Promise<void> => {
        const report: DiagnosticReport = {
            schema_version: 1, kind, description: boundedText(detail, 16384),
            engine_commit: boundedText(build, 128), platform: 'wasm32-browser',
            occurred_at_unix_ms: Date.now(), backtrace: stack === null ? null : boundedText(stack, MAX_LOG_BYTES),
            recent_log: logLines.slice(logHead).map(line => line.text).join(''),
            attachments: [], warnings: ['Browser replay attachment is not yet implemented.'],
        };
        if ((await storage().list()).length >= 10) throw new Error('Report queue is full; retry when online');
        const body = JSON.stringify(report);
        await compressDiagnostic(body);
        await storage().put({ id: PREFIX + Date.now() + '-' + crypto.randomUUID(), body });
        showStatus('Report queued for submission.');
        void flush();
    };
    const failure = (error: unknown): void => {
        if (automaticReports >= 3) return;
        automaticReports++;
        void queue('fatal_error', error instanceof Error ? error.message : String(error), error instanceof Error ? error.stack ?? null : null)
            .catch(failure => showStatus(`Could not queue crash report: ${String(failure)}`));
    };
    button.addEventListener('click', () => { dialog.showModal(); description.focus(); });
    document.querySelector('#bug-report-close')?.addEventListener('click', () => dialog.close());
    form.addEventListener('submit', event => {
        event.preventDefault();
        if (description.value.trim().length === 0) { description.focus(); return; }
        send.disabled = true;
        void queue('bug', description.value.trim(), null)
            .then(() => { description.value = ''; })
            .catch(error => showStatus(`Could not queue report: ${String(error)}`))
            .finally(() => { send.disabled = false; });
    });
    window.addEventListener('error', event => failure(event.error ?? event.message));
    window.addEventListener('unhandledrejection', event => failure(event.reason));
    window.addEventListener('online', () => { void flush(); });
    void flush();
    return {
        setBuild(value) { build = value; },
        failure,
        log(line) {
            // Bound UTF-8 bytes without copying the entire enlarged log every
            // time the game logs an event. Join only when capturing a report.
            const text = boundedText(line + '\n', MAX_LOG_BYTES);
            const bytes = encoder.encode(text).length;
            logLines.push({ text, bytes });
            logBytes += bytes;
            while (logBytes > MAX_LOG_BYTES) {
                logBytes -= logLines[logHead]!.bytes;
                // Release large evicted strings before the next array compaction.
                logLines[logHead++] = { text: '', bytes: 0 };
            }
            if (logHead >= 1024) { logLines.splice(0, logHead); logHead = 0; }
            // Rust wasm panic hooks write to console.error before wasm traps.
            if (line.includes('panicked at')) failure(line);
        },
    };
}
