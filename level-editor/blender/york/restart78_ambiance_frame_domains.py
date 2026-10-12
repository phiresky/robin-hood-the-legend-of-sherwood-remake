"""Compare source frame dimensions and encoded PNG alpha before sharing UV rectangles."""
from pathlib import Path
import hashlib
import json
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'datadirs/fullgame_gog_hackable/Data/Animations'
OUT = ROOT / 'level-editor/work/active/york/restart2/restart78-ambiance-frame-domains-v2'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    OUT.mkdir(exist_ok=False)
    banks = ['Yorkpatch', 'yorkfx', 'York_crow', 'York_pigeon', 'pixel_vert']
    rows = []
    pins = {}
    for bank in banks:
        day = SOURCE / 'Day' / (bank + '.rhs.d')
        manifest = day / 'manifest.json'
        pins[str(manifest.relative_to(ROOT))] = sha(manifest)
        for variant in ['Fog', 'Night']:
            other = SOURCE / variant / (bank + '.rhs.d')
            if not other.exists():
                rows.append(dict(bank=bank, ambiance=variant, fallback='Day', reason='Requested bank absent'))
                continue
            variant_manifest = other / 'manifest.json'
            pins[str(variant_manifest.relative_to(ROOT))] = sha(variant_manifest)
            for file in sorted(day.rglob('*.png')):
                relative = file.relative_to(day)
                candidate = other / relative
                if not candidate.exists():
                    raise ValueError(f'Missing corresponding frame {candidate}')
                a = Image.open(file).convert('RGBA')
                b = Image.open(candidate).convert('RGBA')
                alpha_equal = a.size == b.size and a.getchannel('A').tobytes() == b.getchannel('A').tobytes()
                size = (max(a.width, b.width), max(a.height, b.height))
                aa, bb = Image.new('L', size), Image.new('L', size)
                aa.paste(a.getchannel('A'), (0, 0))
                bb.paste(b.getchannel('A'), (0, 0))
                aligned_alpha_equal = aa.tobytes() == bb.tobytes()
                rows.append(dict(bank=bank, ambiance=variant, frame=str(relative), day_size=list(a.size), variant_size=list(b.size), dimensions_equal=a.size == b.size, alpha_equal=alpha_equal, aligned_alpha_equal=aligned_alpha_equal, rgba_equal=a.size == b.size and a.tobytes() == b.tobytes(), day_sha256=sha(file), variant_sha256=sha(candidate)))
    groups = []
    for bank in banks:
        for variant in ['Fog', 'Night']:
            frames = [r for r in rows if r['bank'] == bank and r['ambiance'] == variant]
            if frames and frames[0].get('fallback'):
                groups.append(frames[0])
                continue
            groups.append(dict(bank=bank, ambiance=variant, frames=len(frames), changed_dimensions=sum(not r['dimensions_equal'] for r in frames), changed_alpha=sum(not r['alpha_equal'] for r in frames), changed_aligned_alpha=sum(not r['aligned_alpha_equal'] for r in frames), changed_rgba=sum(not r['rgba_equal'] for r in frames)))
    report = dict(status='SOURCE_DOMAIN_COMPARISON_NOT_GEOMETRY_APPROVAL', summary=groups, frames=rows, source_manifests=pins, implications=['Different dimensions require per-ambiance UV/source rectangles.', 'Encoded PNG alpha is not decoded native transparency: color-key pixels may remain opaque. Decode source keys before silhouette conclusions.', 'Identical source pixels do not certify mission or gameplay integration.'])
    (OUT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(groups, indent=2))

if __name__ == '__main__':
    main()
