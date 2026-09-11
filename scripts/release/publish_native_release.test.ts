import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { stageReleaseAssets, inventory, main, packageTimestamp, publish, releaseTag, runTimestamp, type GitHubScriptContext } from './publish_native_release.ts';

const repo = { owner: 'owner', repo: 'repo' };
const core = { info() {} };
const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');

async function fixture(t: { after(fn: () => Promise<void>): void }) {
  const root = await mkdtemp(join(tmpdir(), 'native-release-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const name of ['robinhood-remake-windows-Setup.exe', 'robinhood-remake-windows-Portable.zip', 'robinhood-remake-linux.AppImage', 'game package.nupkg']) {
    await writeFile(join(root, name), 'package');
  }
  for (const runtime of ['win', 'linux']) {
    await writeFile(join(root, `releases.${runtime}.json`), JSON.stringify({ Assets: [{
      FileName: 'game package.nupkg', Size: 7, SHA256: hash(Buffer.from('package')),
    }] }));
  }
  return root;
}

function api() {
  const calls: string[] = [];
  const bytes = new Map<number, Buffer>();
  const draft = {
    id: 42, tag_name: 'v1', target_commitish: 'commit', draft: true,
    upload_url: 'https://uploads.github.com/repos/owner/repo/releases/42/assets{?name,label}',
    assets: [] as { name: string; id: number }[],
  };
  const state = {
    listed: false, createdAt: '2026-09-07T23:59:42Z',
    listError: false, tagError: false, corrupt: false, omit: false, promoted: false,
  };
  const client = {
    paginate: {
      async *iterator() {
        calls.push('list');
        if (state.listError) throw new Error('authentication failed');
        yield { data: [] };
        yield { data: state.listed ? [draft] : [] };
      },
    },
    rest: {
      actions: {
        async getWorkflowRun(args: { run_id: number }) {
          assert.equal(args.run_id, 99);
          return { data: { created_at: state.createdAt } };
        },
      },
      git: {
        async getRef(args: { ref: string }) {
          calls.push('tag');
          assert.equal(args.ref, 'tags/v1');
          if (state.tagError) throw new Error('tag not found');
          return { data: {} };
        },
      },
      repos: {
        listReleases() { throw new Error('use pagination'); },
        async createRelease(args: { draft: boolean; target_commitish: string; prerelease: boolean }) {
          calls.push('create');
          assert.equal(args.draft, true);
          assert.equal(args.target_commitish, 'commit');
          return { data: draft };
        },
        async uploadReleaseAsset(args: { release_id: number; name: string; data: Buffer; url: string }) {
          calls.push('upload');
          assert.equal(args.release_id, 42);
          assert.equal(args.url, draft.upload_url);
          assert.ok(Buffer.isBuffer(args.data));
          const id = bytes.size + 1;
          bytes.set(id, args.data);
          if (!state.omit) draft.assets.push({ name: args.name, id });
          return { data: {} };
        },
        async getRelease(args: { release_id: number }) {
          calls.push('get');
          assert.equal(args.release_id, 42);
          return { data: draft };
        },
        async getReleaseAsset(args: { asset_id: number; headers: { accept: string } }) {
          calls.push('download');
          assert.equal(args.headers.accept, 'application/octet-stream');
          const body = state.corrupt ? Buffer.from('wrong') : bytes.get(args.asset_id);
          assert.ok(body);
          return { data: Uint8Array.from(body).buffer };
        },
        async updateRelease(args: { release_id: number; draft: boolean }) {
          calls.push('promote');
          assert.equal(args.release_id, 42);
          assert.equal(args.draft, false);
          state.promoted = true;
          return { data: draft };
        },
      },
    },
  };
  // Only the exercised API surface is implemented; production uses full Octokit types.
  return { github: client as unknown as GitHubScriptContext['github'], client, draft, bytes, state, calls };
}

test('nightly tags preserve commit hash; stable tags require v prefix', () => {
  for (const event of ['schedule', 'workflow_dispatch']) {
    assert.deepEqual(releaseTag(event, 'main', '202609072359', 'd01eb1295003abcdef'),
      { tag: 'nightly-2026-09-07-2359-d01eb1295003', prerelease: true });
  }
  assert.deepEqual(releaseTag('push', 'v1.2.3', '', 'commit'), { tag: 'v1.2.3', prerelease: false });
  assert.throws(() => releaseTag('push', 'main', '', 'commit'), /version tag/);
});

test('timestamp uses original workflow creation time and normalizes timezone', async () => {
  const { github, state } = api();
  assert.equal(await runTimestamp(github, repo, 99), '202609072359');
  state.createdAt = '2026-09-08T02:05:00+02:00';
  assert.equal(await runTimestamp(github, repo, 99), '202609080005');
  state.createdAt = '2026-09-08T02:05:00';
  await assert.rejects(runTimestamp(github, repo, 99), /timezone/);
  state.createdAt = 'invalidZ';
  await assert.rejects(runTimestamp(github, repo, 99), /invalid/);
});

test('inventory requires complete, hash-verified update indexes', async t => {
  const root = await fixture(t);
  assert.equal((await inventory(root)).size, 6);
  await writeFile(join(root, 'game package.nupkg'), 'tampered');
  await assert.rejects(inventory(root), /size\/hash differs/);
  await rm(join(root, 'game package.nupkg'));
  await assert.rejects(inventory(root), /missing asset/);
});

test('inventory rejects duplicate names, symlinks, empty assets and missing indexes', async t => {
  const root = await fixture(t);
  await mkdir(join(root, 'other'));
  await writeFile(join(root, 'other', 'game package.nupkg'), 'package');
  await assert.rejects(inventory(root), /duplicate/);
  await rm(join(root, 'other'), { recursive: true });
  await symlink(join(root, 'game package.nupkg'), join(root, 'link'));
  await assert.rejects(inventory(root), /symlink/);
  await rm(join(root, 'link'));
  await writeFile(join(root, 'empty'), '');
  await assert.rejects(inventory(root), /empty release asset/);
  await rm(join(root, 'empty'));
  await writeFile(join(root, 'releases.win.json'), '{"Assets":[]}');
  await assert.rejects(inventory(root), /invalid Velopack/);
  await rm(join(root, 'releases.win.json'));
  await assert.rejects(inventory(root), /missing Velopack/);
});

test('new draft can remain absent from listing through upload and promotion', async t => {
  const root = await fixture(t);
  for (const prerelease of [true, false]) {
    const a = api();
    await publish(a.github, core, root, repo, 'v1', 'commit', prerelease);
    assert.equal(a.calls.filter(call => call === 'list').length, 1);
    assert.equal(a.calls.includes('tag'), !prerelease);
    assert.equal(a.calls.at(-1), 'promote');
    assert.equal(a.calls.filter(call => call === 'download').length, 6);
    assert.equal(a.state.listed, false);
  }
});

test('existing draft resumes only missing uploads; published release stays immutable', async t => {
  const root = await fixture(t);
  const a = api();
  a.state.listed = true;
  a.bytes.set(1, Buffer.from('package'));
  a.draft.assets.push({ name: 'game package.nupkg', id: 1 });
  await publish(a.github, core, root, repo, 'v1', 'commit', true);
  assert.equal(a.calls.includes('create'), false);
  assert.equal(a.calls.filter(call => call === 'upload').length, 5);
  a.draft.draft = false;
  a.calls.length = 0;
  await publish(a.github, core, root, repo, 'v1', 'commit', false);
  assert.deepEqual(a.calls, ['list', 'download', 'download', 'download', 'download', 'download', 'download']);
});

test('mismatched bytes or missing uploaded assets block promotion', async t => {
  const root = await fixture(t);
  for (const mode of ['corrupt', 'omit'] as const) {
    const a = api();
    a.state[mode] = true;
    await assert.rejects(publish(a.github, core, root, repo, 'v1', 'commit', true),
      mode === 'corrupt' ? /immutable release asset differs/ : /missing assets/);
    assert.equal(a.state.promoted, false);
  }
});

test('wrong commit, unexpected assets, auth failures and missing stable tags fail closed', async t => {
  const root = await fixture(t);
  for (const mode of ['commit', 'extra', 'auth', 'tag']) {
    const a = api();
    if (mode === 'commit') { a.state.listed = true; a.draft.target_commitish = 'other'; }
    if (mode === 'extra') { a.state.listed = true; a.draft.assets.push({ id: 1, name: 'extra' }); }
    if (mode === 'auth') a.state.listError = true;
    if (mode === 'tag') a.state.tagError = true;
    await assert.rejects(publish(a.github, core, root, repo, 'v1', 'commit', false));
    assert.equal(a.calls.includes('create'), false);
    assert.equal(a.state.promoted, false);
  }
});

test('github-script entry points load through require without a build', async () => {
  const { createRequire } = await import('node:module');
  const module = createRequire(import.meta.url)('./publish_native_release.ts');
  assert.equal(module.main, main);
  assert.equal(module.packageTimestamp, packageTimestamp);
  const a = api();
  const outputs: unknown[] = [];
  await packageTimestamp({
    github: a.github, context: { repo, runId: 99 },
    core: { setOutput: (...args: unknown[]) => outputs.push(args) },
  } as unknown as GitHubScriptContext);
  assert.deepEqual(outputs, [['timestamp', '202609072359']]);
});

test('workflow imports TypeScript directly for timestamp and publication', async () => {
  const workflow = await readFile(new URL('../../.github/workflows/native-release.yml', import.meta.url), 'utf8');
  assert.match(workflow, /require\('\.\/scripts\/release\/publish_native_release\.ts'\)\.main/);
  assert.match(workflow, /require\('\.\/scripts\/release\/publish_native_release\.ts'\)\.packageTimestamp/);
  assert.doesNotMatch(workflow, /publish_native_release\.py/);
});


test('real Octokit uploads raw bytes to returned URL and decodes binary downloads', async t => {
  const { getOctokit } = await import('@actions/github');
  const root = await fixture(t);
  const uploaded = new Map<string, Buffer>();
  const draft = {
    id: 42, tag_name: 'v1', target_commitish: 'commit', draft: true,
    upload_url: 'https://uploads.github.com/repos/owner/repo/releases/42/assets{?name,label}',
    assets: [] as { id: number; name: string }[],
  };
  let promoted = false;
  const github = getOctokit('test-token', {
    request: {
      fetch: async (input: string | URL | Request, init?: RequestInit) => {
        const request = new Request(input, init);
        const url = new URL(request.url);
        assert.equal(request.headers.get('authorization'), 'token test-token');
        if (url.hostname === 'uploads.github.com') {
          assert.equal(request.method, 'POST');
          assert.equal(url.pathname, '/repos/owner/repo/releases/42/assets');
          assert.equal(request.headers.get('content-type'), 'application/octet-stream');
          const name = url.searchParams.get('name');
          assert.ok(name);
          const bytes = Buffer.from(await request.arrayBuffer());
          assert.equal(request.headers.get('content-length'), String(bytes.length));
          uploaded.set(name, bytes);
          draft.assets.push({ id: uploaded.size, name });
          return Response.json({}, { status: 201 });
        }
        assert.equal(url.hostname, 'api.github.com');
        if (url.pathname.endsWith('/releases')) {
          if (request.method === 'GET') return Response.json([]);
          assert.equal(request.method, 'POST');
          assert.equal((await request.json()).draft, true);
          return Response.json(draft, { status: 201 });
        }
        if (url.pathname.includes('/releases/assets/')) {
          const asset = draft.assets.find(asset => asset.id === Number(url.pathname.split('/').at(-1)));
          assert.ok(asset);
          assert.equal(request.headers.get('accept'), 'application/octet-stream');
          return new Response(Uint8Array.from(uploaded.get(asset.name)!), {
            headers: { 'content-type': 'application/octet-stream' },
          });
        }
        assert.equal(url.pathname, '/repos/owner/repo/releases/42');
        if (request.method === 'PATCH') {
          assert.equal((await request.json()).draft, false);
          promoted = true;
        } else {
          assert.equal(request.method, 'GET');
        }
        return Response.json(draft);
      },
    },
  });
  await publish(github, core, root, repo, 'v1', 'commit', true);
  assert.equal(uploaded.size, 6);
  assert.equal(promoted, true);
});

test('inventory requires each user-facing download without raw archives', async t => {
  for (const name of ['robinhood-remake-windows-Setup.exe', 'robinhood-remake-windows-Portable.zip', 'robinhood-remake-linux.AppImage']) {
    const root = await fixture(t);
    const assets = await inventory(root);
    assert.equal(assets.has('robin-windows-x86_64.zip'), false);
    assert.equal(assets.has('robin-linux-x86_64.tar.gz'), false);
    await rm(join(root, name));
    await assert.rejects(inventory(root), /missing platform artifact/);
  }
});

test('staging renames downloads and both update packages without changing identity or hashes', async t => {
  const root = await mkdtemp(join(tmpdir(), 'native-staging-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const output = join(root, 'output');
  for (const channel of ['win', 'linux'] as const) {
    const input = join(root, channel);
    await mkdir(input);
    const packageId = 'io.github.phiresky.robinhood';
    const original = `${packageId}-1.2.3${channel === 'linux' ? '-linux' : ''}-full.nupkg`;
    const entry = { PackageId: packageId, Version: '1.2.3', Type: 'Full',
      FileName: original, Size: 7, SHA256: hash(Buffer.from('package')) };
    await writeFile(join(input, original), 'package');
    await writeFile(join(input, `releases.${channel}.json`), JSON.stringify({ Assets: [entry] }));
    const downloads = channel === 'win'
      ? [`${packageId}-win-Setup.exe`, `${packageId}-win-Portable.zip`]
      : [`${packageId}.AppImage`];
    for (const name of [...downloads, 'RELEASES', `assets.${channel}.json`]) {
      await writeFile(join(input, name), 'payload');
    }
    await stageReleaseAssets(input, output, channel);
    const renamed = `robinhood-remake-1.2.3-${channel === 'win' ? 'windows' : 'linux'}-full.nupkg`;
    const index = JSON.parse(await readFile(join(output, `releases.${channel}.json`), 'utf8'));
    assert.deepEqual(index.Assets, [{ ...entry, FileName: renamed }]);
    assert.equal(await readFile(join(output, renamed), 'utf8'), 'package');
  }
  const assets = await inventory(output);
  assert.equal(assets.size, 7);
  assert.equal(assets.has('RELEASES'), false);
  assert.equal(assets.has('assets.win.json'), false);
});

test('normal release packages contain every modding binary on both platforms', async t => {
  const workflow = await readFile(new URL('../../.github/workflows/native-release.yml', import.meta.url), 'utf8');
  assert.match(workflow, /cargo build --locked --release -p robin_modding_tools --bins --features robin_rs\/release/);
  const stage = workflow.split('      - name: Stage package input\n')[1]
    .split('\n      - name:')[0].split('        run: |\n')[1]
    .split('\n').map(line => line.replace(/^          /, '')).join('\n');
  const binaries = (await readdir(new URL('../../crates/robin_modding_tools/src/bin/', import.meta.url)))
    .filter(name => name.endsWith('.rs')).map(name => name.slice(0, -3));
  assert.equal(binaries.length, 4);
  assert.match(workflow, /cargo build --locked --release -p robin_replay_format --features native-admission --bin robin-replay-admission/);
  binaries.push('robin-replay-admission');
  for (const runtime of ['win-x64', 'linux-x64']) {
    const root = await mkdtemp(join(tmpdir(), 'modding-package-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    const suffix = runtime === 'win-x64' ? '.exe' : '';
    const target = runtime === 'win-x64' ? 'x86_64-pc-windows-gnu' : 'x86_64-unknown-linux-gnu';
    const executable = 'robin' + suffix;
    const packaged = runtime === 'win-x64' ? 'Robin Hood - The Legend of Sherwood.exe' : 'robin';
    await mkdir(join(root, 'target', target, 'release'), { recursive: true });
    await mkdir(join(root, 'assets/core-datadir'), { recursive: true });
    await mkdir(join(root, 'docs'));
    await writeFile(join(root, 'README.md'), 'readme');
    for (const name of ['MODDING_TOOLS.md', 'JSON_PATCH_MODS.md']) {
      await writeFile(join(root, 'docs', name), name);
    }
    for (const name of [executable, ...binaries.map(name => name + suffix)]) {
      await writeFile(join(root, 'target', target, 'release', name), name, { mode: 0o755 });
    }
    const script = stage
      .replaceAll('${{ matrix.runtime }}', runtime)
      .replaceAll('${{ matrix.target }}', target)
      .replaceAll('${{ matrix.executable }}', executable)
      .replaceAll('${{ matrix.pack_executable }}', packaged);
    execFileSync('bash', ['-e', '-c', script], { cwd: root });
    for (const name of [...binaries.map(name => name + suffix), packaged]) {
      assert.equal(await readFile(join(root, 'target/package-input', name), 'utf8'), name === packaged ? executable : name);
    }
    assert.equal(await readFile(join(root, 'target/package-input/docs/MODDING_TOOLS.md'), 'utf8'), 'MODDING_TOOLS.md');
  }
});
