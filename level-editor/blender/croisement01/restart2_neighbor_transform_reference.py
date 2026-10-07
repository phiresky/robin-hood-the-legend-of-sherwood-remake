"""Read evaluated neighbor transforms from independently reopened saved scenes."""
import argparse,json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('--include-tree06',action='store_true');p.add_argument('--tree06-revision',type=int,default=6);a=p.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);R=ROOT/'level-editor/work/croisement01-refinement/restart2';out=R/(f'bank-neighbor-transform-reference-tree06-v{a.tree06_revision}.json' if a.include_tree06 and a.tree06_revision!=6 else ('bank-neighbor-transform-reference-v2.json' if a.include_tree06 else 'bank-neighbor-transform-reference-v1.json'));assert not out.exists();acquire();rows=[]
sources=[(0,'approved-tree00-wood-fill-v1/croisement01-tree-00/baked-v4-support/worker.blend'),(1,'approved-tree01-isolated-wood-fill-v1/croisement01-tree-01/baked-v1-luminance/worker.blend'),(2,'tree02-v8/assets/croisement01-tree-02/model.blend'),(3,'approved-tree03-fill-v1/croisement01-tree-03/baked-v1-luminance/worker.blend')]
if a.include_tree06:sources.append((6,f'tree06-v{a.tree06_revision}/assets/croisement01-tree-06/model.blend'))
for n,rel in sources:
 path=R/rel;bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();targets=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement01-tree-{n:02d}' and 'foliage' not in o.get('source_node','') and o.get('projection_component')!='crown'];assert targets
 rows.append(dict(mask=n,model_sha256=sha(path),objects=[dict(name=o.name,source_node=o.get('source_node'),matrix_world=[list(row) for row in o.matrix_world]) for o in targets]))
out.write_text(json.dumps(dict(status='Independently reopened source transform references',sources=rows),indent=2)+'\n');print(out)
