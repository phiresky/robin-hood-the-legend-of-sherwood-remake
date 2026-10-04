"""Verify saved observed fern images and source ownership without changing the worker."""
import argparse
import hashlib
import json
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=args.workspace.resolve();digest=sha(w/'model.blend')
    acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
    row=json.loads((w/'inspection/construction.json').read_text());expected=row['known_rgba_sha256'];objects=[o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name];checks=[]
    for obj in objects:
        ownership=obj.data.color_attributes['Source ownership']
        for mat in obj.data.materials:
            if not mat.get('foliage_observed'):continue
            images={n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image}
            if not images:raise ValueError('Observed material has no source image')
            for image in images:
                if image.packed_file is None:raise ValueError('Observed source is not packed')
                actual=hashlib.sha256(image.packed_file.data).hexdigest()
                if actual!=expected:raise ValueError('Packed observed source differs from exact native image')
                checks.append(dict(object=obj.name,material=mat.name,image=image.name,packed_source_sha256=actual))
        for face in obj.data.polygons:
            mat=obj.data.materials[face.material_index];owned=bool(mat.get('foliage_observed'))
            if any(abs(ownership.data[i].color[0]-float(owned))>1e-6 for i in face.loop_indices):raise ValueError('Observed flag and corner ownership disagree')
    if not checks:raise ValueError('No observed sources audited')
    if sha(w/'model.blend')!=digest:raise ValueError('Model changed during saved audit')
    (w/'inspection/saved-source-audit.json').write_text(json.dumps(dict(status='PASS',model_sha256=digest,checks=checks,semantics='Packed native source RGBA and per-corner observed ownership verified. Hidden palette materials remain inferred; this does not approve geometry.'),indent=2)+'\n');release();print(w/'inspection/saved-source-audit.json')
if __name__=='__main__':main()
