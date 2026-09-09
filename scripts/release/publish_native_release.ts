/** Publish a complete, hash-verified draft without replacing existing assets.
 * Runtime dependencies come from github-script; npm dependencies are type-only.
 */
import type * as actionsCore from '@actions/core';
import type * as actionsGithub from '@actions/github';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, readdir, readFile } from 'node:fs/promises';
import { basename, join } from 'node:path';

export type GitHubScriptContext = {
  core: typeof actionsCore;
  context: typeof actionsGithub.context;
  github: ReturnType<typeof actionsGithub.getOctokit>;
};
type GitHub = GitHubScriptContext['github'];
type Repo = { owner: string; repo: string };
type Release = Awaited<ReturnType<GitHub['rest']['repos']['getRelease']>>['data'];
export type Assets = Map<string, { path: string; sha256: string; size: number }>;

export async function inventory(root: string): Promise<Assets> {
  const assets: Assets = new Map();
  async function walk(path: string): Promise<void> {
    const stat = await lstat(path);
    if (stat.isSymbolicLink()) throw new Error(`release input contains a symlink: ${path}`);
    if (stat.isDirectory()) {
      for (const name of (await readdir(path)).sort()) await walk(join(path, name));
    } else if (stat.isFile()) {
      const name = basename(path);
      if (assets.has(name)) throw new Error(`duplicate release asset name: ${name}`);
      if (stat.size === 0) throw new Error(`empty release asset: ${path}`);
      const hash = createHash('sha256');
      for await (const chunk of createReadStream(path)) hash.update(chunk);
      assets.set(name, { path, sha256: hash.digest('hex'), size: stat.size });
    } else {
      throw new Error(`unsupported release input: ${path}`);
    }
  }
  await walk(root);
  for (const name of ['robin-windows-x86_64.zip', 'robin-linux-x86_64.tar.gz']) {
    if (!assets.has(name)) throw new Error(`missing platform artifact: ${name}`);
  }
  for (const runtime of ['win', 'linux']) {
    const indexName = `releases.${runtime}.json`;
    const indexAsset = assets.get(indexName);
    if (!indexAsset) throw new Error(`missing Velopack index: ${indexName}`);
    const index = JSON.parse(await readFile(indexAsset.path, 'utf8'));
    if (!Array.isArray(index?.Assets) || index.Assets.length === 0) {
      throw new Error(`empty/invalid Velopack index: ${indexName}`);
    }
    for (const entry of index.Assets) {
      const name = entry?.FileName;
      const asset = typeof name === 'string' ? assets.get(name) : undefined;
      if (!asset) throw new Error(`${indexName} references missing asset ${JSON.stringify(name)}`);
      if (entry.Size !== asset.size || String(entry.SHA256 ?? '').toLowerCase() !== asset.sha256) {
        throw new Error(`${indexName} size/hash differs for ${name}`);
      }
    }
  }
  return assets;
}

export function releaseTag(event: string, ref: string, timestamp: string, commit: string) {
  if (event === 'schedule' || event === 'workflow_dispatch') {
    return { tag: `nightly-${timestamp}-${commit.slice(0, 12)}`, prerelease: true };
  }
  if (!ref.startsWith('v')) throw new Error('stable releases require a version tag');
  return { tag: ref, prerelease: false };
}

export async function runTimestamp(github: GitHub, repo: Repo, runId: number): Promise<string> {
  const { data } = await github.rest.actions.getWorkflowRun({ ...repo, run_id: runId });
  if (!/(Z|[+-]\d{2}:\d{2})$/.test(data.created_at)) {
    throw new Error('workflow created_at must include a timezone');
  }
  const created = new Date(data.created_at);
  if (Number.isNaN(created.getTime())) throw new Error('invalid workflow created_at');
  return created.toISOString().slice(0, 16).replace(/[-T:]/g, '');
}

export async function findRelease(github: GitHub, repo: Repo, tag: string) {
  // Auth/network errors propagate; they must never mean "not found".
  for await (const page of github.paginate.iterator(github.rest.repos.listReleases, { ...repo, per_page: 100 })) {
    const release = page.data.find(release => release.tag_name === tag);
    if (release) return release;
  }
  return undefined;
}

export async function verifyRemote(github: GitHub, repo: Repo, release: Release, assets: Assets, allowMissing: boolean) {
  const remote = new Map(release.assets.map(asset => [asset.name, asset]));
  for (const name of remote.keys()) {
    if (!assets.has(name)) throw new Error(`release contains unexpected asset: ${name}`);
  }
  const missing = [...assets.keys()].filter(name => !remote.has(name)).sort();
  if (missing.length && !allowMissing) throw new Error(`release is missing assets: ${missing.join(', ')}`);
  for (const [name, asset] of remote) {
    const { data } = await github.rest.repos.getReleaseAsset({
      ...repo, asset_id: asset.id, headers: { accept: 'application/octet-stream' },
    });
    // Octokit's endpoint types describe JSON metadata, while this Accept header
    // returns binary bytes. Reject metadata/error bodies instead of hashing them.
    const bytes: unknown = data;
    if (!(bytes instanceof ArrayBuffer) && !Buffer.isBuffer(bytes)) {
      throw new Error(`expected binary download for release asset: ${name}`);
    }
    const digest = createHash('sha256').update(bytes instanceof ArrayBuffer ? Buffer.from(bytes) : bytes).digest('hex');
    if (digest !== assets.get(name)!.sha256) {
      throw new Error(`immutable release asset differs: ${name}; use a new candidate tag`);
    }
  }
  return missing;
}

export async function publish(
  github: GitHub, core: Pick<GitHubScriptContext['core'], 'info'>,
  root: string, repo: Repo, tag: string, commit: string, prerelease: boolean,
) {
  const assets = await inventory(root);
  let release = await findRelease(github, repo, tag);
  if (!release) {
    if (!prerelease) await github.rest.git.getRef({ ...repo, ref: `tags/${tag}` });
    // Keep the creation response; the list may not yet contain the new draft.
    ({ data: release } = await github.rest.repos.createRelease({
      ...repo, tag_name: tag, target_commitish: commit, name: tag,
      body: `Build of ${commit}. Assets verified before publication.`, draft: true, prerelease,
    }));
  }
  if (release.target_commitish !== commit) throw new Error('release target differs from the requested commit');
  const missing = await verifyRemote(github, repo, release, assets, release.draft);
  if (!release.draft) {
    core.info(`${tag} is already published with identical assets`);
    return;
  }
  const releaseId = release.id;
  for (const name of missing) {
    const asset = assets.get(name)!;
    // TODO: Stream large uploads/downloads when Octokit's endpoint typings and
    // fetch transport support it together; currently buffer one asset at a time.
    await github.rest.repos.uploadReleaseAsset({
      ...repo, release_id: releaseId,
      // Preserve the URI template so Octokit expands the name query parameter.
      url: release.upload_url,
      name, headers: { 'content-type': 'application/octet-stream', 'content-length': asset.size },
      // The REST schema types the binary body as string; Octokit accepts Buffer.
      data: await readFile(asset.path) as unknown as string,
    });
  }
  ({ data: release } = await github.rest.repos.getRelease({ ...repo, release_id: releaseId }));
  await verifyRemote(github, repo, release, assets, false);
  await github.rest.repos.updateRelease({ ...repo, release_id: releaseId, draft: false });
  core.info(`published verified candidate ${tag}`);
}

export async function packageTimestamp({ core, context, github }: GitHubScriptContext) {
  core.setOutput('timestamp', await runTimestamp(github, context.repo, context.runId));
}

export async function main({ core, context, github }: GitHubScriptContext) {
  const timestamp = ['schedule', 'workflow_dispatch'].includes(context.eventName)
    ? await runTimestamp(github, context.repo, context.runId) : '';
  const ref = context.ref.replace(/^refs\/(tags|heads)\//, '');
  const { tag, prerelease } = releaseTag(context.eventName, ref, timestamp, context.sha);
  await publish(github, core, 'release-assets', context.repo, tag, context.sha, prerelease);
}
