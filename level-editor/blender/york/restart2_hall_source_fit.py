"""Inspect furniture source fit in isolation; not a complete revealed state."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',required=True)
parser.add_argument('--include-candle',action='store_true')
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
worker=OUT/'restart2'/args.version
destination=worker/'source-fit'
if destination.exists():raise FileExistsError(destination)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from source_projection_bake import bake
bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
destination.mkdir()
source=OUT/'baseline/revealed.png'
mask_ids={824:631,825:634,826:633,827:640,828:632,829:636}
label='hall-furniture-isolated'
assignments=[(f'building-{n}',index) for n,index in mask_ids.items()]
if args.include_candle:assignments.append(('scenery-york-great-hall-candle-stand',635))
nodes=[node for node,index in assignments]
manifest={'version':1,'mask_inventory':str(OUT/'baseline/masks/manifest.json'),
          'projections':{label:{'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
            'state':'Revealed source artwork; six furniture receivers inspected independently. Full room and neighboring fireplace state still pending.',
            'assignments':[{'reviewed':True,'source_node':node,'mask_indices':[index],
                            'review_evidence':'Individually inspected native silhouettes in restart2/hall-furniture-native-masks.png.'}
                           for node,index in assignments]}}}
authority=destination/'source-masks.json'
authority.write_text(json.dumps(manifest,indent=2)+'\n')
bake('york',source,destination/'projection.json',receiver_nodes=nodes,occluder_nodes=nodes,
     projection_label=label,texels_per_unit=2,preserve_authored=False,source_mask_manifest=authority)
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'native-identities.blend'),compress=True)
# A separate diagnostic display selector allows furniture-only complete-object
# inspection. It is not a replacement for the canonical great-hall grouping.
for obj in bpy.data.collections['york Working'].all_objects:
    if obj.type=='MESH' and not obj.hide_render and obj.get('source_node') in nodes:
        obj['asset_group']='private-york-hall-furniture'
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True)
config=json.loads((OUT/'geometry-pass-01/assets/york-castle-great-hall/workspace.json').read_text())
config.update(asset_id='private-york-hall-furniture',source_path=str(source),part_ids=nodes)
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(destination/'scope.json').write_text(json.dumps({'status':'HOLD','scope':'Isolated furniture source fit only.',
    'geometry_approval':False,'complete_revealed_state':False,
    'excluded_context':'All non-furniture occluders are omitted only for this isolated diagnostic. Native joint inspection will still show the covered room.',
    'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest()},indent=2)+'\n')
