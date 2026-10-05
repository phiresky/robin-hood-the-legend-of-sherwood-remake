"""Bake an approved-scope planar ground fill with exact local pixel protection."""
import json
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
from restore_ground75_source import geometry
from render_multiview_asset import render


def rgb(path):return np.array(Image.open(path).convert('RGBA'))


def main():
    source=OUT/'restart2-ground38/cumulative848-v1'
    prep=OUT/'restart2-ground-completion/preparation-v1'
    experiment=OUT/'restart2-ground-completion/approved-fill-retry-v2'
    generated=experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
    output=experiment/'bake-v1'
    if output.exists():raise FileExistsError(output)
    approval=json.loads((experiment/'approval.json').read_text())
    if approval['status']!='approved' or approval['approved_by']!='user' or approval['model_sha256']!=sha(source/'model.blend'):raise ValueError('Exact ground approval missing')
    if sha(experiment/'input.png')!=approval['input_sha256'] or sha(experiment/'mask.png')!=approval['mask_sha256']:raise ValueError('Approved input changed')
    original=rgb(experiment/'input.png');raw=rgb(generated/'generated-raw.png');filled=rgb(generated/'generated-preserved.png');mask=rgb(experiment/'mask.png')[:,:,3]==0
    if original.shape!=raw.shape or filled.shape!=original.shape:raise ValueError('Output geometry/dimensions changed')
    expected=original.copy();expected[mask,:3]=raw[mask,:3]
    if not np.array_equal(expected,filled):raise ValueError('Local protected composite differs')
    known=np.array(Image.open(prep/'known.png').convert('L'))>0
    state=np.array(Image.open(prep/'state_reservation.png').convert('L'))>0
    deferred=np.array(Image.open(prep/'deferred_state_floor.png').convert('L'))>0
    relief=np.array(Image.open(prep/'separate_relief.png').convert('L'))>0
    if mask.sum()!=685385 or (mask&(known|state|relief)).any():raise ValueError('Editable domain changed')
    for name,domain in [('known',known),('state',state),('relief',relief)]:
        if not np.array_equal(filled[domain],original[domain]):raise ValueError(name+' protected pixels changed')
    review=json.loads((experiment/'visual-review.json').read_text())
    if review['status']!='PASS for guarded bake' or review['generated_sha256']!=sha(generated/'generated-preserved.png'):raise ValueError('Exact generated review missing')
    acquire()
    try:
        output.mkdir()
        bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0
        obj=bpy.data.objects['Croisement02 Terrain'];before=geometry(obj)
        texture=next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE')
        previous=np.empty(len(texture.image.pixels),np.float32);texture.image.pixels.foreach_get(previous)
        previous=np.rint(previous.reshape(1152,1792,4)[::-1]*255).astype('uint8')
        if not np.array_equal(previous,original):raise ValueError('Approved atlas differs from saved source')
        image=bpy.data.images.load(str(generated/'generated-preserved.png'),check_existing=False);image.pack();texture.image=image
        if geometry(obj)!=before:raise ValueError('Ground geometry or UV changed')
        bpy.ops.wm.save_as_mainfile(filepath=str(output/'model.blend'),compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'));obj=bpy.data.objects['Croisement02 Terrain']
        if geometry(obj)!=before:raise ValueError('Reopened geometry/UV changed')
        texture=next(n for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE')
        values=np.empty(len(texture.image.pixels),np.float32);texture.image.pixels.foreach_get(values)
        packed=np.rint(values.reshape(1152,1792,4)[::-1]*255).astype('uint8')
        if not np.array_equal(packed,filled):raise ValueError('Packed atlas differs from guarded composite')
        write_json(output/'validation.json',dict(status='PASS',model_sha256=sha(output/'model.blend'),source_model_sha256=sha(source/'model.blend'),geometry_uv_signature=before,geometry_uv_unchanged=True,packed_rgba_exact=True,known_rgba_pixels_preserved=int(known.sum()),state_union_rgba_pixels_preserved=int(state.sum()),deferred_state_floor_rgba_pixels_preserved=int(deferred.sum()),separate_relief_rgba_pixels_preserved=int(relief.sum()),generated_pixels=int(mask.sum()),protected_pixels_changed=0,provider_mask_sent=False,local_authoritative_mask_sha256=sha(experiment/'mask.png'),approval_sha256=sha(experiment/'approval.json'),generated_sha256=sha(generated/'generated-preserved.png'),user_texture_approval=None,publication=False))
        manifest=json.loads((source/'views.json').read_text());write_json(output/'views.json',manifest)
        render(output/'views.json',output/'actual',width=512)
        images=[Image.open(output/'actual'/f'view-{i}-textured.png').convert('RGB') for i in range(8)]
        w,h=images[0].size;sheet=Image.new('RGB',(4*w,2*h))
        for i,im in enumerate(images):sheet.paste(im,(i%4*w,i//4*h))
        sheet.save(output/'actual/textured.png')
        if sha(source/'model.blend')!=approval['model_sha256']:raise ValueError('Frozen source changed')
        print(output/'model.blend')
    finally:release()


if __name__=='__main__':main()
