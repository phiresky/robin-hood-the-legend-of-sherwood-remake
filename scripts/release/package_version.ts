/** Native package versions are independent of unpublished workspace crate versions. */
import { readFile } from 'node:fs/promises';
import type { GitHubScriptContext } from './publish_native_release.ts';

type GitHub = GitHubScriptContext['github'];
type Repo = GitHubScriptContext['context']['repo'];

function parseVersion(version: string) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(version);
  if (!match) throw new Error(`Invalid package version: ${version}`);
  const pre = match[4]?.split('.') ?? [];
  if (pre.some(part => /^\d+$/.test(part) && part.length > 1 && part.startsWith('0'))) {
    throw new Error(`Invalid numeric prerelease identifier: ${version}`);
  }
  return { core: match.slice(1, 4).map(BigInt), pre };
}

/** SemVer precedence; build metadata never makes an update newer. */
export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left), b = parseVersion(right);
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) return a.core[i]! < b.core[i]! ? -1 : 1;
  }
  if (!a.pre.length || !b.pre.length) return Number(!a.pre.length) - Number(!b.pre.length);
  for (let i = 0; i < Math.max(a.pre.length, b.pre.length); i++) {
    const x = a.pre[i], y = b.pre[i];
    if (x === y) continue;
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    const xn = /^\d+$/.test(x), yn = /^\d+$/.test(y);
    if (xn && yn) return BigInt(x) < BigInt(y) ? -1 : 1;
    if (xn !== yn) return xn ? -1 : 1;
    return x < y ? -1 : 1;
  }
  return 0;
}

export function feedVersions(bytes: string): string[] {
  const index: unknown = JSON.parse(bytes);
  const assets = (index as { Assets?: unknown } | null)?.Assets;
  if (!Array.isArray(assets) || !assets.length) throw new Error('Empty or invalid Velopack update feed');
  return assets.map(asset => {
    if (typeof asset?.Version !== 'string') throw new Error('Missing package Version in update feed');
    parseVersion(asset.Version);
    return asset.Version;
  });
}

/** Fail closed on API/feed errors; a failed lookup cannot mean no prior version. */
export async function verifyPackageVersion(
  github: GitHub, repo: Repo, candidate: string, candidateTag: string,
): Promise<void> {
  if (compareVersions(candidate, '0.0.1') < 0) {
    throw new Error(`Package version ${candidate} is below Velopack's minimum 0.0.1`);
  }
  for await (const page of github.paginate.iterator(github.rest.repos.listReleases, { ...repo, per_page: 100 })) {
    for (const release of page.data) {
      // A retry of the same candidate is verified byte-for-byte by the publisher.
      if (release.draft || release.tag_name === candidateTag) continue;
      for (const asset of release.assets) {
        if (!/^releases\.(win|linux)\.json$/.test(asset.name)) continue;
        const { data } = await github.rest.repos.getReleaseAsset({
          ...repo, asset_id: asset.id, headers: { accept: 'application/octet-stream' },
        });
        const bytes: unknown = data;
        if (!(bytes instanceof ArrayBuffer) && !Buffer.isBuffer(bytes)) {
          throw new Error(`Expected binary download for update feed ${release.tag_name}/${asset.name}`);
        }
        const content = (bytes instanceof ArrayBuffer ? Buffer.from(bytes) : bytes).toString('utf8');
        for (const published of feedVersions(content)) {
          if (compareVersions(candidate, published) <= 0) {
            throw new Error(`Package version ${candidate} must be newer than published ${published} (${release.tag_name}/${asset.name}); bump the game version or use a newer nightly`);
          }
        }
      }
    }
  }
}

/** Check the actual packaged versions again immediately before promotion. */
export async function verifyStagedPackageVersion(
  github: GitHub, repo: Repo, assets: Map<string, { path: string }>, candidateTag: string,
) {
  const versions = new Set<string>();
  for (const channel of ['win', 'linux']) {
    const asset = assets.get(`releases.${channel}.json`);
    if (!asset) throw new Error(`Missing ${channel} package feed`);
    for (const version of feedVersions(await readFile(asset.path, 'utf8'))) versions.add(version);
  }
  if (versions.size !== 1) throw new Error('Platform packages must contain the same version');
  await verifyPackageVersion(github, repo, [...versions][0]!, candidateTag);
}
