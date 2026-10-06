"""Prepare repository texture inputs from exact approved gate geometry and source domains."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';AUTH=WORK/'restart2/gate-texture-authority-v1';GEO=WORK/'restart2/gate-geometry-v10';OUT=WORK/'restart2/gate-texture-inputs-v1'
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
auth=json.loads((AUTH/'manifest.json').read_text());approved=json.loads((GEO/'user-geometry-approval.json').read_text());assert sha(Path(approved['receipt']))==approved['receipt_sha256']
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_review import render_review
from refinement_workspace import _geometry
OUT.mkdir(parents=True)
reports=[]
for row in auth['states']:
 state=row['state'];dest=OUT/state;dest.mkdir();model=GEO/state/'model.blend';assert sha(model)==row['approved_model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();before={o.name:_geometry(o,protect_appearance=True) for o in scene.objects}
 source=Image.new('RGBA',(2500,1100));source.alpha_composite(Image.open(AUTH/state/'native-source.png'),tuple(row['source_bbox'][:2]));source.save(dest/'source.png');source_path=(dest/'source.png').resolve()
 Image.open(AUTH/state/'known-gate-domain.png').save(dest/'known-domain.png')
 inventory={'version':1,'index_namespace':'Authored gate first-hit domain index, not native mask ID','masks':[{'index':0,'kind':'authored-artwork-domain','box_top_left':row['source_bbox'][:2],'box_size':row['source_bbox'][2:],'png':'known-domain.png'}]};(dest/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
 mask={'version':1,'mask_inventory':str((dest/'inventory.json').resolve()),'projections':{'gate-native':{'source_sha256':sha(source_path),'state':state,'assignments':[{'reviewed':True,'asset_group':'york-castle-portcullis','mask_indices':[0],'review_evidence':'Approved exact gate geometry; native patch000 only; first-hit boundary exclusions preserved.'}]}}};(dest/'source-masks.json').write_text(json.dumps(mask,indent=2)+'\n')
 collection=bpy.data.collections.new('Gate texture context');scene.collection.children.link(collection)
 for o in scene.objects:
  if o.type=='MESH':collection.objects.link(o)
 present=sorted({o.get('source_node') for o in collection.objects});layer={'source_path':str(source_path),'projection_label':'gate-native','receiver_nodes':['scenery-york-castle-portcullis'],'occluder_nodes':present}
 packet=render_review(dest/'modified',scene_name=scene.name,collection_name=collection.name,asset_id='york-castle-portcullis',source_path=source_path,width=384,height=512,source_mask_manifest=dest/'source-masks.json',projection_layers=[layer],render_object_names=['scenery-york-castle-portcullis'])
 assert before=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in before}
 reports.append({'state':state,'approved_model_sha256':sha(model),'geometry_uv_materials_preserved':True,'known_domain_sha256':sha(dest/'known-domain.png'),'views_sha256':sha(dest/'modified/views.json'),'provider_request':'Not sent; visual input review and supplemental timber reference selection pending'})
(OUT/'input-review.json').write_text(json.dumps({'status':'Inputs prepared; visual review pending','approval_receipt_sha256':approved['receipt_sha256'],'states':reports},indent=2)+'\n');print('GATE INPUTS COMPLETE',flush=True)
