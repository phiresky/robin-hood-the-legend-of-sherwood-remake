"""Inspect saved hidden foliage independently of its observed source envelope."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render

def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=args.workspace.resolve()
    acquire()
    try:
        before=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
        destination=w/'inspection/hidden-volume';destination.mkdir(exist_ok=False)
        scene=bpy.data.scenes['Croisement02 Refinement'];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
        rows=[]
        for obj in bpy.data.collections['Croisement02 Working'].all_objects:
            if obj.type!='MESH' or obj.get('asset_group')!=w.name:continue
            bm=bmesh.new();bm.from_mesh(obj.data)
            shell=[f for f in bm.faces if f.material_index<3]
            hidden=[f for f in bm.faces if f.material_index>=3]
            centres=np.array([list(obj.matrix_world@f.calc_center_median()) for f in hidden])
            low,high=centres.min(0),centres.max(0);relative=(centres-(low+high)/2)/np.maximum((high-low)/2,1e-6)
            rows.append(dict(object=obj.name,removed_envelope_faces=len(shell),hidden_faces=len(hidden),hidden_face_centroid_bounds=[low.tolist(),high.tolist()],central_half_box_faces=int((np.abs(relative)<.5).all(1).sum()),note='Face centroids measure saved 3D distribution, not visible alpha density or canopy quality.'))
            bmesh.ops.delete(bm,geom=shell,context='FACES');bm.to_mesh(obj.data);bm.free()
        packet=json.loads((w/'modified/views.json').read_text())
        for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
        manifest=destination/'cameras.json';write_json(manifest,packet);render(manifest,destination,width=384)
        images=[Image.open(destination/f'view-{i}-textured.png').convert('RGB') for i in range(8)];width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
        for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
        sheet.save(destination/'sheet.png')
        if sha(w/'model.blend')!=before:raise RuntimeError('Inspection mutated model')
        write_json(destination/'evidence.json',dict(model_sha256=before,objects=rows,sheet_sha256=sha(destination/'sheet.png'),status='diagnostic; visual review required',model_preserved=True))
    finally:release()
if __name__=='__main__':main()
