"""An unused physical atlas remains immutable during a guarded texture bake."""
import json,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from bake_texture_candidate import snapshot
from render_slots import acquire,release

def main():
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(tree_workspace(41)/'model.blend'))
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        names={o.name for o in scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-41'}
        before=snapshot(scene,names)
        unused={k:v for k,v in before['physical_foliage'].items() if not v['faces']}
        assert len(unused)==2 and all(v['known_rgba'] for v in unused.values())
        checked=[]
        for key in unused:
            name,slot=key.rsplit('/',1);obj=scene.objects[name];slot=int(slot);original=obj.data.materials[slot]
            material=original.copy();texture=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE');texture.image=texture.image.copy();obj.data.materials[slot]=material
            image=texture.image;pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels)
            for channel,label in [(0,'RGB'),(3,'alpha')]:
                changed=pixels.copy();changed[channel]=1 if changed[channel]<.5 else 0;image.pixels.foreach_set(changed)
                after=snapshot(scene,names)
                assert after['physical_foliage'][key]!=before['physical_foliage'][key],f'Unused {label} mutation escaped guard'
                assert after['geometry']==before['geometry']
                image.pixels.foreach_set(pixels)
                assert snapshot(scene,names)==before,'Restored unused atlas differs'
                checked.append(dict(slot=slot,mutation=label,detected=True,affected_faces=0))
            obj.data.materials[slot]=original
        report=dict(status='PASS',unused_slots=len(unused),checks=checked,model_saved=False)
        (OUT/'tree41-unused-foliage-guard-test.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
    finally:release()

if __name__=='__main__':main()
