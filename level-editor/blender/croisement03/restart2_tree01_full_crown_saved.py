"""Private combined crown review with separately owned exact native frame0 cells."""
import sys,json,hashlib,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent));import restart2_tree01_canopy_saved as review
from tree_geometry import replace_mesh,one_sided
B=R/'level-editor/work/croisement03-refinement/restart2';CELLS=B/'tree01-native-cells-cpu-v2/geometry.json';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def build(scene):
 d=json.loads(CELLS.read_text());path=Path(d['source_frame']);assert hashlib.sha256(path.read_bytes()).hexdigest()==d['source_frame_sha256'];assert hashlib.sha256((review.I/'geometry.json').read_bytes()).hexdigest()==d['inferred_crown_sha256'];obj=bpy.data.objects.new('Tree01 observed native frame0 provisional cells',bpy.data.meshes.new('Tree01 observed native'));scene.collection.objects.link(obj);mat=bpy.data.materials.new('Tree01 exact native observed RGBA');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();links=mat.node_tree.links;uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Foliage UV';tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path));tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP';links.new(uv.outputs['UV'],tex.inputs['Vector']);emission=nodes.new('ShaderNodeEmission');links.new(tex.outputs['Color'],emission.inputs['Color']);transparent=nodes.new('ShaderNodeBsdfTransparent');mix=nodes.new('ShaderNodeMixShader');links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(transparent.outputs[0],mix.inputs[1]);links.new(emission.outputs[0],mix.inputs[2]);output=nodes.new('ShaderNodeOutputMaterial');links.new(mix.outputs[0],output.inputs[0]);one_sided(mat);replace_mesh(obj,d['vertices'],d['faces'],d['uvs'],[mat],known=[True]*len(d['faces']));obj['source_scope']='Provisional native Arbre08 spatial fragment; not static inferred ownership';obj['source_sha256']=d['source_frame_sha256']
def verify(scene,output):
 d=json.loads(CELLS.read_text());obj=scene.objects['Tree01 observed native frame0 provisional cells'];obj.data.calc_loop_triangles();points=[obj.matrix_world@v.co for v in obj.data.vertices];assert float(np.max(np.abs(np.array(points)-d['vertices'])))<1e-4
 for polygon in obj.data.polygons:
  for loop in polygon.loop_indices:assert np.max(np.abs(np.array(obj.data.uv_layers.active.data[loop].uv)-d['uvs'][obj.data.loops[loop].vertex_index]))<1e-7
 tex=next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');assert hashlib.sha256(bytes(tex.image.packed_file.data)).hexdigest()==d['source_frame_sha256'];tree=BVHTree.FromPolygons(points,[list(t.vertices)for t in obj.data.loop_triangles],all_triangles=True);wood=[]
 for o in scene.objects:
  if not o.name.startswith('Provisional Tree01 wood'):continue
  o.data.calc_loop_triangles();wood.append(BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(t.vertices)for t in o.data.loop_triangles],all_triangles=True))
 source=np.array(Image.open(d['source_frame']).convert('RGBA'));bad=[]
 for y,x in np.argwhere(source[:,:,3]>0):
  origin=Vector((float(x)+.5,-(float(y)+.5)/S,0))+RAY*5000;hit,normal,face,distance=tree.ray_cast(origin,-RAY)
  if hit is None:bad.append([int(x),int(y),'native miss']);continue
  projected=[hit.x,-hit.y*S-hit.z*C]
  if max(abs(projected[0]-x-.5),abs(projected[1]-y-.5))>1e-3:bad.append([int(x),int(y),'projection'])
  for w in wood:
   wh,wn,wf,wd=w.ray_cast(origin,-RAY)
   if wh is not None and wd<distance-1e-3:bad.append([int(x),int(y),'wood occludes native leaf'])
 receipt={'status':'PASS_EXACT_NATIVE_FRAME0_FIRSTHITS'if not bad else'HOLD_NATIVE_FIRSTHITS','pixels':int((source[:,:,3]>0).sum()),'failures':bad,'source_sha256':d['source_frame_sha256'],'cells_sha256':hashlib.sha256(CELLS.read_bytes()).hexdigest(),'limits':['Frame0 selected fragment only; shared dynamic timing and other map source owners unchanged and not proven here.','Packed original PNG and UVs exact; no source pixels assigned to inferred leaf backs.']};(output/'native-source-guard.json').write_text(json.dumps(receipt,indent=2)+'\n');assert not bad,bad[:10]
def main():
 review.I=B/'tree01-canopy-volume-cpu-v3';review.O=B/'tree01-combined-crown-saved-v1';review.EXTRA_BUILD=build;review.EXTRA_VERIFY=verify;review.main()
if __name__=='__main__':main()
