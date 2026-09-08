import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const VERSION_ID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u;

export function extractWranglerDeployVersion(text, expectedWorker) {
    const events = [];
    for (const [index, line] of text.split('\n').entries()) {
        if (line.length === 0) continue;
        let event;
        try {
            event = JSON.parse(line);
        } catch (error) {
            throw new Error(`Wrangler output line ${index + 1} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
        }
        if (event?.type === 'deploy') events.push(event);
    }
    if (events.length !== 1) {
        throw new Error(`Wrangler output must contain exactly one deploy event, found ${events.length}`);
    }
    const [deployment] = events;
    if (deployment.worker_name !== expectedWorker) {
        throw new Error(`Wrangler deployed ${JSON.stringify(deployment.worker_name)}, expected ${JSON.stringify(expectedWorker)}`);
    }
    if (!VERSION_ID.test(deployment.version_id)) {
        throw new Error('Wrangler deploy event has no canonical Worker version ID');
    }
    return deployment.version_id;
}

async function main() {
    const [outputPath, expectedWorker, extra] = process.argv.slice(2);
    if (outputPath === undefined || expectedWorker === undefined || extra !== undefined) {
        throw new Error('usage: extract-wrangler-deploy-version.mjs WRANGLER_OUTPUT EXPECTED_WORKER');
    }
    const text = await readFile(outputPath, 'utf8');
    console.log(extractWranglerDeployVersion(text, expectedWorker));
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
    main().catch(error => {
        console.error(error instanceof Error ? error.message : String(error));
        process.exitCode = 1;
    });
}
