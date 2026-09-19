import assert from 'node:assert/strict';
import { test } from 'node:test';
import { compareVersions, feedVersions, verifyPackageVersion } from './package_version.ts';
import type { GitHubScriptContext } from './publish_native_release.ts';

const repo = { owner: 'owner', repo: 'repo' };
function api(versions: string[]) {
  const releases = versions.map((version, id) => ({
    draft: false, tag_name: `published-${id}`,
    assets: [{ name: id % 2 ? 'releases.win.json' : 'releases.linux.json', id }],
  }));
  let failed = false;
  const client = {
    paginate: { async *iterator() {
      if (failed) throw new Error('API unavailable');
      for (const release of releases) yield { data: [release] };
    } },
    rest: { repos: {
      listReleases() {},
      async getReleaseAsset({ asset_id }: { asset_id: number }) {
        return { data: Buffer.from(JSON.stringify({ Assets: [{ Version: versions[asset_id] }] })) };
      },
    } },
  };
  return {
    github: client as unknown as GitHubScriptContext['github'], releases,
    fail() { failed = true; },
  };
}

test('SemVer precedence handles nightlies, stable releases and metadata', () => {
  const ordered = ['0.0.1', '0.1.0-alpha', '0.1.0-nightly.9', '0.1.0-nightly.10',
    '0.1.0-nightly.202609110808', '0.1.0-nightly.202609171419', '0.1.0', '0.1.1-nightly.1'];
  for (let i = 1; i < ordered.length; i++) assert.equal(compareVersions(ordered[i]!, ordered[i - 1]!), 1);
  assert.equal(compareVersions('0.1.0+abc', '0.1.0+def'), 0);
  for (const invalid of ['0.01.0', '0.1.0-nightly.01', '0.1', 'v0.1.0', '0.1.0-']) {
    assert.throws(() => compareVersions(invalid, '0.1.0'), /Invalid/);
  }
});

test('zero workspace version and downgrades cannot become packages', async () => {
  const { github } = api(['0.1.0-nightly.202609110808']);
  for (const version of ['0.0.0-nightly.202609171419+c013beafcde4', '0.0.1-nightly.1']) {
    await assert.rejects(verifyPackageVersion(github, repo, version, 'new'), /minimum/);
  }
  for (const version of ['0.0.2', '0.1.0-nightly.202609100000', '0.1.0-nightly.202609110808+different']) {
    await assert.rejects(verifyPackageVersion(github, repo, version, 'new'), /must be newer/);
  }
  await verifyPackageVersion(github, repo, '0.1.0-nightly.202609171419+c013beafcde4', 'new');
});

test('every published platform and page contributes to the version floor', async () => {
  const { github, releases } = api(['0.1.0-nightly.1', '0.2.0', '0.1.0-nightly.2']);
  await assert.rejects(verifyPackageVersion(github, repo, '0.1.0-nightly.3', 'new'), /published 0.2.0/);
  releases[1]!.draft = true;
  await verifyPackageVersion(github, repo, '0.1.0-nightly.3', 'new');
});

test('same candidate retry is allowed but API failures are not ignored', async () => {
  const a = api(['0.1.0']);
  await verifyPackageVersion(a.github, repo, '0.1.0', 'published-0');
  a.fail();
  await assert.rejects(verifyPackageVersion(a.github, repo, '0.2.0', 'new'), /API unavailable/);
});

test('malformed update feeds fail closed', () => {
  for (const value of ['null', '{}', '{"Assets":[]}', '{"Assets":[{}]}', '{"Assets":[{"Version":"bad"}]}']) {
    assert.throws(() => feedVersions(value));
  }
});
