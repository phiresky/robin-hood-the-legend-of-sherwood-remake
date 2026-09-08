// ANSI SGR → CSS renderer for emscripten stdout/stderr.
//
// `tracing_subscriber`'s formatter emits colour via `\x1b[Nm` escapes
// even on wasm (because emscripten's stderr looks like a TTY).  We
// parse the subset it actually uses — reset, bold/dim/italic/underline,
// fg colors 30–37 / 90–97 — and render each run into its own styled
// `<span>`.  Everything else is stripped.

const ANSI_COLORS: Record<number, string> = {
    30: '#000', 31: '#e06c75', 32: '#98c379', 33: '#d19a66',
    34: '#61afef', 35: '#c678dd', 36: '#56b6c2', 37: '#abb2bf',
    90: '#5c6370', 91: '#ff7b85', 92: '#b5e890', 93: '#e5c07b',
    94: '#82c0ff', 95: '#d8a4e8', 96: '#7ad0d8', 97: '#fff',
};

const ANSI_RE = /\x1b\[([\d;]*)m/g;
const MAX_LOG_LINES = 600;

function ansiStyleFor(codes: number[]): string {
    let s = '';
    for (const c of codes) {
        if (c === 0) return ''; // reset → no span
        else if (c === 1) s += 'font-weight:bold;';
        else if (c === 2) s += 'opacity:.65;';
        else if (c === 3) s += 'font-style:italic;';
        else if (c === 4) s += 'text-decoration:underline;';
        else {
            const color = ANSI_COLORS[c];
            if (color !== undefined) s += `color:${color};`;
        }
    }
    return s;
}

/** Strip every SGR escape so DevTools doesn't render `␛[2m…` garbage. */
export function stripAnsi(s: string): string {
    return s.replace(ANSI_RE, '');
}

/** Append one log line to `target`, rendering ANSI colour runs as styled spans. */
export function appendLogLine(
    target: HTMLElement,
    msg: string,
    cls?: string,
): void {
    const line = document.createElement('div');
    if (cls !== undefined) line.className = cls;

    let last = 0;
    let style = '';
    const flush = (end: number): void => {
        if (end <= last) return;
        const text = msg.slice(last, end);
        if (style !== '') {
            const span = document.createElement('span');
            span.setAttribute('style', style);
            span.textContent = text;
            line.appendChild(span);
        } else {
            line.appendChild(document.createTextNode(text));
        }
    };

    ANSI_RE.lastIndex = 0;
    for (let m: RegExpExecArray | null; (m = ANSI_RE.exec(msg)) !== null; ) {
        flush(m.index);
        const codesStr = m[1] ?? '';
        const codes = codesStr
            .split(';')
            .filter((x) => x !== '')
            .map(Number);
        // Bare `\e[m` (no codes) means reset.
        style = codes.length === 0 ? '' : ansiStyleFor(codes);
        last = m.index + m[0].length;
    }
    flush(msg.length);
    target.appendChild(line);
    while (target.childElementCount > MAX_LOG_LINES) {
        target.firstElementChild?.remove();
    }
    target.scrollTop = target.scrollHeight;
}
