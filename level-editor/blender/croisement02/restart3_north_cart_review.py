"""Record exact native comparisons and physical component connections for a cart trial."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_north_cart_support import audit


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]);metadata=json.loads((worker/'manifest.json').read_text());dest=worker/'native-review-v1';dest.mkdir(exist_ok=False)
    acquire()
    try:
        box=[1210,104,1374,268];size=164;scale=5;source_box=metadata['source_box']
        for label,path in [('context',Path(metadata['source_frame']['image'])),('scoped',worker/'source.png')]:
            canvas=Image.new('RGBA',(size,size));canvas.alpha_composite(Image.open(path).convert('RGBA'),(source_box[0]-box[0],source_box[1]-box[1]));canvas.resize((size*scale,size*scale),Image.Resampling.NEAREST).save(dest/f'{label}.png')
        for label,path in [('baseline',OUT/'north-cart-initial-candidate-v5/worker.blend'),('candidate',worker/'worker.blend')]:
            bpy.ops.wm.open_mainfile(filepath=str(path));scene=bpy.context.scene;bpy.context.view_layer.update();scene.cycles.samples=8
            target=point((box[0]+box[2])/2,(box[1]+box[3])/2,0);camera=scene.camera;camera.data.sensor_fit='HORIZONTAL';camera.data.ortho_scale=size
            camera.location=target+RAY*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=scene.render.resolution_y=size*scale
            gray=next(o for o in scene.objects if o.type=='MESH').data.materials[1]
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid' else None;scene.render.filepath=str(dest/f'{label}-{mode}.png');bpy.ops.render.render(write_still=True)
        sheet=Image.new('RGBA',(size*scale*3,size*scale*2))
        for i,name in enumerate(['context','baseline-actual','candidate-actual','scoped','baseline-solid','candidate-solid']):sheet.paste(Image.open(dest/f'{name}.png'),((i%3)*size*scale,(i//3)*size*scale))
        sheet.save(dest/'comparison.png')
        objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];trees={}
        for obj in objects:
            vertices=[obj.matrix_world@v.co for v in obj.data.vertices];trees[obj.name]=BVHTree.FromPolygons(vertices,[p.vertices[:] for p in obj.data.polygons])
        edges={n:[] for n in trees}
        for i,a in enumerate(trees):
            for b in list(trees)[i+1:]:
                if trees[a].overlap(trees[b]):edges[a].append(b);edges[b].append(a)
        base=set(metadata['preserved_running_gear_geometry']);connected=set(base)
        while True:
            expanded=connected|set().union(*(set(edges[n]) for n in connected))
            if expanded==connected:break
            connected=expanded
        disconnected=sorted(set(trees)-connected)
        expected=np.asarray(Image.open(dest/'scoped.png'))[:,:,3]>127;actual=np.asarray(Image.open(dest/'candidate-solid.png'))[:,:,3]>127
        write_json(dest/'report.json',dict(model_sha256=sha(worker/'worker.blend'),baseline_sha256=sha(OUT/'north-cart-initial-candidate-v5/worker.blend'),
            native_direction=list(RAY),native_box=box,source_iou=float((expected&actual).sum()/(expected|actual).sum()),
            missing_scoped_render_pixels=int((expected&~actual).sum()),extra_render_pixels=int((actual&~expected).sum()),
            canopy_connected_to_running_gear=not disconnected,disconnected_components=disconnected,physical_surface_graph=edges,
            limits=['Broad historical source polygon includes unresolved dark undercarriage/harness pixels, so alpha coverage is not semantic completeness.',
                    'Surface graph measures intersections/touches, not material stress or motion.']))
        audit(worker,worker/'current-support-v1')
    finally:release()

if __name__=='__main__':main()
