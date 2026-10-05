"""Close the unsupported native front valance/screen gap with finite cabin geometry."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from restart3_north_cart_canopy import signature
from scenery_geometry import Mesh


def main():
    root=OUT/'restart3-north-cart';base=root/'post-clearance-v3';old=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==old['model_sha256'];dest=root/'front-closure-v4';dest.mkdir(exist_ok=False)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();target=scene.objects['Finite front cabin screen'];paint,gray=target.data.materials[:2]
        fixed={o.name:signature(o) for o in scene.objects if o.type=='MESH' and o!=target};angle,length,width,rise,cx,cy=json.loads((root/'roof-fit-v1/fit.json').read_text())['parameters'];eave=old['eave_world_z'];u=Vector((math.cos(angle),math.sin(angle),0));v=Vector((math.sin(angle),-math.cos(angle),0));center=point(1138+cx,114+cy,eave)
        bpy.data.objects.remove(target,do_unlink=True);m=Mesh();m.box(center+u*2.5+Vector((0,0,-17)),u,v,2,2*(width-2),26)
        mesh(scene,'Finite front cabin screen',m.vertices,m.faces,paint,gray,old['source_box'],old['asset_id'])
        for name,digest in fixed.items():assert signature(scene.objects[name])==digest
        Image.open(base/'source.png').save(dest/'source.png');Image.open(base/'source-domain.png').save(dest/'source-domain.png')
        metadata={k:v for k,v in old.items() if k not in ['model_sha256','recipe','source_box','components']};metadata.update(status='Private source-conforming front closure; root review pending',parent_model_sha256=old['model_sha256'],front_gap_correction=dict(old_screen_top_z=eave-10,new_screen_top_z=eave-4,valance_bottom_z=eave-4,change='Raise only upper edge of finite front screen to meet native opaque valance band',material_interpretation='Native ornamented screen/cloth closure; hidden material inferred'),preserved_except_front_screen=fixed)
        finish(scene,gray,dest,old['source_box'],metadata)
        write_json(dest/'closure-preservation.json',dict(model_sha256=sha(dest/'worker.blend'),parent_model_sha256=old['model_sha256'],all_other_geometry_exact=True,wheel_clearance_receipt=str(base/'wheel-clearance.json'),wheel_clearance_sha256=sha(base/'wheel-clearance.json'),correction=metadata['front_gap_correction']))
        (dest/'derived-recipe').mkdir();write_json(dest/'derived-recipe.json',dict(recipe=record_recipe(dest/'derived-recipe',Path(__file__))))
    finally:release()

if __name__=='__main__':main()
