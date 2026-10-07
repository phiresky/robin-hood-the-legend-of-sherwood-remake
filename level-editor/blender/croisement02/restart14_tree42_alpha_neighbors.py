"""Measure physical source-ray ownership on pinned published canopy and neighbors."""
from pathlib import Path
import sys,json,hashlib,math,struct
from collections import Counter
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from refinement_review import _tree
from tree_geometry import SIN,COS,RAY
OUT=ROOT/'level-editor/work/croisement02-refinement';BASE=OUT/'restart14-canopy-animation';DEST=BASE/'tree42-alpha-neighbors-v5';LIB=ROOT/'level-editor/library';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
IDS=['croisement02-tree-42','croisement02-tree-31','croisement02-tree-32','croisement02-tree-30','croisement02-south-field-haystack','croisement02-south-field-wattle-fence','croisement02-southwest-path-wattle-fence','croisement02-shrub-69','croisement02-shrub-76']
def main():
 DEST.mkdir(exist_ok=False);map_path=LIB/'scenes/croisement02.rhlos-map.json';document=json.loads(map_path.read_text());idx={e['id']:e for e in json.loads((LIB/'3d-assets/index.json').read_text())['assets']};source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];motion=json.loads((BASE/'tree42-correspondence-v1/report.json').read_text());bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;groups={};pins=[];materials=[]
 for aid in IDS:
  dp=LIB/'3d-assets'/idx[aid]['descriptor'];d=json.loads(dp.read_text());p=next(p for p in document['placements']if aid in p['assets']);t=p['transform'];assert t['rot_deg']==0;mp=dp.parent/d['model'];raw=mp.read_bytes();g=json.loads(raw[20:20+struct.unpack_from('<I',raw,12)[0]]);old=set(bpy.data.objects);oldm=set(bpy.data.materials);bpy.ops.import_scene.gltf(filepath=str(mp));added=[o for o in bpy.data.objects if o not in old];root=[o for o in added if o.parent not in added];assert len(root)==1;root[0].location+=Vector((t['dx'],-t['dy']/SIN,t['dz']));groups[aid]=[o for o in added if o.type=='MESH'];bpy.context.view_layer.update()
  for o in groups[aid]:o['audit_asset']=aid
  for gm in g.get('materials',[]):
   assert gm.get('alphaMode','OPAQUE')in('OPAQUE','MASK'),('Unsupported blend opacity',gm.get('name'))
   assert gm.get('alphaCutoff',.5)==.5
   assert gm.get('pbrMetallicRoughness',{}).get('baseColorFactor',[1,1,1,1])[3]==1
  for material in [m for m in bpy.data.materials if m not in oldm]:
   nodes=list(material.node_tree.nodes);alpha_nodes=[n for n in nodes if n.type=='TEX_IMAGE'and n.outputs['Alpha'].links];images={n.image for n in alpha_nodes};assert len(images)<=1,('Multiple alpha images',material.name)
   textured=bool(images)
   if textured:
    texture=alpha_nodes[0];image=texture.image;interpolation=texture.interpolation;extension=texture.extension;uv_links=texture.inputs['Vector'].links;uv_name=None
    if uv_links:
     assert len(uv_links)==1 and uv_links[0].from_node.type=='UVMAP';uv_name=uv_links[0].from_node.uv_map
    used=[o for o in groups[aid]if material in list(o.data.materials)]
    for color in [n for n in nodes if n.type=='VERTEX_COLOR'and n.outputs['Alpha'].links]:
     for o in used:
      layer=o.data.color_attributes[color.layer_name]if color.layer_name else o.data.color_attributes.active_color;assert layer is not None;values=np.empty(len(layer.data)*4,np.float32);layer.data.foreach_get('color',values);assert np.all(values.reshape(-1,4)[:,3]==1),('Nonunit vertex alpha requires barycentric opacity',material.name)
    material.node_tree.nodes.clear();shader=material.node_tree.nodes.new('ShaderNodeBsdfPrincipled');tex=material.node_tree.nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation=interpolation;tex.extension=extension;material.node_tree.links.new(tex.outputs['Alpha'],shader.inputs['Alpha'])
    if uv_name is not None:
     uvnode=material.node_tree.nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv_name;material.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector'])
    material['foliage_physical_opacity']=True;material['opacity_semantics']='physical-coverage'
   else:material['foliage_physical_opacity']=False
   materials.append({'asset':aid,'name':material.name,'alpha_textured':textured,'backface_culling':material.use_backface_culling,'audit_conversion':'MASK image alpha threshold0.5, unit base and vertex alpha proved; private ray-only shader, not saved/rendered'})
  pins.append({'id':aid,'descriptor':str(dp),'descriptor_sha256':sha(dp),'model':str(mp),'model_sha256':sha(mp),'placement':p,'resources':[{'path':str(LIB/r['path']),'sha256':sha(LIB/r['path'])}for r in d.get('resources',[])]})
 crown=next(o for o in groups[IDS[0]]if o.get('projection_component')=='crown');initial=np.array([v.co[:]for v in crown.data.vertices]);world=np.array([crown.matrix_world@v.co for v in crown.data.vertices]);native=np.c_[world[:,0],-world[:,1]*SIN-world[:,2]*COS];assert 590<native[:,0].min()<640 and 920<native[:,0].max()<980,(native.min(0),native.max(0));inverse=crown.matrix_world.inverted().to_3x3();neighbor_trees={aid:_tree(objects)[:2]for aid,objects in groups.items()if aid!=IDS[0]};results=[]
 for phase,f in enumerate(source['frames']):
  samples=[s for s in motion['rows'][phase]['samples']if s['accepted']];points=np.array([s['pixel']for s in samples]);vectors=np.array([s['delta']for s in samples]);delta=np.zeros((len(world),2))
  for start in range(0,len(world),2000):
   dist=np.sum((native[start:start+2000,None,:]-points[None,:,:])**2,axis=2);ii=np.argsort(dist,axis=1)[:,:4];ds=np.take_along_axis(dist,ii,axis=1);weight=np.exp(-ds/(2*12**2))*(ds<28**2);delta[start:start+2000]=np.sum(vectors[ii]*weight[:,:,None],axis=1)/np.maximum(weight.sum(1)[:,None],1e-9)
  disp=np.c_[delta[:,0],-delta[:,1]*SIN,-delta[:,1]*COS];local=np.array([inverse@Vector(v)for v in disp]);crown.data.vertices.foreach_set('co',(initial+local).reshape(-1));crown.data.update();bpy.context.view_layer.update();tree,owners,_=_tree(groups[IDS[0]]);a=np.array(Image.open(f['path']));x,y,w,h=f['bbox'];counts=Counter();missing=[];blocked=[];depths=[]
  for py,px in zip(*np.nonzero(a[:,:,3])):
   sx=int(px+x);sy=int(py+y);origin=Vector((sx+.5,-(sy+.5)/SIN,0))+RAY*5000;hit=tree.ray_cast(origin,-RAY);distance=hit[3]if hit[0]is not None else float('inf');first=(distance,IDS[0]if hit[0]is not None else '<none>')
   for aid,(nt,no)in neighbor_trees.items():
    nh=nt.ray_cast(origin,-RAY)
    if nh[0]is not None and nh[3]<first[0]:first=(nh[3],aid)
   counts[first[1]]+=1
   if hit[0]is None:missing.append([sx,sy])
   if first[1]not in(IDS[0],'<none>'):blocked.append({'pixel':[sx,sy],'asset':first[1],'tree42_hit':hit[0]is not None,'depth_lead':distance-first[0]if math.isfinite(distance)else None})
  results.append({'phase':phase,'source_pixels':int(np.count_nonzero(a[:,:,3])),'first_hit_counts':dict(counts),'tree42_alpha_misses':missing,'foreign_first_hits':blocked});print('PHASE',phase,dict(counts),'own misses',len(missing),flush=True)
 crown.data.vertices.foreach_set('co',initial.reshape(-1));crown.data.update();assert all(sha(Path(p['model']))==p['model_sha256']for p in pins)
 (DEST/'report.json').write_text(json.dumps({'status':'MEASURED_NOT_COMPLETION','map_sha256':sha(map_path),'pins':pins,'material_opacity':materials,'phases':results,'limitations':['Independent per-asset rays avoid combined-BVH near-coplanar ordering artifacts.','Ground/bank omitted because this bounded test concerns foreground canopy ownership; misses are not reassigned ground.','Physical alpha threshold0.5, explicit image alpha and culling; color resemblance does not establish source ownership.','No published asset or model was written.']},indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
