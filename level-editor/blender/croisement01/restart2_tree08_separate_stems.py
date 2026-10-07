"""Test a bounded inferred-interior clearance between two distinct native stems."""
import json,subprocess
from pathlib import Path
import numpy as np
from restart2_tree08_remaining_forks import inputs
from restart2_tree08_cap_plan import section_loops,serialize,read_paths,triangulate
R=Path(__file__).resolve().parents[2]/'work/croisement01-refinement/restart2'
OUT=R/'tree08-v12-distinct-stems-clearance-v1'
KERNEL=R/'tree08-v12-local-fork-cpu-v1/cap-intersect'
def operate(subject,clips,intersection=False):
 if not clips:return [] if intersection else subject
 result=subprocess.run([str(KERNEL)]+(['intersection'] if intersection else []),input=serialize(subject)+serialize(clips),capture_output=True,text=True,check=True,timeout=10)
 return read_paths(iter(result.stdout.split()))
def expanded(mesh):
 vertices,faces=mesh;n=np.zeros_like(vertices)
 t=vertices[faces];fn=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]);fn/=np.linalg.norm(fn,axis=1)[:,None]
 for i in range(3):np.add.at(n,faces[:,i],fn)
 n/=np.linalg.norm(n,axis=1)[:,None]
 return vertices+n*1e-4,faces

def main():
 OUT.mkdir(exist_ok=False);meshes,_,_,origin=inputs(R);cutters={i:expanded(meshes[i]) for i in [88,105]};subjects={103:meshes[103],**cutters};all_triangles=[]
 for index,(vertices,faces) in subjects.items():
  output=[]
  for fi,face in enumerate(faces):
   triangle=vertices[face];center=triangle.mean(0);normal=np.cross(triangle[1]-triangle[0],triangle[2]-triangle[0]);normal/=np.linalg.norm(normal);u=triangle[0]-center;u/=np.linalg.norm(u);basis=np.array([u,np.cross(normal,u)])
   patches=[(triangle-center)@basis.T]
   try:
    if index!=103:patches=operate(patches,section_loops(*meshes[103],center,normal,basis),True)
    if patches:
     clips=[]
     for j,mesh in cutters.items():
      if j!=index:clips.extend(section_loops(*mesh,center,normal,basis))
     if clips:patches=operate(patches,clips)
    for patch in triangulate(patches) if patches else []:
     world=center+patch@basis
     if np.dot(np.cross(world[1]-world[0],world[2]-world[0]),normal)<0:world=world[::-1]
     if index!=103:world=world[::-1]
     output.append(world+origin)
   except Exception as error:
    (OUT/f'failure-{index}-{fi}.json').write_text(json.dumps(dict(section=index,face=fi,error=repr(error),subject=((triangle-center)@basis.T).tolist(),patches=[p.tolist() for p in patches]),indent=2)+'\n');raise
  np.savez_compressed(OUT/f'part-{index}.npz',triangles=np.asarray(output).reshape(-1,3,3));all_triangles.extend(output);print('COMPLETE',index,len(output),flush=True)
 np.savez_compressed(OUT/'triangles.npz',triangles=np.asarray(all_triangles))
 (OUT/'scope.json').write_text(json.dumps(dict(status='CPU candidate only; source visibility and topology unproven',receiver=103,retained_stems=[88,105],maximum_temporary_cutter_displacement=1e-4,method='Subtract a tiny outward-offset cutter from overlapping inferred interior only; retain original source-facing surfaces elsewhere. Cut surfaces use reversed original cutter walls. Native separated stems are not fused.'),indent=2)+'\n')
if __name__=='__main__':main()
