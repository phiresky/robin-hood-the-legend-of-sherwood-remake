import {
    lstatSync,
    readdirSync,
    readFileSync,
    realpathSync,
    rmSync,
} from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse } from 'es-module-lexer/js';

const STATIC_IMPORT = 1;
const IMPORT_META = 3;

const RULES = Object.freeze({
    engine: Object.freeze({
        discard: 'browser_identity_vault.js',
        retain: 'browser_identity_client.js',
    }),
    identity_signer: Object.freeze({
        discard: 'browser_identity_client.js',
        retain: 'browser_identity_vault.js',
    }),
});

export function stageBrowserIdentityOrigin(origin, rootArgument, entryArgument) {
    const rule = RULES[origin];
    if (rule === undefined || rootArgument === undefined || entryArgument === undefined) {
        throw new Error('origin, root, and entry are required');
    }
    if (!/^[\x21-\x7e]+$/u.test(entryArgument)
        || entryArgument.startsWith('/')
        || entryArgument.includes('\\')
        || entryArgument.split('/').some(segment => segment.length === 0 || segment === '.' || segment === '..')) {
        throw new Error(`entry path is not canonical relative ASCII: ${entryArgument}`);
    }

    const root = realpathSync(rootArgument);
    const entry = resolve(root, entryArgument);
    if (entry !== root && !entry.startsWith(`${root}${sep}`)) {
        throw new Error(`entry escapes staged ${origin} origin: ${entryArgument}`);
    }

    function regularFilesBelow(directory) {
        const result = [];
        for (const item of readdirSync(directory, { withFileTypes: true })) {
            const path = resolve(directory, item.name);
            const facts = lstatSync(path);
            if (facts.isSymbolicLink()) {
                throw new Error(`symlink is forbidden in staged ${origin} origin: ${relative(root, path)}`);
            }
            if (facts.isDirectory()) result.push(...regularFilesBelow(path));
            else if (facts.isFile()) result.push(path);
            else throw new Error(`non-regular staged ${origin} artifact: ${relative(root, path)}`);
        }
        return result;
    }

    function exactGeneratedModule(files, basename, optional = false) {
        const matches = files.filter(path => path.endsWith(`${sep}${basename}`));
        if (optional && matches.length === 0) return null;
        if (matches.length !== 1) {
            throw new Error(
                `staged ${origin} origin must contain exactly one generated ${basename}; found ${matches.length}`,
            );
        }
        return matches[0];
    }

    const files = regularFilesBelow(root);
    if (!files.includes(entry) || !lstatSync(entry).isFile()) {
        throw new Error(`staged ${origin} origin is missing entry: ${entryArgument}`);
    }
    const retained = exactGeneratedModule(files, rule.retain);
    const discarded = exactGeneratedModule(files, rule.discard, true);
    for (const [generated, basename] of [[retained, rule.retain], [discarded, rule.discard]]) {
        if (generated === null) continue;
        const owner = basename === 'browser_identity_vault.js' ? 'robin_identity_signer' : 'robin_rs';
        const checkedIn = new URL(`../../crates/${owner}/js/${basename}`, import.meta.url);
        if (!readFileSync(generated).equals(readFileSync(checkedIn))) {
            throw new Error(`generated ${basename} differs from its checked-in source authority`);
        }
    }
    const javascript = new Set(files.filter(path => path.endsWith('.js')));
    const imports = new Map();
    for (const path of javascript) {
        let parsed;
        try {
            [parsed] = parse(readFileSync(path, 'utf8'), relative(root, path));
        } catch (error) {
            throw new Error(
                `invalid JavaScript module in staged ${origin} origin: ${relative(root, path)}`,
                { cause: error },
            );
        }
        const targets = [];
        for (const imported of parsed) {
            if (imported.t === IMPORT_META) continue;
            if (imported.t !== STATIC_IMPORT || imported.a !== -1 || imported.n === undefined) {
                throw new Error(
                    `staged ${origin} module has unsupported dynamic, phased, or attributed import: ${relative(root, path)}`,
                );
            }
            if (!(imported.n.startsWith('./') || imported.n.startsWith('../'))
                || imported.n.includes('\\')
                || imported.n.includes('?')
                || imported.n.includes('#')) {
                throw new Error(
                    `staged ${origin} module has non-relative or non-canonical import: ${imported.n}`,
                );
            }
            const target = resolve(dirname(path), imported.n);
            if ((target !== root && !target.startsWith(`${root}${sep}`)) || !javascript.has(target)) {
                throw new Error(`staged ${origin} module imports undeclared JavaScript: ${imported.n}`);
            }
            targets.push(target);
            if (target === discarded) {
                throw new Error(
                    `refusing to discard referenced ${rule.discard} from staged ${origin} origin`,
                );
            }
        }
        imports.set(path, targets);
    }

    const state = new Map();
    const reachable = new Set();
    function visit(path) {
        if (state.get(path) === 'visiting') {
            throw new Error(`staged ${origin} JavaScript module graph contains a cycle`);
        }
        if (state.get(path) === 'visited') return;
        state.set(path, 'visiting');
        reachable.add(path);
        for (const target of imports.get(path) ?? []) visit(target);
        state.set(path, 'visited');
    }
    visit(entry);
    if (!reachable.has(retained)) {
        throw new Error(`staged ${origin} entry cannot reach its required ${rule.retain}`);
    }

    // Separate crates emit only their own transport. Also accept legacy build
    // output with one byte-verified, proven-unreferenced opposite-role snippet.
    if (discarded !== null) rmSync(discarded);
    return {
        retained: relative(root, retained).split(sep).join('/'),
        removed: discarded === null ? null : relative(root, discarded).split(sep).join('/'),
    };
}

function main() {
    const [origin, root, entry] = process.argv.slice(2);
    if (origin === undefined || root === undefined || entry === undefined || process.argv.length !== 5) {
        console.error('usage: stage-browser-identity-origin.mjs ORIGIN ROOT ENTRY');
        process.exitCode = 2;
        return;
    }
    const staged = stageBrowserIdentityOrigin(origin, root, entry);
    console.log(
        `staged ${origin} identity closure: retained ${staged.retained}, removed ${staged.removed}`,
    );
}

const invoked = process.argv[1];
if (invoked !== undefined && import.meta.url === pathToFileURL(resolve(invoked)).href) main();
