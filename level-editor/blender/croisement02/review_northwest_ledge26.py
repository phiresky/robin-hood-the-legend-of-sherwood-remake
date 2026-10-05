"""Classify three ledge-edge centers and verify the saved local correction."""
import hashlib,json
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from correct_bank_foot import surface,digest
from workspace_components import appearance_state
from refinement_workspace import _geometry
from trial_northwest_ledge26 import hit
from tree_geometry import RAY
from render_multiview_asset import render


def main():
    folder=OUT/'restart2-bank321/northwest-ledge26-v2';output=folder/'reopened-review'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    source=OUT/'restart2-bank321/northwest-edge-ramp-v1/worker.blend'
    bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
        bank_tree=surface([o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'])
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
        obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-035')
        name=obj.name;appearance=appearance_state(obj,{})
        before=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);topology=[tuple(p.vertices) for p in obj.data.polygons]
        foreign={o.name:digest(_geometry(o)) for o in bpy.data.objects if o.type=='MESH' and o!=obj}
        foreign_appearance={o.name:digest(appearance_state(o,{})) for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop' and o!=obj}
        bpy.ops.wm.open_mainfile(filepath=str(folder/'worker.blend'));bpy.context.view_layer.update()
        obj=bpy.data.objects[name];after=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);actual=appearance_state(obj,{})
        if [tuple(p.vertices) for p in obj.data.polygons]!=topology:raise ValueError('Topology changed')
        if actual['uv_layers'][:len(appearance['uv_layers'])]!=appearance['uv_layers'] or actual['active_uv']!=appearance['active_uv']:raise ValueError('Existing UVs changed')
        if actual['materials'][:len(appearance['materials'])]!=appearance['materials']:raise ValueError('Existing material or packed image changed')
        if foreign!={o.name:digest(_geometry(o)) for o in bpy.data.objects if o.type=='MESH' and o!=obj}:raise ValueError('Unrelated geometry changed')
        if foreign_appearance!={o.name:digest(appearance_state(o,{})) for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop' and o!=obj}:raise ValueError('Unrelated appearance changed')
        if np.max(abs(after[:,1:]-before[:,1:]))>.0001:raise ValueError('Unrequested vertical/depth movement')
        support=json.loads((source.parent/'underside-support.json').read_text());base_ids=set()
        for row in support['rows']:
            if 'triangle' in row:base_ids.update(row['triangle'])
            if 'vertex' in row:base_ids.add(row['vertex'])
        base_error=float(np.max(abs(after[list(base_ids)]-before[list(base_ids)])))
        if base_error>.0001:raise ValueError('Previously supported base changed')
        native=bpy.data.images['NW ledge exact native source'];mask=bpy.data.images['NW ledge native domain']
        if hashlib.sha256(native.packed_file.data).hexdigest()!=sha(OUT/'animation-references/composite-frame-0.png'):raise ValueError('Native overlay changed source bytes')
        if hashlib.sha256(mask.packed_file.data).hexdigest()!=sha(folder/'native-overlay-domain.png'):raise ValueError('Native overlay mask changed')
        rocks=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop'];tree=surface(rocks)
        def visible(x,y):
            p,_,_,d=hit(tree,x,y);b,_,_,bd=hit(bank_tree,x,y)
            return p is not None and(b is None or d<bd)
        rows=[]
        for x,y in [(108,109),(109,112),(110,115)]:
            p,_,_,d=hit(tree,x+.5,y+.5);b,_,_,bd=hit(bank_tree,x+.5,y+.5)
            count=sum(visible(x+(a+.5)/16,y+(b_+.5)/16) for a in range(16) for b_ in range(16))
            lo,hi=x-.5,x+1.5
            if not visible(lo,y+.5):raise ValueError('Missing inner edge reference')
            for _ in range(20):
                mid=(lo+hi)/2
                if visible(mid,y+.5):lo=mid
                else:hi=mid
            boundary=(lo+hi)/2;gap=x+.5-boundary
            rows.append(dict(pixel=[x,y],center_rock_hit=p is not None,center_visible=visible(x+.5,y+.5),visible_subpixels=count,total_subpixels=256,visible_fraction=count/256,center_to_visible_edge=gap,classification='narrow partially covered edge' if count and 0<=gap<.5 else 'requires further diagnosis'))
        write_json(output/'edge-classification.json',dict(model_sha256=sha(folder/'worker.blend'),bank_sha256=sha(bank),method='16x16 physical source-camera rays per pixel plus binary-search row boundary; no geometry changes',rows=rows))
        write_json(output/'preservation.json',dict(status='PASS',model_sha256=sha(folder/'worker.blend'),source_sha256=sha(source),topology_exact=True,old_uvs_materials_packed_images_exact=True,new_native_source_and_mask_packed_bytes_exact=True,other_geometry_and_rock_appearance_exact=True,world_y_z_unchanged=True,supported_base_vertices=len(base_ids),maximum_supported_base_change=base_error,prior_support_sha256=sha(source.parent/'underside-support.json'),user_approval=None))
        packet=json.loads((OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/views.json').read_text())
        if np.max(abs(np.array(packet['views'][0]['camera_matrix_world'])[:3,2]-np.array(RAY)))>.000001:raise ValueError('First view is not native camera')
        write_json(output/'views.json',packet)
        scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
        render(output/'views.json',output/'actual',width=384)
        sheet=Image.new('RGB',(1536,768))
        for i in range(8):sheet.paste(Image.open(output/'actual'/f'view-{i}-textured.png').convert('RGB'),((i%4)*384,(i//4)*384))
        sheet.save(output/'actual8.png');print(json.dumps(rows))
    finally:release()


if __name__=='__main__':main()
