"""Clear a canopy post from an intact wheel while retaining side-board support."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart3_cart_cargo_debris import *
from restart3_north_cart_canopy import signature
from scenery_geometry import Mesh


def main():
    root=OUT/'restart3-north-cart';base=root/'front-screen-v2';old=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==old['model_sha256'];dest=root/'post-clearance-v3';dest.mkdir(exist_ok=False)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;bpy.context.view_layer.update()
        target=scene.objects['Canopy support 1 78.23'];wheel=scene.objects['Wheel rim 76 1'];fixed={o.name:signature(o) for o in scene.objects if o.type=='MESH' and o!=target}
        def intersection(obj):
            copy=obj.copy();copy.data=obj.data.copy();scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy
            modifier=copy.modifiers.new('Exact wheel overlap measure','BOOLEAN');modifier.operation='INTERSECT';modifier.solver='EXACT';modifier.object=wheel;bpy.ops.object.modifier_apply(modifier=modifier.name)
            bm=bmesh.new();bm.from_mesh(copy.data);amount=abs(bm.calc_volume(signed=True));bm.free();bpy.data.objects.remove(copy,do_unlink=True);return amount
        before=intersection(target);name=target.name;paint,gray=target.data.materials[:2]
        angle=json.loads((root/'roof-fit-v1/fit.json').read_text())['parameters'][0];u=Vector((math.cos(angle),math.sin(angle),0));v=Vector((math.sin(angle),-math.cos(angle),0))
        pts=[p.co.copy() for p in target.data.vertices];along=(min(p.dot(u) for p in pts)+max(p.dot(u) for p in pts))/2;across=(min(p.dot(v) for p in pts)+max(p.dot(v) for p in pts))/2;top=max(p.z for p in pts)
        bpy.data.objects.remove(target,do_unlink=True);m=Mesh();m.box(u*along+v*across+Vector((0,0,(46+top)/2)),u,v,3,3,top-46)
        target=mesh(scene,name,m.vertices,m.faces,paint,gray,old['source_box'],old['asset_id']);after=intersection(target);assert after<1e-4,(before,after)
        for n,digest in fixed.items():assert signature(scene.objects[n])==digest
        Image.open(base/'source.png').save(dest/'source.png');Image.open(base/'source-domain.png').save(dest/'source-domain.png')
        metadata={k:v for k,v in old.items() if k not in ['model_sha256','recipe','source_box','components']};metadata.update(status='Private source-fitted canopy/front screen with wheel-clear support post',parent_model_sha256=old['model_sha256'],post_clearance=dict(target=name,wheel=wheel.name,intersection_before=before,intersection_after=after,bottom_z_before=35,bottom_z_after=46,support='Retained side board spans Z35–51; overlap Z46–51 remains'),preserved_except_post=fixed)
        finish(scene,gray,dest,old['source_box'],metadata)
        (dest/'derived-recipe').mkdir();write_json(dest/'derived-recipe.json',dict(recipe=record_recipe(dest/'derived-recipe',Path(__file__))))
        write_json(dest/'wheel-clearance.json',dict(model_sha256=sha(dest/'worker.blend'),**metadata['post_clearance'],all_other_geometry_exact=True))
    finally:release()

if __name__=='__main__':main()
