import { lstat, readFile, readdir } from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import {
    DEPLOYMENT,
    validatePublicHeaders,
    validateSignerHeaders,
} from './verify-cloudflare-deployment.mjs';
import { verifyIdentitySignerBridge } from './verify-identity-signer-bridge.mjs';

const ALLOWED_EXTENSIONS = new Set([
    '.avif', '.css', '.gif', '.gz', '.html', '.ico', '.jpeg', '.jpg', '.js', '.json',
    '.mjs', '.otf', '.png', '.svg', '.ttf', '.txt', '.wasm', '.webmanifest',
    '.webp', '.woff', '.woff2', '.xml',
]);
const TEXT_EXTENSIONS = new Set(['.css', '.html', '.js', '.json', '.mjs', '.svg', '.webmanifest', '.xml']);
const RETIRED_PAGES_FALLBACK = /(?:^|[^a-z0-9.-])phiresky\.github\.io(?::[0-9]{1,5})?(?:[/?#]|$)|robin-hood-the-legend-of-sherwood-remake-binaries/iu;

function containsRetiredPagesFallback(compiled) {
    let normalized = compiled;
    for (let pass = 0; pass < 3; pass += 1) {
        const decoded = normalized
            .replace(/(?:%|\\x)([0-9a-f]{2})/giu, (_match, hex) => String.fromCharCode(Number.parseInt(hex, 16)))
            .replace(/\\u([0-9a-f]{4})/giu, (_match, hex) => String.fromCharCode(Number.parseInt(hex, 16)))
            .replace(/["'`]\s*\+\s*["'`]/gu, '');
        if (decoded === normalized) break;
        normalized = decoded;
    }
    return RETIRED_PAGES_FALLBACK.test(normalized);
}

async function auditTree(directory, label) {
    const root = resolve(directory);
    const rootFacts = await lstat(root).catch(() => undefined);
    if (rootFacts === undefined || !rootFacts.isDirectory() || rootFacts.isSymbolicLink()) {
        throw new Error(`${label} artifact is not a real directory: ${root}`);
    }
    const files = new Set();
    async function visit(directoryPath) {
        for (const entry of await readdir(directoryPath, { withFileTypes: true })) {
            const absolute = resolve(directoryPath, entry.name);
            const path = relative(root, absolute).split(sep).join('/');
            const facts = await lstat(absolute);
            if (facts.isSymbolicLink()) throw new Error(`${label} artifact contains a symbolic link: ${path}`);
            if (facts.isDirectory()) await visit(absolute);
            else if (facts.isFile()) files.add(path);
            else throw new Error(`${label} artifact contains a non-regular entry: ${path}`);
        }
    }
    await visit(root);
    for (const path of files) {
        if (!/^[\x21-\x7e]+$/u.test(path) || path.includes('\\') || path.split('/').some(segment => segment === '..')) {
            throw new Error(`${label} artifact has a non-canonical path: ${path}`);
        }
        if (path !== '_headers' && !ALLOWED_EXTENSIONS.has(extname(path).toLowerCase())) {
            throw new Error(`${label} artifact has a non-public file extension: ${path}`);
        }
        if (path.endsWith('.map') || path.endsWith('.d.ts') || /(?:^|\/)node_modules(?:\/|$)/u.test(path)) {
            throw new Error(`${label} artifact contains source or dependency output: ${path}`);
        }
    }
    return { files, root };
}

function csp(html, path) {
    const policy = html.match(/<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]+)"/u)?.[1];
    if (policy === undefined) throw new Error(`${path} is missing its static Content-Security-Policy`);
    return policy;
}

function directive(policy, name) {
    return policy.match(new RegExp(`(?:^|;)\\s*${name}\\s+([^;]+)`, 'u'))?.[1]?.trim().split(/\s+/u);
}

function validateHtmlReferences(root, sourcePath, html, files, label) {
    if (/<base(?:\s|>)/iu.test(html)) throw new Error(`${sourcePath} must not define a base URL`);
    for (const match of html.matchAll(/<(?:script|link|img)\b[^>]*\b(?:src|href)="([^"]+)"[^>]*>/giu)) {
        const reference = match[1];
        if (reference === undefined || reference.startsWith('#') || /^[a-z][a-z0-9+.-]*:/iu.test(reference)) continue;
        const clean = reference.split(/[?#]/u, 1)[0];
        const absolute = clean.startsWith('/')
            ? resolve(root, clean.slice(1))
            : resolve(root, dirname(sourcePath), clean);
        const path = relative(root, absolute).split(sep).join('/');
        if (path.startsWith('../') || !files.has(path)) {
            throw new Error(`${label} ${sourcePath} references missing or escaping asset ${reference}`);
        }
    }
}

async function compiledText(root, files) {
    return (await Promise.all([...files]
        .filter(path => path.endsWith('.js') || path.endsWith('.wasm'))
        .map(path => readFile(resolve(root, path)).then(bytes => bytes.toString('latin1'))))).join('\n');
}

export async function verifyPublicBuild(directory) {
    const { files, root } = await auditTree(directory, 'Public');
    for (const required of ['_headers', 'index.html', 'leaderboards/index.html']) {
        if (!files.has(required)) throw new Error(`Public artifact is missing ${required}`);
    }
    if ([...files].some(path => path.startsWith('identity-signer/'))) {
        throw new Error('Public artifact must not contain the isolated signer application');
    }
    validatePublicHeaders(await readFile(resolve(root, '_headers'), 'utf8'));

    const gameHtml = await readFile(resolve(root, 'index.html'), 'utf8');
    const leaderboardHtml = await readFile(resolve(root, 'leaderboards/index.html'), 'utf8');
    validateHtmlReferences(root, 'index.html', gameHtml, files, 'Public');
    validateHtmlReferences(root, 'leaderboards/index.html', leaderboardHtml, files, 'Public');
    const gamePolicy = csp(gameHtml, 'index.html');
    const leaderboardPolicy = csp(leaderboardHtml, 'leaderboards/index.html');
    if (JSON.stringify(directive(gamePolicy, 'frame-src')) !== JSON.stringify([DEPLOYMENT.signerOrigin])
        || JSON.stringify(directive(leaderboardPolicy, 'frame-src')) !== JSON.stringify([DEPLOYMENT.signerOrigin])) {
        throw new Error('Public documents must frame only the isolated signer origin');
    }
    if (JSON.stringify(directive(leaderboardPolicy, 'connect-src')) !== JSON.stringify(["'self'"])) {
        throw new Error('Leaderboard document must use only the same-origin /api VPS route');
    }
    if (/name="robin-highscores-api"/u.test(leaderboardHtml)) {
        throw new Error('Production leaderboard document must not carry a mutable API override');
    }

    const compiled = await compiledText(root, files);
    if (!compiled.includes(DEPLOYMENT.signerOrigin)) {
        throw new Error('Public JavaScript is not pinned to the isolated signer origin');
    }
    if (!compiled.includes('/api/v1')) {
        throw new Error('Public JavaScript is missing the same-origin /api/v1 contract');
    }
    if (containsRetiredPagesFallback(compiled)) {
        throw new Error('Public JavaScript contains a retired GitHub Pages or binaries fallback');
    }
}

export async function verifySignerBuild(directory) {
    const { files, root } = await auditTree(directory, 'Signer');
    for (const required of [
        '_headers',
        'identity-signer/index.html',
        'identity-signer/bridge/leaderboard_identity_bridge.js',
        'identity-signer/bridge/leaderboard_identity_bridge_bg.wasm',
    ]) {
        if (!files.has(required)) throw new Error(`Signer artifact is missing ${required}`);
    }
    if (files.has('index.html') || files.has('leaderboards/index.html')) {
        throw new Error('Signer artifact must not contain the public game or leaderboard application');
    }
    validateSignerHeaders(await readFile(resolve(root, '_headers'), 'utf8'));
    const html = await readFile(resolve(root, 'identity-signer/index.html'), 'utf8');
    validateHtmlReferences(root, 'identity-signer/index.html', html, files, 'Signer');
    const policy = csp(html, 'identity-signer/index.html');
    if (JSON.stringify(directive(policy, 'default-src')) !== JSON.stringify(["'none'"])
        || JSON.stringify(directive(policy, 'connect-src')) !== JSON.stringify(["'self'"])
        || JSON.stringify(directive(policy, 'script-src'))
            !== JSON.stringify(["'self'", "'wasm-unsafe-eval'"])) {
        throw new Error('Signer document must fail closed to same-origin code and storage only');
    }
    await verifyIdentitySignerBridge(resolve(root, 'identity-signer/bridge'));

    const compiled = await compiledText(root, files);
    for (const required of [
        'robinhood.multiplayer-identity.v1',
        'robinhood/browser-seat-proof/v1',
        'robinhood.browser-identity.v1',
        '/identity-signer/bridge/leaderboard_identity_bridge.js',
        '/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm',
    ]) {
        if (!compiled.includes(required)) throw new Error(`Signer JavaScript is missing exact protocol domain ${required}`);
    }
    for (const forbidden of ['sign_raw', 'sign_bytes', 'export_private_key']) {
        if (compiled.includes(forbidden)) throw new Error(`Signer JavaScript exposes forbidden generic operation ${forbidden}`);
    }
    if (containsRetiredPagesFallback(compiled)) {
        throw new Error('Signer JavaScript contains a retired GitHub Pages or binaries fallback');
    }
}

async function main() {
    const [first, second] = process.argv.slice(2);
    if (first === '--signer' && second !== undefined && process.argv.length === 4) {
        await verifySignerBuild(second);
        return;
    }
    if (first === undefined || second !== undefined) {
        throw new Error('usage: node scripts/verify-static-build.mjs [--signer] DIST');
    }
    await verifyPublicBuild(first);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
