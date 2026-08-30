import type { BrowserContentEdition, VerifiedBrowserJoinTicket } from './join_ticket.js';

const HASH_RE = /^[0-9a-f]{64}$/;
const PATH_RE = /^[A-Za-z0-9._/-]+$/;
const MAX_MANIFEST_BYTES = 4 * 1024 * 1024;
const MAX_CONTENT_FILE_BYTES = 512 * 1024 * 1024;
const MAX_CONTENT_PACKAGE_BYTES = 2 * 1024 * 1024 * 1024;

export type MultiplayerBuildManifest = {
    readonly commit: string;
    readonly short: string;
    readonly netProtocol: number;
    readonly ticketSchema: number;
    readonly multiplayerContent: {
        readonly schema: 2;
        readonly demo: RemoteDemoContent;
        readonly full: { readonly manifestSha256: string } | null;
    };
};

type RemoteDemoContent = {
    readonly url: string;
    readonly sha256: string;
    readonly byteLength: number;
    readonly nativeContentSha256: string;
};

type LocalContentFile = {
    readonly path: string;
    readonly kind: 'shipping' | 'asset';
    readonly byte_length: number;
    readonly sha256: string;
};

type LocalContentManifest = {
    readonly schema: 2;
    readonly edition: 'full';
    readonly engine_version: string;
    readonly native_content_sha256: string;
    readonly datadir: Omit<LocalContentFile, 'kind'>;
    readonly files: readonly LocalContentFile[];
};

export type PreparedMultiplayerContent = {
    readonly edition: BrowserContentEdition;
    readonly datadir: Uint8Array<ArrayBuffer>;
    readonly dataBaseUrl: string;
    readonly assets: readonly { readonly path: string; readonly bytes: Uint8Array<ArrayBuffer> }[];
    readonly shippingFiles: readonly { readonly path: string; readonly bytes: Uint8Array<ArrayBuffer> }[];
};

function exactObject(value: unknown, label: string, keys: readonly string[]): Record<string, unknown> {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const object = value as Record<string, unknown>;
    const actual = Object.keys(object);
    if (actual.length !== keys.length || actual.some((key, index) => key !== keys[index])) {
        throw new Error(`${label} has missing, unknown, or non-canonical fields`);
    }
    return object;
}

function safeInteger(value: unknown, label: string, min = 0): number {
    if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min) {
        throw new Error(`${label} must be an integer of at least ${min}`);
    }
    return value;
}

function sha256String(value: unknown, label: string): string {
    if (typeof value !== 'string' || !HASH_RE.test(value)) {
        throw new Error(`${label} must be a lowercase SHA-256 digest`);
    }
    return value;
}

function canonicalPath(value: unknown, label: string): string {
    if (
        typeof value !== 'string'
        || !PATH_RE.test(value)
        || value.startsWith('/')
        || value.endsWith('/')
        || value.split('/').some((part) => part.length === 0 || part === '.' || part === '..')
    ) {
        throw new Error(`${label} must be a contained canonical relative path`);
    }
    return value;
}

function parseDemoContent(value: unknown): RemoteDemoContent {
    const object = exactObject(value, 'Demo multiplayer content', [
        'url', 'sha256', 'byteLength', 'nativeContentSha256',
    ]);
    const url = String(object.url ?? '');
    let parsed: URL;
    try {
        parsed = new URL(url);
    } catch {
        throw new Error('Demo multiplayer content URL is invalid');
    }
    if (parsed.protocol !== 'https:' || parsed.toString() !== url) {
        throw new Error('Demo multiplayer content URL must be canonical HTTPS');
    }
    return {
        url,
        sha256: sha256String(object.sha256, 'Demo multiplayer content digest'),
        byteLength: safeInteger(object.byteLength, 'Demo multiplayer content byte length', 1),
        nativeContentSha256: sha256String(
            object.nativeContentSha256,
            'Demo native content identity',
        ),
    };
}

export function parseMultiplayerBuildManifest(
    value: unknown,
    ticket: VerifiedBrowserJoinTicket,
): MultiplayerBuildManifest {
    const object = value as Record<string, unknown>;
    if (object === null || typeof object !== 'object' || Array.isArray(object)) {
        throw new Error('multiplayer build manifest must be an object');
    }
    const commit = String(object.commit ?? '');
    const short = String(object.short ?? '');
    if (commit !== ticket.payload.engine_version || short !== commit.slice(0, 12)) {
        throw new Error('multiplayer build manifest does not match the host-signed engine version');
    }
    if (object.netProtocol !== ticket.payload.net_protocol || object.ticketSchema !== ticket.payload.schema) {
        throw new Error('multiplayer build manifest protocol is incompatible with the invitation');
    }
    const content = exactObject(object.multiplayerContent, 'multiplayer content catalog', [
        'schema', 'demo', 'full',
    ]);
    if (content.schema !== 2) throw new Error('unsupported multiplayer content catalog schema');
    const full = content.full === null ? null : exactObject(content.full, 'Full content binding', [
        'manifestSha256',
    ]);
    return {
        commit,
        short,
        netProtocol: ticket.payload.net_protocol,
        ticketSchema: ticket.payload.schema,
        multiplayerContent: {
            schema: 2,
            demo: parseDemoContent(content.demo),
            full: full === null ? null : {
                manifestSha256: sha256String(full.manifestSha256, 'Full content manifest digest'),
            },
        },
    };
}

async function digestHex(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function fetchExactDemo(content: RemoteDemoContent): Promise<PreparedMultiplayerContent> {
    if (content.byteLength > MAX_CONTENT_FILE_BYTES) {
        throw new Error('Demo multiplayer datadir exceeds the browser content limit');
    }
    const response = await fetch(content.url, { cache: 'force-cache' });
    if (!response.ok) throw new Error(`fetch Demo multiplayer content: HTTP ${response.status}`);
    const claimedLength = response.headers.get('Content-Length');
    if (claimedLength !== null && Number(claimedLength) !== content.byteLength) {
        throw new Error('Demo multiplayer content length does not match its build manifest');
    }
    const datadir = new Uint8Array(await response.arrayBuffer());
    if (datadir.byteLength !== content.byteLength || await digestHex(datadir) !== content.sha256) {
        throw new Error('Demo multiplayer content does not match its exact build manifest');
    }
    return {
        edition: 'demo',
        datadir,
        dataBaseUrl: content.url.slice(0, content.url.lastIndexOf('/')),
        assets: [],
        shippingFiles: [],
    };
}

function parseContentFile(value: unknown, label: string, withKind: boolean): LocalContentFile {
    const keys = withKind
        ? ['path', 'kind', 'byte_length', 'sha256']
        : ['path', 'byte_length', 'sha256'];
    const object = exactObject(value, label, keys);
    const kind = withKind ? object.kind : 'shipping';
    if (kind !== 'shipping' && kind !== 'asset') throw new Error(`${label} has an invalid kind`);
    const byteLength = safeInteger(object.byte_length, `${label} byte length`, 1);
    if (byteLength > MAX_CONTENT_FILE_BYTES) throw new Error(`${label} exceeds the browser file limit`);
    return {
        path: canonicalPath(object.path, `${label} path`),
        kind,
        byte_length: byteLength,
        sha256: sha256String(object.sha256, `${label} digest`),
    };
}

function parseLocalManifest(bytes: Uint8Array<ArrayBuffer>, engineVersion: string): LocalContentManifest {
    let text: string;
    try {
        text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    } catch {
        throw new Error('Full content manifest is not UTF-8');
    }
    let raw: unknown;
    try {
        raw = JSON.parse(text) as unknown;
    } catch {
        throw new Error('Full content manifest is not JSON');
    }
    const object = exactObject(raw, 'Full content manifest', [
        'schema', 'edition', 'engine_version', 'native_content_sha256', 'datadir', 'files',
    ]);
    if (JSON.stringify(object) !== text) {
        throw new Error('Full content manifest is not canonical JSON');
    }
    if (object.schema !== 2 || object.edition !== 'full' || object.engine_version !== engineVersion) {
        throw new Error('Full content manifest does not match this engine and edition');
    }
    const nativeContentSha256 = sha256String(
        object.native_content_sha256,
        'Full native content identity',
    );
    if (!Array.isArray(object.files) || object.files.length === 0) {
        throw new Error('Full content manifest must list its exact package files');
    }
    const datadir = parseContentFile(object.datadir, 'Full content datadir', false);
    const files = object.files.map((file, index) => parseContentFile(
        file,
        `Full content file ${index}`,
        true,
    ));
    const paths = new Set<string>([datadir.path]);
    for (const file of files) {
        if (paths.has(file.path)) throw new Error(`Full content manifest repeats ${file.path}`);
        paths.add(file.path);
    }
    const total = datadir.byte_length + files.reduce((sum, file) => sum + file.byte_length, 0);
    if (total > MAX_CONTENT_PACKAGE_BYTES) throw new Error('Full content package exceeds 2 GiB');
    return {
        schema: 2,
        edition: 'full',
        engine_version: engineVersion,
        native_content_sha256: nativeContentSha256,
        datadir: {
            path: datadir.path,
            byte_length: datadir.byte_length,
            sha256: datadir.sha256,
        },
        files,
    };
}

function selectedFilesByRelativePath(files: FileList): Map<string, File> {
    const selected = Array.from(files);
    const manifest = selected.find((file) => (
        file.webkitRelativePath === 'robinhood-web-content.json'
        || file.webkitRelativePath.endsWith('/robinhood-web-content.json')
    ));
    if (manifest === undefined) {
        throw new Error('Selected folder has no robinhood-web-content.json');
    }
    const manifestPath = manifest.webkitRelativePath || manifest.name;
    const suffix = 'robinhood-web-content.json';
    const root = manifestPath.slice(0, manifestPath.length - suffix.length);
    const byPath = new Map<string, File>();
    for (const file of selected) {
        const selectedPath = file.webkitRelativePath || file.name;
        if (!selectedPath.startsWith(root)) throw new Error('Selected files do not share one package root');
        const relative = canonicalPath(selectedPath.slice(root.length), 'selected content path');
        if (byPath.has(relative)) throw new Error(`Selected folder repeats ${relative}`);
        byPath.set(relative, file);
    }
    return byPath;
}

async function fileBytes(file: File, expected: LocalContentFile): Promise<Uint8Array<ArrayBuffer>> {
    if (file.size !== expected.byte_length) throw new Error(`${expected.path} has the wrong byte length`);
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (await digestHex(bytes) !== expected.sha256) throw new Error(`${expected.path} has the wrong digest`);
    return bytes;
}

async function loadExactFull(
    ticket: VerifiedBrowserJoinTicket,
    expectedManifestSha256: string,
    selected: FileList,
): Promise<PreparedMultiplayerContent> {
    const files = selectedFilesByRelativePath(selected);
    const manifestFile = files.get('robinhood-web-content.json');
    if (manifestFile === undefined || manifestFile.size > MAX_MANIFEST_BYTES) {
        throw new Error('Full content manifest is missing or too large');
    }
    const manifestBytes = new Uint8Array(await manifestFile.arrayBuffer());
    if (await digestHex(manifestBytes) !== expectedManifestSha256) {
        throw new Error('Full content manifest is not the exact build-authorized manifest');
    }
    const manifest = parseLocalManifest(manifestBytes, ticket.payload.engine_version);
    if (manifest.native_content_sha256 !== ticket.payload.content_identity_sha256) {
        throw new Error('Selected Full package does not match the native host content closure');
    }
    const expectedPaths = new Set([
        'robinhood-web-content.json',
        manifest.datadir.path,
        ...manifest.files.map((file) => file.path),
    ]);
    for (const path of files.keys()) {
        if (!expectedPaths.has(path)) throw new Error(`Selected Full package has unexpected file ${path}`);
    }
    if (files.size !== expectedPaths.size) throw new Error('Selected Full package is incomplete');

    const datadirFile = files.get(manifest.datadir.path);
    if (datadirFile === undefined) throw new Error(`Full package is missing ${manifest.datadir.path}`);
    const datadir = await fileBytes(datadirFile, { ...manifest.datadir, kind: 'shipping' });
    const assets: Array<{ path: string; bytes: Uint8Array<ArrayBuffer> }> = [];
    const shippingFiles: Array<{ path: string; bytes: Uint8Array<ArrayBuffer> }> = [];
    for (const entry of manifest.files) {
        const file = files.get(entry.path);
        if (file === undefined) throw new Error(`Full package is missing ${entry.path}`);
        const prepared = { path: entry.path, bytes: await fileBytes(file, entry) };
        (entry.kind === 'asset' ? assets : shippingFiles).push(prepared);
    }
    return {
        edition: 'full',
        datadir,
        dataBaseUrl: `robin-preloaded://${expectedManifestSha256}`,
        assets,
        shippingFiles,
    };
}

export async function prepareMultiplayerContent(
    ticket: VerifiedBrowserJoinTicket,
    build: MultiplayerBuildManifest,
    requestFullFolder: () => Promise<FileList>,
): Promise<PreparedMultiplayerContent> {
    if (ticket.payload.content_edition === 'demo') {
        if (
            build.multiplayerContent.demo.nativeContentSha256
            !== ticket.payload.content_identity_sha256
        ) {
            throw new Error('Demo browser content does not match the native host content closure');
        }
        return await fetchExactDemo(build.multiplayerContent.demo);
    }
    const full = build.multiplayerContent.full;
    if (full === null) {
        throw new Error('This exact browser build has no authorized Full content package');
    }
    return await loadExactFull(ticket, full.manifestSha256, await requestFullFolder());
}
