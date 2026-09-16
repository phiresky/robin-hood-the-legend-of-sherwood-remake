import { createHash } from 'node:crypto';
import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseWebContentManifest, verifyDatadirCorpus, RETAINED_DEMO_GENERATIONS } from './verify-datadir-corpus.mjs';

/** Add an immutable Full package and bind a recorded browser build to it. */
export async function addFullReplayContent({ corpus, source, build, retainedGenerations = RETAINED_DEMO_GENERATIONS }) {
    if (!/^[0-9a-f]{12}$/u.test(build)) throw new Error('Expected a 12-hex recorded build');
    const root = resolve(corpus);
    const data = resolve(source, 'Data');
    const bytes = await readFile(resolve(data, 'robinhood-web-content.json'));
    const manifest = parseWebContentManifest(bytes, 'Full content', 'full');
    const digest = createHash('sha256').update(bytes).digest('hex');
    const base = `datadirs/full/${digest}`;
    await mkdir(resolve(root, base), { recursive: true });
    // Exclusive writes preserve already-published objects. Identical reruns are allowed.
    async function install(path, content) {
        const destination = resolve(root, path);
        await mkdir(resolve(destination, '..'), { recursive: true });
        try { await writeFile(destination, content, { flag: 'wx' }); }
        catch (error) {
            if (error.code !== 'EEXIST' || !content.equals(await readFile(destination))) throw error;
        }
    }
    await install(`${base}/robinhood-web-content.json`, bytes);
    for (const file of [manifest.datadir, ...manifest.files]) {
        const content = await readFile(resolve(data, file.path));
        if (content.length !== file.byte_length
            || createHash('sha256').update(content).digest('hex') !== file.sha256) {
            throw new Error(`Full content mismatch: ${file.path}`);
        }
        await install(`${base}/${file.path}`, content);
    }
    await install(`datadirs/replays/${build}.json`, Buffer.from(JSON.stringify({
        url: `/${base}/datadir.bin`, sha256: manifest.datadir.sha256,
        byteLength: manifest.datadir.byte_length,
    }) + '\n'));
    return verifyDatadirCorpus(root, { retainedGenerations });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const [existing, source, build, output] = process.argv.slice(2);
    if (!output) throw new Error('Usage: add-full-replay-content.mjs EXISTING_CORPUS CONVERTER_OUTPUT BUILD NEW_CORPUS');
    await mkdir(output);
    await cp(existing, output, { recursive: true });
    const result = await addFullReplayContent({ corpus: output, source, build });
    console.log(`Verified Full replay corpus: ${result.assetCount} assets`);
}
