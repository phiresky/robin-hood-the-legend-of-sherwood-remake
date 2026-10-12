"""Freeze the complete four-state hanging-cover geometry as one grouped-review card."""
import html
import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qualify as q


def main():
    dest = q.OUT / 'profile03-geometry-card-v1'
    assert not dest.exists()
    construction_path = q.OUT / 'geometry-v9-short-support03-v1/report.json'
    construction = json.loads(construction_path.read_text())
    views_dir = q.OUT / 'geometry-v9-short-support03-review-v1'
    views = json.loads((views_dir / 'self-review.json').read_text())
    junction_dir = q.OUT / 'support03-junction-sections-v1'
    junctions = json.loads((junction_dir / 'report.json').read_text())
    sections = json.loads((junction_dir / 'self-review.json').read_text())
    assert views['status'] == 'ALL_64_SAVED_VIEWS_SELF_REVIEWED'
    assert sections['status'] == 'ALL_16_SECTIONS_SELF_REVIEWED'
    assert construction['model'] == views['model'] == sections['model'] == junctions['model']
    assert q.pin(q.ROOT / construction['model']['path']) == construction['model']
    assert len(junctions['reviews']) == 8
    dest.mkdir()
    pins = [construction['model']]
    def img(path, label):
        pins.append(q.pin(path))
        rel = html.escape(os.path.relpath(path, dest))
        return f'<figure><figcaption>{html.escape(label)}</figcaption><a href="{rel}"><img loading="lazy" src="{rel}"></a></figure>'
    parts = ['<h1>Hidden archer03 foliage cover</h1><p>Geometry only: four authored poses of the cover between the existing trunks, with two short inferred attachments per pose. Original camera is top-left in every model sheet. Gray unknown surfaces still need texture fill.</p>']
    parts.append(img(q.OUT / 'archer03-source-context.png', 'Original artwork: background, initial, two transition poses and integrated terminal'))
    parts.append('<p>This is shallow hanging foliage, not a complete new tree crown. The cover opens downward in the source sequence. Eight supports bridge gaps of 1.7–3.9 units to the neighboring wood; their complete finite volumes cross no other receiver. Eighteen tiny source leaf islands, totaling 84 pixels across the four poses, are retained without invented individual twigs. The native holds remain discrete; no continuous deformation is asserted.</p>')
    for phase in ['initial','transition-0','transition-1','applied']:
        parts.append('<h2>' + html.escape(phase) + '</h2>')
        for mode in ['textured','solid']:
            parts.append(img(views_dir / phase / f'{mode}-eight.png', f'Saved model — {mode}, eight views'))
    parts.append('<details><summary>Measured attachment joins: exact saved mesh sections</summary><p>Orange is the actual saved support, green is foliage and brown is the target wood. Both perpendicular slices show connected endpoints. These are geometry diagrams, not texture previews. A separate full-map finite-volume check covers the other receivers.</p>')
    for row in junctions['reviews']:
        path = q.ROOT / row['sheet']['path']
        assert q.pin(path) == row['sheet']
        parts.append(img(path, row['phase'] + ' / ' + row['component']))
    parts.append('</details><p>Approval scope: these shapes and attachments. Texture fill, native canopy composition and runtime integration are separate decisions.</p>')
    for path in [construction_path, views_dir/'report.json', views_dir/'self-review.json', junction_dir/'report.json', junction_dir/'self-review.json', q.OUT/'profile03-proposed-support-sweep-v2/report.json', q.OUT/'profile03-map-requalification-v2.json', q.OUT/'profile03-fine-source-details-v1.json', q.OUT/'qualification.json']:
        pins.append(q.pin(path))
    (dest/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Hidden archer03 geometry</title><style>body{background:#171b20;color:#e7ebef;font:16px system-ui;max-width:1200px;margin:32px auto;padding:0 20px}h1,h2{color:white}figure{margin:20px 0}figcaption{margin-bottom:8px;color:#cad5df}img{max-width:100%;height:auto;background:#30363e}details{border:1px solid #56616c;padding:16px}summary{cursor:pointer}p{line-height:1.5;max-width:1000px}</style>'+''.join(parts))
    (dest/'handoff.json').write_text(json.dumps(dict(status='GEOMETRY_READY_FOR_GROUPED_USER_REVIEW',
        card_id='croisement02-hidden-archer03-foliage-geometry', title='Hidden archer03 foliage cover — four states',
        scope='Four discrete source foliage poses and eight short inferred attachments. No texture or native runtime integration approval.',
        model=construction['model'], gallery=q.pin(dest/'index.html'), inputs=pins, recipe=q.pin(Path(__file__)),
        limits=['18 fine source leaf islands retained without invented per-pixel supports.', 'Gray unknown backs and supports pending approved texture fill.', 'Native physical source composition remains separate.', 'Canonical assets and qualified state bindings unchanged.']), indent=2)+'\n')
    print(dest/'index.html')


if __name__ == '__main__':
    main()
