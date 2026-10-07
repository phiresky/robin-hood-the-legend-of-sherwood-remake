"""Reconstruct observed mound atlas ownership before any texture completion."""
import hashlib, io, json, shutil, sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from restart25_audit_approved_state_materials import BASE,RECEIPT,EXPECTED,sha
from render_slots import acquire,release
from refinement_review import _tree
from restart6_source_gap_audit import RAY,SIN
DEST=BASE/'restart25-approved-state-materialization-v1/mound-ownership-v1'

def main():
 assert sha(RECEIPT)==EXPECTED
 member=next(m for c in json.loads(RECEIPT.read_text())['decisions_by_card'] for m in c['members'] if m['asset_id']=='croisement02-hiding-mounds-all-initial-geometry')
 model=Path(member['model']);assert sha(model)==member['model_sha256']
 memory=int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024
 assert memory>=6*1024**3 and shutil.disk_usage(BASE).free>=10*1024**3
 validation=model.with_name('validation.json');authority=json.loads(validation.read_text());assert authority['model_sha256']==member['model_sha256']
 source_meta=json.loads((BASE/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());source=Path(source_meta['source']);assert sha(source)==source_meta['source_sha256']==authority['source_sha256']
 rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];DEST.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();records=[];packed={};templates={};reuse=[]
 for site in authority['records']:
  objects=[bpy.data.objects[n] for n in site['objects']];tree,owners,_=_tree(objects);assigned={o:np.zeros((h,w),bool) for o in objects};x0,y0=site['source_origin']
  for y,x in np.argwhere(rgba[:,:,3]>0):
   for dx,dy in [(a,b) for a in [.01,.25,.5,.75,.99] for b in [.01,.25,.5,.75,.99]]:
    p,_,tri,_=tree.ray_cast(Vector((float(x0+x+dx),float(-(y0+y+dy)/SIN),0))+RAY*2000,-RAY)
    if p is not None:assigned[owners[tri]][y,x]=True
  assert np.array_equal(np.logical_or.reduce(list(assigned.values())),rgba[:,:,3]>0),site['tag']
  rows=[]
  for oi,ob in enumerate(objects):
   coords=np.array([list(v.co) for v in ob.data.vertices],dtype=np.float64)
   topology=[list(p.vertices) for p in ob.data.polygons]
   uv=np.array([list(d.uv) for d in ob.data.uv_layers.active.data],dtype=np.float64)
   material_indices=[p.material_index for p in ob.data.polygons]
   if oi not in templates:templates[oi]=(coords,topology,uv,material_indices)
   reference,faces,reference_uv,mi=templates[oi]
   assert faces==topology and mi==material_indices and np.array_equal(uv,reference_uv),(site['tag'],ob.name,'template structure')
   translations=coords-reference;delta=np.median(translations,axis=0);residual=float(np.max(np.abs(translations-delta)))
   assert residual<=.0005,(site['tag'],ob.name,'nonrigid template change',residual)
   reuse.append({'site':site['tag'],'object':ob.name,'template_index':oi,'translation':delta.tolist(),'maximum_translation_residual':residual,'topology_material_indices_uv_exact':True})
   expected=rgba.copy();expected[:,:,:3]=137;expected[assigned[ob]]=rgba[assigned[ob]]
   observed=[];unknown=[]
   for mat in ob.data.materials:
    texs=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image];assert len(texs)==1
    tex=texs[0];assert tex.interpolation=='Closest' and tex.extension=='CLIP' and tex.image.packed_file
    pixels=np.array(Image.open(io.BytesIO(bytes(tex.image.packed_file.data))).convert('RGBA'));assert np.array_equal(expected,pixels),(site['tag'],ob.name,mat.name)
    shaders=[n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED'];assert len(shaders)==1;shader=shaders[0]
    assert not shader.inputs['Alpha'].is_linked and shader.inputs['Alpha'].default_value==1, 'Approved closed solid coverage must not gain alpha discard'
    color=shader.inputs['Emission Color']
    if color.is_linked:
     assert color.links[0].from_socket==tex.outputs['Color'];observed.append(mat.name)
    else:
     unknown.append(mat.name)
   assert len(observed)==len(unknown)==1
   key=ob.name.replace(' ','_');packed[key]=assigned[ob]
   rows.append({'object':ob.name,'observed_material':observed[0],'unknown_material':unknown[0],'observed_texels':int(assigned[ob].sum()),'ownership_array':key,'expected_rgba_sha256':hashlib.sha256(expected.tobytes()).hexdigest()})
  records.append({'site':site['tag'],'instances':[a['id'] for a in site['aliases']],'source_origin':site['source_origin'],'objects':rows});print(json.dumps({'site':site['tag'],'objects':len(rows),'known_assignments':sum(r['observed_texels'] for r in rows)}),flush=True)
 arrays=DEST/'observed-texels.npz';np.savez_compressed(arrays,**packed)
 assert sha(model)==member['model_sha256']
 report={'status':'PASS_EXACT_SAVED_NATIVE_ASSIGNMENT','approval':{'path':str(RECEIPT),'sha256':EXPECTED,'member':member},'source':{'path':str(source),'sha256':sha(source)},'validation':{'path':str(validation),'sha256':sha(validation)},'ownership':{'path':str(arrays),'sha256':sha(arrays)},'records':records,'rigid_template_reuse':reuse,'known_rule':'Only observed material AND assigned source texel. Image presence, source alpha, or generic gray-color equality alone are insufficient.','coverage_rule':'Closed solid geometry, Alpha1; preserve native known RGBA exactly, unknown front texels and reverse materials may receive new appearance.','model_unchanged':True,'generation_started':False}
 (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'report':str(DEST/'report.json'),'sha256':sha(DEST/'report.json'),'sites':len(records),'objects':len(packed)}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
