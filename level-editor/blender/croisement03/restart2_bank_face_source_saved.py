"""Project reviewed native stone interiors onto saved rock faces in a private derivative."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_opening
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as guard
import restart2_bank_full_v1 as base
B=guard.B;INPUT=B/'bank-boulder-fronts-saved-v3';OUT=B/'bank-boulder-source-saved-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 global INPUT,OUT
 if '--input' in sys.argv:INPUT=Path(sys.argv[sys.argv.index('--input')+1]).resolve()
 if '--out' in sys.argv:OUT=Path(sys.argv[sys.argv.index('--out')+1]).resolve()
 assert not OUT.exists();guard.resources();guard.acquire(slots=2)
 try:
  guard.resources();OUT.mkdir();model=INPUT/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
  sourcepath=B.parent/'baseline/covered.png';source=np.array(Image.open(sourcepath).convert('RGBA'));old=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;proposal=np.array(Image.open(B/'bank-face-source-proposal-v1/proposed-native-stone.png'))>0
  # Thin isolated color matches remain unassigned even inside a reviewed stone region.
  broad=binary_opening(proposal,structure=np.ones((3,3),bool));faces=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text())['faces'];expected={};perface=[]
  for f in faces:
   im=Image.new('L',(source.shape[1],source.shape[0]));ImageDraw.Draw(im).polygon([tuple(p)for p in f['native_outline']],fill=255);domain=(np.array(im)>0)&proposal
   perface.append(dict(id=f['id'],owner=f['owner'],proposed=int(domain.sum()),thin_or_boundary_rejected=int((domain&~broad).sum()),accepted=0,ray_rejected=[]))
   for y,x in zip(*np.nonzero(domain&broad)):expected[(int(x),int(y))]=(int(f['owner']),len(perface)-1)
  rows=[];geometry={}
  for obj in scene.objects:
   if obj.type!='MESH':continue
   mesh=obj.data;mesh.calc_loop_triangles();tri=list(mesh.loop_triangles);verts=[obj.matrix_world@v.co for v in mesh.vertices];rows.append((obj,BVHTree.FromPolygons(verts,[list(t.vertices)for t in tri],all_triangles=True),tri));geometry[obj.name]=([list(v)for v in verts],[list(p.vertices)for p in mesh.polygons])
  assignments={};accepted=[];oldhits=[]
  for y,x in zip(*np.nonzero(old|broad)):
   x,y=int(x),int(y);origin=Vector((x+.5,-(y+.5)/base.S,0))+base.RAY*5000;hits=[]
   for obj,bvh,tri in rows:
    p,n,f,d=bvh.ray_cast(origin,-base.RAY)
    if p is not None:hits.append((d,obj.name,tri[f].polygon_index))
   first=min(hits)if hits else None
   if old[y,x]:
    assert first and first[1].startswith('Candidate bank '),(x,y,first);oldhits.append([x,y,first[1],first[2]])
   else:
    owner,idx=expected[(x,y)]
    if not first or first[1]!=f'Candidate bank {owner}':perface[idx]['ray_rejected'].append([x,y,None if first is None else first[1]]);continue
    perface[idx]['accepted']+=1;accepted.append([x,y])
   assignments.setdefault((first[1],first[2]),[]).append((x,y))
  for (name,face),pixels in assignments.items():
   obj=scene.objects[name];xs,ys=zip(*pixels);l,t,r,b=min(xs),min(ys),max(xs)+1,max(ys)+1;rgba=source[t:b,l:r].copy();rgba[:,:,3]=0
   for x,y in pixels:rgba[y-t,x-l,3]=255
   path=OUT/f'{name.replace(" ","-")}-face-{face}.png';Image.fromarray(rgba).save(path);im=bpy.data.images.load(str(path));im.pack();mat=bpy.data.materials.new(f'Private reviewed stone {name}/{face}');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';em=nodes.new('ShaderNodeEmission');gray=nodes.new('ShaderNodeBsdfPrincipled');gray.inputs['Base Color'].default_value=(.25,.25,.25,1);mix=nodes.new('ShaderNodeMixShader');out=nodes.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Color'],em.inputs['Color']);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(gray.outputs[0],mix.inputs[1]);links.new(em.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs['Surface']);obj.data.materials.append(mat);obj.data.polygons[face].material_index=len(obj.data.materials)-1;uv=obj.data.uv_layers.active or obj.data.uv_layers.new(name="Native stone coordinates")
   for li in obj.data.polygons[face].loop_indices:
    v=obj.matrix_world@obj.data.vertices[obj.data.loops[li].vertex_index].co;uv.data[li].uv=((v.x-l)/(r-l),1-(-v.y*base.S-v.z*base.C-t)/(b-t))
  bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'worker.blend'));assert (OUT/'worker.blend').stat().st_size<8*1024**2
  bpy.ops.wm.open_mainfile(filepath=str(OUT/'worker.blend'))
  for name,(verts,faces0)in geometry.items():
   obj=bpy.context.scene.objects[name];assert [list(obj.matrix_world@v.co)for v in obj.data.vertices]==verts;assert [list(p.vertices)for p in obj.data.polygons]==faces0
  assert sha(model)==digest;assert len(oldhits)==3446
  provenance_path=INPUT/'interface-construction.json';provenance=json.loads(provenance_path.read_text());provenance['inherited_geometry_provenance']={'path':str(provenance_path),'sha256':sha(provenance_path),'reason':'Material-only derivative; saved vertex/face arrays independently asserted identical.'};(OUT/'interface-construction.json').write_text(json.dumps(provenance,indent=2)+'\n')
  report=dict(status='PRIVATE_SOURCE_PROJECTION_REQUIRES_SAVED_VISUAL_REVIEW',input_model_sha256=digest,model_sha256=sha(OUT/'worker.blend'),source_sha256=sha(sourcepath),protected3446_unchanged_geometry=True,protected_firsthits=oldhits,proposed=2516,accepted=len(accepted),accepted_pixels=accepted,thin_or_boundary_rejected=int((proposal&~broad).sum()),faces=perface,limits=['Only directly reviewed broad stone interiors assigned experimentally; thin matches and wrong receivers remain unknown.','Opening is a conservative rejection filter, not independent ownership evidence.','No canonical writes or final appearance approval.']);(OUT/'projection.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k]for k in ['status','accepted','thin_or_boundary_rejected']})
 finally:guard.release()
if __name__=='__main__':main()
