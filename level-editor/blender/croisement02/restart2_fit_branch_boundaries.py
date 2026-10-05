"""Private bounded silhouette dilation for explicitly reviewed thin wood boundaries."""
import argparse,json,sys,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY


def screens(points):return np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS))

def surfaces(objects,points):
    return [BVHTree.FromPolygons([Vector(v) for v in p],[list(f.vertices) for f in o.data.polygons]) for o,p in zip(objects,points)]

def hits(trees,xy):
    result=[]
    for x,y in xy:
        origin=Vector((float(x),-float(y)/SIN,0))+RAY*5000
        result.append(any(t.ray_cast(origin,-RAY)[0] is not None for t in trees))
    return np.array(result,bool)


def main():
    parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[43,45,46]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=args.index
    worker=OUT/f'restart2-wood/tree{index}-branch-sdf-v1';digest=sha(worker/'model.blend');out=OUT/f'restart2-wood/tree{index}-branch-fitted-v1';out.mkdir(exist_ok=False)
    path=OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-wood35.png' if index==35 else OUT/f'understory-candidates/mixed75-91-boundary-review/91-proposed-wood{index}.png'
    mask=np.asarray(Image.open(path).convert('L'))>0;ys,xs=np.nonzero(mask);targets=np.column_stack((xs+.5,ys+.5));padding=32
    box=(max(0,int(xs.min())-padding),max(0,int(ys.min())-padding),min(1792,int(xs.max())+padding+1),min(1152,int(ys.max())+padding+1))
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
    allobjects=list(bpy.data.collections['Croisement02 Working'].all_objects);objects=[o for o in allobjects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{index:02d}' and o.get('projection_component')!='crown']
    protected={o.name:_geometry(o,protect_appearance=True) for o in allobjects if o.type=='MESH' and o not in objects}
    original=[np.array([o.matrix_world@v.co for v in o.data.vertices],float) for o in objects];points=[p.copy() for p in original];trees=surfaces(objects,points)
    yy,xx=np.mgrid[box[1]:box[3],box[0]:box[2]];grid=np.column_stack((xx.ravel()+.5,yy.ravel()+.5));baseline=hits(trees,grid);protected_targets=grid[baseline];initial=hits(trees,targets);steps=[]
    for iteration in range(32):
        present=hits(trees,targets);lost=protected_targets[~hits(trees,protected_targets)]
        missing=np.concatenate((targets[~present],lost),axis=0)
        if not len(missing):break
        projected=[screens(p) for p in points];all_projected=np.concatenate(projected);distance=np.sum((missing[:,None,:]-all_projected[None,:,:])**2,axis=2)
        nearest=np.argmin(distance,axis=1);selected=int(np.argmax(distance[np.arange(len(missing)),nearest]));target=missing[selected];center=all_projected[nearest[selected]];delta=target-center;length=float(np.linalg.norm(delta))
        if length>6:raise ValueError('Boundary farther than six source pixels from mesh')
        if length<1e-5:raise ValueError('Unresolved exact projected vertex boundary')
        direction=delta/length;shift=direction*(length+.4);radius=max(4.,4*(length+.4));moved=0
        for i,(p,screen) in enumerate(zip(points,projected)):
            offset=screen-center;distance=np.linalg.norm(offset,axis=1);weight=np.maximum(0.,1-distance/radius)**2
            # Keep the opposite side anchored: this expands the nearest silhouette rather than translating a thin branch.
            inward=offset@direction;gate=np.clip(1+inward/max(1.,length+.4),0,1);weight*=gate
            worldshift=np.array([shift[0],-SIN*shift[1],-COS*shift[1]])
            p+=weight[:,None]*worldshift
            floor=np.minimum(original[i][:,2],.1);lift=np.maximum(0.,floor-p[:,2]);p+=lift[:,None]*np.asarray(RAY)/RAY.z
            moved+=int(np.count_nonzero(weight))
        displacement=max(float(np.max(np.linalg.norm(p-q,axis=1))) for p,q in zip(points,original))
        if displacement>8:raise ValueError('Bounded displacement exceeds eight world units')
        steps.append(dict(iteration=iteration,target=list(target),source_distance=length,moved_vertices=moved,max_world_displacement=displacement))
        trees=surfaces(objects,points)
    final=hits(trees,targets);lost=protected_targets[~hits(trees,protected_targets)];reports=[]
    if not final.all() or len(lost):
        write_json(out/'failure.json',dict(status='Private solver HOLD',initial_covered=int(initial.sum()),final_covered=int(final.sum()),target_pixels=len(targets),baseline_lost=lost.tolist(),steps=steps));return
    for obj,p,old in zip(objects,points,original):
        inverse=obj.matrix_world.inverted();changed=np.linalg.norm(p-old,axis=1)>1e-7
        for vertex,world in zip(obj.data.vertices,p):vertex.co=inverse@Vector(world)
        obj.data.update();bm=bmesh.new();bm.from_mesh(obj.data);report=dict(object=obj.name,moved_vertices=int(changed.sum()),total_vertices=len(p),max_displacement=float(np.max(np.linalg.norm(p-old,axis=1))),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free()
        if report['nonmanifold_edges'] or report['degenerate_faces']:raise ValueError(report)
        reports.append(report)
    if protected!={o.name:_geometry(o,protect_appearance=True) for o in allobjects if o.type=='MESH' and o not in objects}:raise ValueError('Protected asset changed')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
    if sha(worker/'model.blend')!=digest:raise ValueError('Input changed')
    write_json(out/'evidence.json',dict(status='Private geometry candidate; projection stale, needs solid review and new native bake',model_sha256=sha(out/'model.blend'),previous_worker=str(tree_workspace(index)),previous_model_sha256=sha(tree_workspace(index)/'model.blend'),geometry_input=str(worker),geometry_input_sha256=digest,boundary_mask=str(path),boundary_mask_sha256=sha(path),classification='Inferred contextual boundary role, not independently proven material',target_pixels=len(targets),initial_covered=int(initial.sum()),final_covered=int(final.sum()),baseline_samples=len(protected_targets),baseline_lost=0,guard_box=box,parts=reports,steps=steps,protected_appearance=protected,limitations=['Closed original topology retained; local source-plane displacement bounded.','Existing textures are not current after geometry edit; no approval or canonical selection inherited.']))
    (out/'recipe.py').write_text(Path(__file__).read_text())

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
