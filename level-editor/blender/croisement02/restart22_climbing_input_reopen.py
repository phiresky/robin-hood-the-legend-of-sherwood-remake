"""Reopen prepared texture models and repeat exact native guards without save/render."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from refinement_review import _tree
from tree_geometry import RAY,SIN
from restart2_prepare_endpoint_inputs import signature
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';INPUT=BASE/'climbing-texture-inputs-v1';DEST=BASE/'climbing-texture-experiments-v1/saved-model-reopen.json'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 assert not DEST.exists();assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(x for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')).split()[1])>=6*1024**2
 results=[]
 for state in ['initial','applied']:
  worker=INPUT/state;d=json.loads((worker/'derivation.json').read_text());model=worker/'model.blend';assert sha(model)==d['prepared_model_sha256'];record=json.loads((Path(d['source_model']).parent/'construction.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objects)==1;assert signature(objects)==d['geometry_uv_material_signature'];obj=objects[0];mesh=obj.data;mesh.calc_loop_triangles();tree,owners,_=_tree(objects);source=np.array(Image.open(record['source']).convert('RGBA'));height,width=source.shape[:2];x0,y0=record['source_top_left'];failures=[];known_count=0
  for y in range(-2,height+2):
   for x in range(-2,width+2):
    point,normal,index,distance=tree.ray_cast(Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000,-RAY);expected=0<=y<height and 0<=x<width and source[y,x,3]>=128
    if expected and point is None:failures.append([x,y,'missing']);continue
    if not expected:
     if point is not None and y0+y>=0:failures.append([x,y,'extra'])
     continue
    known_count+=1;tri=mesh.loop_triangles[index];mat=mesh.materials[tri.material_index]
    if not mat.get('foliage_observed'):failures.append([x,y,'nonobserved first hit']);continue
    shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED');tex=shader.inputs['Base Color'].links[0].from_node;image=tex.image;assert image.packed_file and hashlib.sha256(image.packed_file.data).hexdigest()==record['source_sha256'];uv=mesh.uv_layers[tex.inputs['Vector'].links[0].from_node.uv_map];a,b,c=[obj.matrix_world@mesh.vertices[v].co for v in tri.vertices];ab,ac,ap=b-a,c-a,point-a;aa,bb,cc=ab.dot(ab),ab.dot(ac),ac.dot(ac);den=aa*cc-bb*bb;u=(cc*ap.dot(ab)-bb*ap.dot(ac))/den;v=(aa*ap.dot(ac)-bb*ap.dot(ab))/den;coords=[uv.data[i].uv for i in tri.loops];q=coords[0]*(1-u-v)+coords[1]*u+coords[2]*v;sample=[math.floor(q.x*width),height-1-math.floor(q.y*height)]
    if sample!=[x,y]:failures.append([x,y,'UV mismatch',sample])
  assert not failures and known_count==record['source_opaque_centers'];assert sha(model)==d['prepared_model_sha256'] and sha(d['source_model'])==d['source_model_sha256'];results.append(dict(state=state,source_model_sha256=d['source_model_sha256'],prepared_model_sha256=sha(model),geometry_uv_material_signature=signature(objects),native_pixels=known_count,failures=failures))
 data=dict(status='PASS',states=results,total_native_pixels=sum(r['native_pixels'] for r in results),saved_or_rendered=False,api_attempted=False);encoded=json.dumps(data,indent=2)+'\n';assert len(encoded)<2*1024**2;DEST.write_text(encoded);print(encoded)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
