import { cp, lstat, mkdir, readdir, writeFile } from 'node:fs/promises';
import { dirname, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

export async function stageEnginePreloadAssets(coreDirectory, outputDirectory) {
    const core = resolve(coreDirectory);
    const output = resolve(outputDirectory);
    const ui = resolve(core, 'Data/Interface/UI');
    const candidates = [resolve(core, 'Data/AudioDurations.json'), resolve(core, 'Data/Interface/Fonts/arial.ttf')];
    for (const entry of (await readdir(ui, { withFileTypes: true }))
        .sort((left, right) => left.name.localeCompare(right.name, 'en'))) {
        if (entry.isFile() && entry.name.endsWith('.png')) candidates.push(resolve(ui, entry.name));
    }
    if (candidates.length < 3) throw new Error('core engine overlay contains no UI PNG assets');

    const manifest = [];
    for (const source of candidates) {
        const facts = await lstat(source).catch(() => undefined);
        if (facts === undefined || !facts.isFile() || facts.isSymbolicLink()) {
            throw new Error(`required core engine overlay is not a regular file: ${source}`);
        }
        const path = relative(core, source).split(sep).join('/');
        if (!/^Data\/(?:AudioDurations\.json|Interface\/Fonts\/arial\.ttf|Interface\/UI\/[A-Za-z0-9_.-]+\.png)$/u.test(path)) {
            throw new Error(`core engine overlay has a non-canonical path: ${path}`);
        }
        const destination = resolve(output, path);
        await mkdir(dirname(destination), { recursive: true });
        await cp(source, destination, { force: false, errorOnExist: true });
        manifest.push({ path, url: path });
    }
    await writeFile(resolve(output, 'preload-assets.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
    return manifest;
}

async function main() {
    const [coreDirectory, outputDirectory, extra] = process.argv.slice(2);
    if (coreDirectory === undefined || outputDirectory === undefined || extra !== undefined) {
        throw new Error('usage: node scripts/stage-engine-preload-assets.mjs CORE_DIRECTORY OUTPUT_DIRECTORY');
    }
    const manifest = await stageEnginePreloadAssets(coreDirectory, outputDirectory);
    console.log(`staged ${manifest.length} engine preload assets`);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
