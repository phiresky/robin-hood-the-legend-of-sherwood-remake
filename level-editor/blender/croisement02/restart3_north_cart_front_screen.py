"""Represent the opaque native front cabin screen as finite supported geometry."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from restart3_north_cart_canopy import signature
from scenery_geometry import Mesh


def main():
    root=OUT/'restart3-north-cart';base=root/'canopy-fit-v1';metadata=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==metadata['model_sha256']
    dest=root/'front-screen-v2';dest.mkdir(exist_ok=False);acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;bpy.context.view_layer.update()
        old={o.name:signature(o) for o in scene.objects if o.type=='MESH'}
        Image.open(base/'source.png').save(dest/'source.png');Image.open(base/'source-domain.png').save(dest/'source-domain.png')
        angle,length,width,rise,cx,cy=json.loads((root/'roof-fit-v1/fit.json').read_text())['parameters'];eave=metadata['eave_world_z']
        u=Vector((math.cos(angle),math.sin(angle),0));v=Vector((math.sin(angle),-math.cos(angle),0));center=point(1138+cx,114+cy,eave)
        paint,gray=next(o for o in scene.objects if o.type=='MESH').data.materials[:2]
        m=Mesh();m.box(center+u*2.5+Vector((0,0,-20)),u,v,2.0,2*(width-2),20)
        mesh(scene,'Finite front cabin screen',m.vertices,m.faces,paint,gray,metadata['source_box'],metadata['asset_id'])
        for name,digest in old.items():assert signature(bpy.data.objects[name])==digest
        screen=bpy.data.objects['Finite front cabin screen'];native=[(float(p.co.x-1138),float(-p.co.y*SIN-p.co.z*COS-114)) for p in screen.data.vertices]
        finish(scene,gray,dest,metadata['source_box'],dict(status='Private canopy and finite front screen; review pending',asset_id=metadata['asset_id'],source_frame=metadata['source_frame'],source_position=metadata['source_position'],baseline_model_sha256=metadata['baseline_model_sha256'],parent_model_sha256=metadata['model_sha256'],preserved_running_gear_geometry=metadata['preserved_running_gear_geometry'],preserved_parent_geometry=old,roof_fit=metadata['roof_fit'],roof_fit_sha256=metadata['roof_fit_sha256'],eave_world_z=eave,screen_native_bounds=[[min(p[i] for p in native),max(p[i] for p in native)] for i in range(2)],limitations=['Opaque brown front cabin region is represented as a thin screen; detailed weave/wood ornament and hidden depth remain inferred.','Initial static cart only; horse/harness, approach, breakup and terminal model not included.','Dark undercarriage source residuals remain unclassified; no silhouette inflation to cover them.','Unknown gray rear appearance awaits exact geometry approval.']))
        (dest/'derived-recipe').mkdir();write_json(dest/'derived-recipe.json',dict(recipe=record_recipe(dest/'derived-recipe',Path(__file__)),prior_geometry_exact=True))
    finally:release()

if __name__=='__main__':main()
