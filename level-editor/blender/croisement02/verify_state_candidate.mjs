/** Verify staged resource pins and exact native animation compilation. */
import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { compileSceneryAnimation } from '../../shared/src/compile-scenery-animation.ts';
const root = path.resolve(process.argv[2]);
const read = async p => JSON.parse(await fs.readFile(path.join(root, p), 'utf8'));
const source = await read('source/Croisement02.rhp.json');
const recipe = await read('animation-recipe.json');
const staged = await read('library/scenery-animation-assets.json');
assert.equal(staged.assetSources.length, source.animations.length);
const results = [];
for (const [index, entry] of recipe.entries.entries()) {
  const pin = staged.assetSources[index];
  const descriptor = await read(`library/${pin.descriptor}`);
  const bytes = await fs.readFile(path.join(root, 'library', pin.descriptor));
  assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), pin.descriptor_sha256);
  const actual = compileSceneryAnimation(descriptor.gameplay.animations[0], (_node, point) => point.map((v, i) => v + entry.origin[i]));
  assert.deepEqual(actual, source.animations[index], `Animation ${index} changed`);
  for (const resource of pin.resources) {
    const data = await fs.readFile(path.join(root, 'library', resource.path));
    assert.equal(crypto.createHash('sha256').update(data).digest('hex'), resource.sha256);
  }
  results.push({index, profile: actual.sprite.profile_name, exact_native_roundtrip: true, pinned_resources: pin.resources.length});
}
const preservation = await read('preservation.json');
for (const [file, sha] of Object.entries(preservation.files)) {
  const data = await fs.readFile(path.join(root, file));
  assert.equal(crypto.createHash('sha256').update(data).digest('hex'), sha, file);
}
const report = {status: 'PASS', animations: results, preserved_files: Object.keys(preservation.files).length, integration_status: 'NOT_INTEGRATED'};
const completeness = await read('completeness.json');
completeness.holds[0] = 'Exact native animation round-trip verified: verification.json; scene assembly remains pending.';
await fs.writeFile(path.join(root, 'completeness.json'), JSON.stringify(completeness, null, 2) + '\n');
await fs.writeFile(path.join(root, 'verification.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({status: report.status, animations: results.length, preserved_files: report.preserved_files}));
