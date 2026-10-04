// Freeze native animation and patch rows independently of asset grouping.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { loadKeyedFxPng, type RhsProfile } from '../../pipeline/src/fx.ts';

const root = process.cwd();
const output = process.argv[2];
if (!output) throw new Error('Usage: node extract_native_state_evidence.ts <fresh-output-directory>');
await fs.mkdir(output);
const levelPath = path.join(root, 'level-editor/work/york-refinement/baseline/york.rhp.json');
const levelBytes = await fs.readFile(levelPath);
const level = JSON.parse(levelBytes.toString());
const data = path.join(root, 'datadirs/fullgame_gog_hackable/Data/Animations/Day');
const banks = await fs.readdir(data);
const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
const requests = [
  ...level.animations.map((record: any, index: number) => ({ id: `animation-${String(index).padStart(3, '0')}`, kind: 'animation', record, sprite: record.sprite })),
  ...level.patches.map((record: any, index: number) => ({ id: `patch-${String(index).padStart(3, '0')}`, kind: 'patch', record, sprite: record.element_fx.sprite })),
];
const records = [];
let frameCount = 0;
for (const request of requests) {
  const sprite = request.sprite;
  if (sprite.frame_profile_name.toLowerCase() === 'pixel_vert') {
    records.push({ ...request, rows: [], status: 'nonvisual placeholder; retain state metadata' });
    continue;
  }
  const match = banks.find(bank => bank.toLowerCase() === `${sprite.frame_profile_name}.rhs.d`.toLowerCase());
  if (!match) throw new Error(`Missing native bank ${sprite.frame_profile_name}`);
  const directory = path.join(data, match);
  const manifestBytes = await fs.readFile(path.join(directory, 'manifest.json'));
  const manifest = JSON.parse(manifestBytes.toString()) as { profiles: RhsProfile[] };
  const profile = manifest.profiles.find(profile => profile.name === sprite.profile_name);
  if (!profile) throw new Error(`Missing native profile ${sprite.profile_name}`);
  const rows = [];
  for (const [rowIndex, row] of profile.rows.entries()) {
    const frames = [];
    for (const [frameIndex, frame] of row.frames.entries()) {
      let source = path.join(directory, profile.name, row.path, frame.file);
      try { await fs.access(source); }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        source = path.join(directory, row.path, frame.file);
      }
      const png = await loadKeyedFxPng(source);
      const image = `${request.id}/row-${rowIndex}/frame-${String(frameIndex).padStart(3, '0')}.png`;
      await fs.mkdir(path.dirname(path.join(output, image)), { recursive: true });
      await fs.writeFile(path.join(output, image), png);
      if (png.toString('ascii', 12, 16) !== 'IHDR') throw new Error('Missing PNG image header');
      const width = png.readUInt32BE(16), height = png.readUInt32BE(20);
      frames.push({ image, sha256: hash(png), source_sha256: hash(await fs.readFile(source)),
        source: path.relative(root, source), bbox: [sprite.position_x + frame.offset_x,
          sprite.position_y + frame.offset_y, width, height],
        ...frame });
      frameCount++;
    }
    if (!frames.length) throw new Error(`Empty animation row in ${profile.name}`);
    rows.push({ action: row.action, action_id: row.action_id,
      hotspot: [row.hotspot_x, row.hotspot_y], frames });
  }
  records.push({ ...request, bank_manifest_sha256: hash(manifestBytes),
    center: [profile.center_x, profile.center_y], size: [profile.width, profile.height], rows });
}
await fs.writeFile(path.join(output, 'manifest.json'), JSON.stringify({
  map: 'york', ambiance: 'Day', source_level_sha256: hash(levelBytes),
  status: 'native source evidence only; asset ownership and runtime state integration pending',
  placement: 'screen top-left = stored sprite position + frame offset',
  caveats: ['All rows are retained; patch validity flags determine which rows are used.',
    'Frame offsets and delays are metadata, not a validated runtime schedule.',
    'Day references do not establish mission-specific Fog or Night appearances.',
    'No geometry, texture, or state approval is implied.'], frame_count: frameCount, records,
}, null, 2) + '\n');
console.log(JSON.stringify({ output, records: records.length, frames: frameCount }));
