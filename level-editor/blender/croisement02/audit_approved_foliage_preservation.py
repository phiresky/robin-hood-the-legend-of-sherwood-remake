"""Compare native observed foliage directly across approved base/bake pairs."""
import hashlib,json,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from render_slots import acquire,release
from review_evidence import sha

def digest(value):return hashlib.sha256(value).hexdigest()
def snapshot(path,names):
    bpy.ops.wm.open_mainfile(filepath=str(path));result={};physical={}
    for name in names:
        obj=bpy.data.objects[name];mesh=obj.data;ownership=mesh.color_attributes.get('Source ownership')
        for slot,material in enumerate(mesh.materials):
            if not material or not material.get('foliage_physical_opacity'):continue
            faces=[f for f in mesh.polygons if f.material_index==slot]
            if not faces:continue
            assert ownership is not None
            protected=sum(all(ownership.data[i].color[0]==1 for i in f.loop_indices) for f in faces)
            assert not protected or material.get('foliage_observed'), 'Unclassified protected native foliage'
            if material.get('foliage_observed'):assert protected==len(faces)
            textures=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image];assert len(textures)==1
            texture=textures[0];image=texture.image;pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels)
            uv=mesh.uv_layers[texture.inputs['Vector'].links[0].from_node.uv_map]
            geometry=[dict(vertices=[list(mesh.vertices[i].co) for i in f.vertices],uv=[list(uv.data[i].uv) for i in f.loop_indices]) for f in faces]
            physical[f'{name}:{slot}']=dict(alpha_sha256=digest(pixels.reshape(-1,4)[:,3].tobytes()),faces=len(faces),protected_faces=protected,observed=bool(material.get('foliage_observed')))
            if not material.get('foliage_observed'):continue
            result[f'{name}:{slot}']=dict(rgb_alpha_sha256=digest(pixels.tobytes()),image_size=list(image.size),faces=len(faces),geometry_uv_sha256=digest(json.dumps(geometry,sort_keys=True).encode()))
    return dict(observed=result,physical_alpha=physical)

def main():
    stage=OUT/'integration-canopy-cleanup-1';selected=json.loads((stage/'selected-texture-approvals.json').read_text());assets={a['id']:a for a in json.loads((stage/'assembly.json').read_text())['assets']};reports=[];acquire()
    try:
        for asset,record in selected.items():
            base=Path(assets[asset]['worker'])/'model.blend';bake=Path(record['model'])
            assert sha(base)==assets[asset]['geometry_model_sha256']
            assert sha(bake)==record['decision']['evidence_sha256']['model']
            before=snapshot(base,record['receiver_names']);after=snapshot(bake,record['receiver_names'])
            reports.append(dict(asset_id=asset,status='PASS' if before==after else 'FAIL',native_foliage_materials=len(before['observed']),base=str(base),base_sha256=sha(base),bake=str(bake),bake_sha256=sha(bake),before=before,after=after))
            print(asset,reports[-1]['status'],len(before['observed']),flush=True)
        report=dict(status='PASS' if all(r['status']=='PASS' for r in reports) else 'FAIL',selected_decisions_sha256=sha(stage/'selected-texture-approvals.json'),scope='Direct packed native atlas float pixels, observed face geometry and UVs; independent of generic source masks. Read-only inspection; existing approvals unchanged.',assets=reports)
        (OUT/'approved-native-foliage-preservation.json').write_text(json.dumps(report,indent=2)+'\n')
        assert report['status']=='PASS'
    finally:release()

if __name__=='__main__':main()
