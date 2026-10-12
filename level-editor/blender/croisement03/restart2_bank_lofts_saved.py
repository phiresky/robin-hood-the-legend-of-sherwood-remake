"""Save closed shoulder replacements, preserving observed creases and fixed crest."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,bmesh
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement')]
from render_slots import acquire,release
import restart2_bank_whole_builder as whole
import restart2_bank_full_v1 as base
B=R/'level-editor/work/croisement03-refinement/restart2';CPU=B/'bank-shoulder-lofts-cpu-v2';E=B/'bank-east-loft-cpu-v4/geometry.json';OUT=B/'bank-shoulder-lofts-saved-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def resources():
 assert shutil.disk_usage(R).free>=10*1024**3+32*1024**2
 mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def main():
 proof=json.loads((CPU/'combined-source-guard.json').read_text());assert proof['status']=='PASS_SOURCE_CONSTRAINTS_CPU'
 for p in [CPU/'geometry.json',E]:assert proof['input_hashes'][str(p)]==sha(p)
 assert not OUT.exists();east=json.loads(E.read_text())
 # Adapt the independently executed combined ray result to the legacy builder's
 # input contract. Its post-Boolean check still runs against actual saved faces.
 contract={'blocked':len(proof['observed_crease_failures']),'misses':0,'geometry_sha256':sha(CPU/'geometry.json'),'source_receipt_sha256':sha(CPU/'combined-source-guard.json'),'samples':proof['observed_crease_samples']}
 (CPU/'cpu-crease-firsthits.json').write_text(json.dumps(contract,indent=2)+'\n')
 resources();acquire(slots=2)
 try:
  resources();whole.O=CPU;original=base.main
  def scoped_main():
   base.O=OUT;build=base.mesh_object
   def modified(name,points,scene):
    obj=build(name,points,scene)
    if name!='Candidate bank 52':return obj
    def operand(label,vertices,faces):
     mesh=bpy.data.meshes.new(label);mesh.from_pydata(vertices,[],faces);mesh.update();result=bpy.data.objects.new(label,mesh);scene.collection.objects.link(result);return result
    def apply(cutter,operation):
     bpy.context.view_layer.objects.active=obj;mod=obj.modifiers.new('Bounded eastern shoulder '+operation,'BOOLEAN');mod.operation=operation;mod.solver='EXACT';mod.object=cutter;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(cutter,do_unlink=True)
    x=east['x_cut'];box=[[x,-1200,-10],[1000,-1200,-10],[1000,0,-10],[x,0,-10],[x,-1200,500],[1000,-1200,500],[1000,0,500],[x,0,500]];faces=[[0,3,2,1],[4,5,6,7],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]]
    apply(operand('Temporary eastern body trim',box,faces),'DIFFERENCE');apply(operand('Temporary closed east loft',east['vertices'],east['faces']),'UNION')
    bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);assert bm.calc_volume()>0;bm.to_mesh(obj.data);bm.free();obj.data.materials.clear();return obj
   base.mesh_object=modified;original()
  base.main=scoped_main;whole.run()
  (OUT/'loft-inputs.json').write_text(json.dumps({'cpu_geometry_sha256':sha(CPU/'geometry.json'),'east_geometry_sha256':sha(E),'source_guard_sha256':sha(CPU/'combined-source-guard.json'),'status':'Saved construction only; visual anatomy and contacts remain required'},indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
