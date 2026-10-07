"""Read-only material audit of exact approved state workers; no model writes."""
import hashlib,json,sys
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from render_slots import acquire,release
ROOT=HERE.parents[2]
BASE=ROOT/'level-editor/work/croisement02-refinement'
DEST=BASE/'restart25-approved-state-materialization-v1'
RECEIPT=BASE/'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json'
EXPECTED='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
IDS={'croisement02-hole-initial','croisement02-hole-applied','croisement02-leaf-scatter-source-surfaces','croisement02-hiding-mounds-all-initial-geometry'}
def sha(p):
 h=hashlib.sha256()
 with Path(p).open('rb') as f:
  for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
def plain(v):
 if isinstance(v,(str,int,float,bool)) or v is None:return v
 try:return list(v)
 except:return str(v)
def main():
 assert sha(RECEIPT)==EXPECTED
 approval=json.loads(RECEIPT.read_text());members=[m for c in approval['decisions_by_card'] for m in c['members'] if m['asset_id'] in IDS]
 assert len(members)==4
 DEST.mkdir(exist_ok=True)
 results=[]
 for member in members:
  model=Path(member['model']);assert sha(model)==member['model_sha256']
  bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
  objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];uses={}
  for ob in objects:
   for p in ob.data.polygons:
    mat=ob.material_slots[p.material_index].material if p.material_index<len(ob.material_slots) else None
    key=mat.name if mat else '<none>';row=uses.setdefault(key,{'faces':0,'objects':set(),'area':0.0});row['faces']+=1;row['objects'].add(ob.name);row['area']+=p.area
  materials=[]
  for name,usage in uses.items():
   mat=bpy.data.materials.get(name);images=[];colors=[]
   if mat and mat.use_nodes:
    for n in mat.node_tree.nodes:
     if n.type=='TEX_IMAGE' and n.image:
      im=n.image;images.append({'name':im.name,'size':list(im.size),'packed_sha256':hashlib.sha256(im.packed_file.data).hexdigest() if im.packed_file else None,'colorspace':im.colorspace_settings.name,'extension':n.extension,'interpolation':n.interpolation})
     if n.type in {'BSDF_PRINCIPLED','EMISSION','RGB'}:
      socket=n.inputs.get('Base Color') or n.inputs.get('Color')
      if socket and not socket.is_linked:colors.append({'node':n.type,'color':list(socket.default_value)})
   materials.append({'name':name,'faces':usage['faces'],'objects':len(usage['objects']),'area':usage['area'],
                     'images':images,'unlinked_colors':colors,'properties':{k:plain(mat[k]) for k in mat.keys()} if mat else {},
                     'image_free':not images})
  report={'asset_id':member['asset_id'],'scope':member['scope'],'source':str(model),'sha256':member['model_sha256'],
          'meshes':len(objects),'polygons':sum(len(o.data.polygons)for o in objects),'used_materials':len(materials),
          'image_free_faces':sum(r['faces']for r in materials if r['image_free']),
          'materials':materials,'scene_properties':{k:plain(bpy.context.scene[k])for k in bpy.context.scene.keys()},
          'object_selectors':[{'name':o.name,'materials':[m.name if m else None for m in o.data.materials]}for o in objects]}
  results.append(report);assert sha(model)==member['model_sha256']
 out=DEST/'material-audit.json';assert not out.exists();out.write_text(json.dumps({'status':'READ_ONLY_EXACT_APPROVED_WORKERS','approval_sha256':EXPECTED,'records':results},indent=2)+'\n')
 print(json.dumps({'report':str(out),'sha256':sha(out),'records':[{k:r[k]for k in ['asset_id','meshes','polygons','used_materials','image_free_faces']}for r in results]}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
