"""Present measured private scene evidence without an approval or publication action."""
import argparse
import html
import json
import sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);args=parser.parse_args();stage=args.stage.resolve()
    assembly=json.loads((stage/'assembly.json').read_text());model=sha(stage/'scene.blend')
    if assembly['model_sha256']!=model:raise ValueError('Stage changed')
    render=json.loads((stage/'render-evidence.json').read_text());contacts=json.loads((stage/'contacts/evidence.json').read_text());audit=json.loads((stage/'first-hit/audit.json').read_text())
    for record in [render,contacts,audit]:
        if record['model_sha256']!=model:raise ValueError('Evidence model mismatch')
    for view in render['views']:
        if sha(stage/view['image'])!=view['sha256']:raise ValueError('Full-frame render changed')
    for image,expected in contacts['images'].items():
        if sha(stage/'contacts'/image)!=expected:raise ValueError('Contact render changed')
    blocks=[('Eight full-frame views','sheet.png'),('Original source camera; all visible scene geometry','contacts/source-camera.png'),('Labeled source contact regions','contacts/source-contact-sheet.png'),('Fern119/tree23 contact; only tree crowns hidden','contacts/contact-0-tree-crowns-hidden.png'),('North shrubs65/66 contact; only tree crowns hidden','contacts/contact-1-tree-crowns-hidden.png'),('Red: observed ground covered by foreground geometry; source pixels retained','first-hit/observed-ground-covered.png'),('Fern119/tree23 shared23pixels: actual first-hit owner','first-hit/fern119-tree23-owners.png'),('Physical first-hit owners','first-hit/first-hit-owners.png')]
    figures=''.join('<figure><figcaption>'+html.escape(title)+'</figcaption><a href="'+path+'"><img loading="lazy" src="'+path+'"></a></figure>' for title,path in blocks)
    state=[r['id'] for r in assembly['assets'] if 'objects' not in r]
    text=f'''<!doctype html><meta charset="utf-8"><title>Croisement02 private93 scene review</title><style>body{{background:#222;color:#eee;font:16px system-ui;margin:28px;max-width:1600px}}a{{color:#8dd7ff}}img{{max-width:100%;height:auto}}figure{{margin:28px 0;padding:14px;background:#303030}}figcaption{{margin-bottom:10px}}code{{overflow-wrap:anywhere}}</style><h1>Croisement02 private93 scene review</h1><p>Frozen93-group ownership snapshot:89 visible workers,4 hidden state metadata groups. This is an integration diagnostic, not a completed map or an approval request.</p><p>11 approved textures match their selected geometry exactly. Revised southwest logs retain their current stored materials; the incompatible older texture-model replacement was omitted.</p><p>All23 fern119/tree23 shared pixels hit fern119 first. Tree23 cached source artwork was not rewritten. Physical source audit has zero unassigned map pixels;9,094 observed-ground pixels are covered by foreground models and remain preserved in the ground atlas.</p><p>Known remaining issues: legacy canopy sheets in side/rear views; unfilled hidden ground and relief; unfinished state assemblies. Ground versus bank observed ownership remains disjoint. Later canopy selector changes are outside this frozen snapshot.</p><p>Scene SHA256: <code>{model}</code></p><p><a href="assembly.json">Assembly</a> · <a href="first-hit/audit.json">Physical first-hit audit</a> · <a href="first-hit/worker-domain-audit.json">89 worker source-domain measurements</a> · <a href="first-hit/metadata-ownership-audit.json">Global metadata proposal comparison</a> · <a href="first-hit/owner-legend.png">Owner legend</a> · <a href="compression.json">Compression preservation proof</a></p><p>Pending state groups: {html.escape(', '.join(state))}</p>'''+figures
    (stage/'index.html').write_text(text)
    write_json(stage/'review-packet.json',dict(status='private scene validation evidence; visual completion not approved',model_sha256=model,assembly_sha256=sha(stage/'assembly.json'),render_evidence_sha256=sha(stage/'render-evidence.json'),contact_evidence_sha256=sha(stage/'contacts/evidence.json'),first_hit_audit_sha256=sha(stage/'first-hit/audit.json'),worker_domain_audit_sha256=sha(stage/'first-hit/worker-domain-audit.json'),images={path:sha(stage/path) for _,path in blocks},hidden_state_groups=state,approval='none',publication='not performed'))
    print(stage/'index.html')


if __name__=='__main__':main()
