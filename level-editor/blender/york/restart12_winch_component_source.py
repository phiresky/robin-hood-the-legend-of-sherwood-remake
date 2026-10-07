"""Freeze stable winch components with conservative native source ownership."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement';WORK=BASE/'restart2';OUT=WORK/'winch-components-source-v2'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageFilter,ImageChops
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from source_projection_bake import bake
from refinement_workspace import _geometry
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
donor=WORK/'winch-motion-physical-v2';bpy.ops.wm.open_mainfile(filepath=str(donor/'model.blend'));scene=bpy.context.scene
motion=json.loads((donor/'motion.json').read_text());assert sha(donor/'model.blend')==motion['model_sha256'];scene.frame_set(motion['rows'][44]['tick']);bpy.context.view_layer.update()
receivers=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004' and not o.name.startswith('Suspended chain link')];assert len(receivers)==24
chain=[o for o in scene.objects if o.type=='MESH' and o.name.startswith('Suspended chain link')];assert len(chain)==68
before={o.name:_geometry(o) for o in scene.objects};outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in receivers}
source=BASE/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');f=frames[44];original=Image.open(source/f['image']).convert('RGBA');x,y,w,h=f['bbox'];assert original.size==(w,h)
OUT.mkdir();canvas=Image.new('RGBA',(2500,1000));canvas.paste(original,(x,y));canvas.save(OUT/'source-frame44.png')
vs=[];fs=[]
for o in chain:
 off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices);fs.extend(tuple(off+j for j in p.vertices) for p in o.data.polygons)
tree=BVHTree.FromPolygons(vs,fs);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));excluded=Image.new('L',(w,h))
for py in range(h):
 for px in range(w):
  origin=Vector((x+px+.5,-(y+py+.5)/s,0))+back*10000
  if tree.ray_cast(origin,-back)[0] is not None:excluded.putpixel((px,py),255)
# A two-source-pixel guard excludes uncertain chain borders from wood evidence.
excluded=excluded.filter(ImageFilter.MaxFilter(5));allowed=ImageChops.subtract(original.getchannel('A'),excluded);allowed.save(OUT/'receiver-mask.png');excluded.save(OUT/'chain-exclusion.png')
(OUT/'inventory.json').write_text(json.dumps({'masks':[{'index':0,'box_top_left':[x,y],'box_size':[w,h],'png':'receiver-mask.png'}]},indent=2)+'\n')
label='winch-stable-components-final';authority={'version':1,'mask_inventory':'inventory.json','projections':{label:{'source_sha256':sha(OUT/'source-frame44.png'),'state':'Patch004 transition44, source-alpha minus conservative chain projection guard; static and moving stable components only','assignments':[{'reviewed':True,'source_node':'scenery-york-castle-winch','mask_indices':[0],'review_evidence':'Native45-frame source study inspected; chain first-hit silhouette dilated two native pixels conservatively excludes pending iron from wood projection.'}]}}}
(OUT/'source-masks.json').write_text(json.dumps(authority,indent=2)+'\n')
collection=bpy.data.collections.new('Winch component source context');scene.collection.children.link(collection)
for o in scene.objects:
 if o.type=='MESH':collection.objects.link(o)
bake('york',OUT/'source-frame44.png',OUT/'projection.json',receiver_nodes=['scenery-york-castle-winch'],receiver_object_names=[o.name for o in receivers],projection_label=label,source_mask_manifest=OUT/'source-masks.json',texels_per_unit=2,preserve_authored=False,collection_name=collection.name)
assert before=={o.name:_geometry(o) for o in scene.objects};assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside}
poses=[]
for row in motion['rows']:
 scene.frame_set(row['tick']);bpy.context.view_layer.update();poses.append({'source_frame':row['source_frame'],'tick':row['tick'],'components':{o.name:_geometry(o,protect_appearance=True) for o in receivers}})
scene.frame_set(motion['rows'][44]['tick']);bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'component-freeze.json').write_text(json.dumps({'status':'Private source-projected component candidate, self/root review pending; NOT mechanism approval','donor_sha256':sha(donor/'model.blend'),'model_sha256':sha(OUT/'model.blend'),'source_frame_sha256':sha(source/f['image']),'motion_sha256':sha(donor/'motion.json'),'scope':[o.name for o in receivers],'excluded':['All68 suspended chain links and inferred return mechanism','Room context','Runtime state/audio integration'],'all_geometry_unchanged':True,'outside_appearance_unchanged':len(outside),'components_at_final':{o.name:{'geometry':_geometry(o),'appearance_geometry':_geometry(o,protect_appearance=True),'parent':o.parent.name if o.parent else None} for o in receivers},'poses':poses,'limitations':['Projection from final native pose only; hidden backs remain neutral gray.','Chain guard intentionally leaves uncertain source pixels unknown.','Chain phase and return mechanism remain private HOLD.','Two intermediate source-hole boundaries at frames30/31 require explicit scoped review; complete mechanism not claimed.','Early travelling centre22 preceding frames inferred; later descent and rebound source-measured.']},indent=2)+'\n')
print('STABLE COMPONENT SOURCE MODEL SAVED',OUT)
