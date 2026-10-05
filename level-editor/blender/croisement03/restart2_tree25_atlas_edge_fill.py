"""Bounded inferred leaf-edge RGB extension, preserving source and physical alpha."""
import sys,json,hashlib
from pathlib import Path
from array import array
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import _geometry
from bake_reviewed_asset import _materials
from fill_physical_foliage import fill_atlas_edges,triangle_pixels
from render_multiview_asset import render
from refinement_review import _tile

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pixels(image):
    out=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(out)
    return out.reshape(image.size[1],image.size[0],4)
def main():
    e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    src=e/'baked-preserved-v4';out=e/'baked-preserved-v5';assert not out.exists()
    assert sha(src/'worker.blend')=='e97ae23d7ae2bec4837aa459211b3f5e0b9472dc270337055591ce96e41d16a0'
    normalized=e/'material-partition-v1/model.blend';guard=json.loads((e/'material-partition-v1/normalization.json').read_text());assert sha(normalized)==guard['output_model_sha256']
    acquire();bpy.ops.wm.open_mainfile(filepath=str(normalized));native={}
    asset='croisement03-tree-25'
    for obj in bpy.data.objects:
        if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
        for slot,mat in enumerate(obj.data.materials):
            if mat and mat.get('foliage_physical_opacity'):
                nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image];assert len(nodes)==1
                native[(obj.name,slot)]=pixels(nodes[0].image)
    bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));bpy.context.preferences.filepaths.save_version=0
    manifest=json.loads((e/'views-grid8-v4.json').read_text());scene=bpy.data.scenes[manifest['scene_name']]
    geometry={o.name:_geometry(o) for o in scene.objects}
    outside={o.name:_materials(o) for o in scene.objects if o.type=='MESH' and o.get('asset_group')!=asset}
    uv={o.name:{u.name:[list(x.uv) for x in u.data] for u in o.data.uv_layers} for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset}
    protected=[];rows=[]
    for obj in scene.objects:
        if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
        ownership=obj.data.color_attributes['Source ownership'];assert ownership.domain=='CORNER'
        for slot,mat in enumerate(list(obj.data.materials)):
            if not mat or not mat.get('foliage_physical_opacity'):continue
            faces=[f for f in obj.data.polygons if f.material_index==slot]
            flags={ownership.data[i].color[0] for f in faces for i in f.loop_indices}
            assert flags in ({0.},{1.}), (mat.name,flags)
            node=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image][0]
            before=pixels(node.image);base=native[(obj.name,slot)];assert before.shape==base.shape and np.array_equal(before[...,3],base[...,3])
            if flags=={1.}:
                assert np.array_equal(before,base);protected.append((node.image,hashlib.sha256(before.tobytes()).hexdigest()));continue
            generated=np.any(before[...,:3]!=base[...,:3],axis=2)&(base[...,3]>=.5)
            eligible=np.zeros(base.shape[:2],bool)
            link=node.inputs['Vector'].links;assert len(link)==1 and link[0].from_node.type=='UVMAP'
            atlas_uv=obj.data.uv_layers[link[0].from_node.uv_map];obj.data.calc_loop_triangles()
            selected={f.index for f in faces}
            offsets=[(0.,0.)]+[((x+.5)/8-.5,(y+.5)/8-.5) for y in range(8) for x in range(8)]
            for tri in obj.data.loop_triangles:
                if tri.polygon_index not in selected:continue
                for offset in offsets:
                    rr,cc,_=triangle_pixels([atlas_uv.data[i].uv[:] for i in tri.loops],base.shape[1],base.shape[0],offset)
                    eligible[rr,cc]=True
            eligible &= base[...,3]>=.5
            generated &= eligible
            # Each editable texel belongs to an unknown physical surface.
            # The shared helper enforces a 20-percent cap, without fallback bypass.
            chosen=0;after=before.copy();repaired=np.zeros(eligible.shape,bool)
            for radius in (2,1):
                try:candidate,mask=fill_atlas_edges(before,generated,eligible,radius)
                except ValueError as error:
                    if str(error)!='Foliage edge fill exceeds 20% of the physical atlas':raise
                    continue
                chosen=radius;after=candidate;repaired=mask;break
            assert np.array_equal(before[...,3],after[...,3]) and np.array_equal(before[~repaired],after[~repaired])
            assert not np.any(repaired&generated)
            if repaired.any():
                clone=mat.copy();im=node.image.copy();im.pixels.foreach_set(after.ravel());im.pack();clone.node_tree.nodes[node.name].image=im;obj.data.materials[slot]=clone
                actual=pixels(im);assert np.array_equal(actual[...,3],before[...,3]) and np.array_equal(actual[~repaired],before[~repaired])
                assert np.max(np.abs(actual[repaired,:3]-after[repaired,:3]))<=1/255+1e-7
            rows.append(dict(object=obj.name,slot=slot,material=mat.name,radius=chosen,generated_seed_texels=int(generated.sum()),physical_atlas_texels=int(eligible.sum()),extrapolated=int(repaired.sum()),remaining_gray=int((eligible&~generated&~repaired).sum()),physical_alpha_changed=0,protected_rgb_changed=0))
    assert all(hashlib.sha256(pixels(im).tobytes()).hexdigest()==digest for im,digest in protected)
    assert geometry=={o.name:_geometry(o) for o in scene.objects}
    assert outside=={o.name:_materials(o) for o in scene.objects if o.name in outside}
    assert uv=={o.name:{u.name:[list(x.uv) for x in u.data] for u in o.data.uv_layers} for o in scene.objects if o.name in uv}
    out.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'))
    render(e/'views-grid8-v4.json',out/'actual',width=384)
    buffers=[]
    for i in range(8):
        im=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);buf=array('f',[0])*len(im.pixels);im.pixels.foreach_get(buf);buffers.append(buf);bpy.data.images.remove(im)
    _tile(buffers,384,384,out/'actual/textured.png')
    receipt=dict(status='Mechanical PASS; visual review pending',source_model_sha256=sha(src/'worker.blend'),model_sha256=sha(out/'worker.blend'),sheet_sha256=sha(out/'actual/textured.png'),geometry_verified=True,uv_unchanged=True,outside_objects_unchanged=len(outside),protected_rgba_changes=0,protected_known_foliage_atlases=len(protected),physical_foliage=rows,recipe_sha256=sha(__file__),limitation='Unknown-only 1–2 texel color extrapolation from existing generated atlas samples; atlas counts include unused texels and do not measure screen visibility.')
    (out/'validation.json').write_text(json.dumps(receipt,indent=2)+'\n');release();print(json.dumps(receipt))
if __name__=='__main__':main()
