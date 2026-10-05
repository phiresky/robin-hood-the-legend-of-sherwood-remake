"""Read-only initial fence receiver audit; terminal artwork never supplies initial RGB."""
import json,sys
from pathlib import Path
from collections import Counter
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import _tree
from tree_geometry import SIN,RAY

def main():
    dest=OUT/'restart3-initial-fence/first-hit-v1';dest.mkdir(parents=True,exist_ok=False)
    stage=OUT/'restart2-textures/batch-v3-coherent-scene-v1';model=stage/'scene.blend'
    assembly=json.loads((stage/'assembly.json').read_text());assert sha(model)==assembly['model_sha256']
    sourcepath=OUT/'animation-references/composite-frame-0.png'
    source=np.array(Image.open(sourcepath).convert('RGBA'))
    basepath=OUT/'restart3-fence-receiver/terminal-v3/base-atlas.png'
    base=np.array(Image.open(basepath).convert('RGBA'))
    knownpath=OUT/'restart2-ground-completion/preparation-v1/known.png'
    known=np.array(Image.open(knownpath).convert('L'))>0
    rect=np.zeros_like(known);rect[811:963,1018:1170]=True
    assert np.array_equal(base[rect&known],source[rect&known])
    fencepath=OUT/'feedback-wattle-domains/visible-weave.png';fence=np.array(Image.open(fencepath).convert('L'))>0
    inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks'];native={}
    for row in inventory:
        if row['index'] not in [44,98,128]:continue
        a=np.array(Image.open(row['png']).convert('L'))>0;x,y=row['box_top_left'];h,w=a.shape
        full=np.zeros_like(known);full[y:y+h,x:x+w]=a;native[row['index']]=full
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.window.scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.view_layer.update()
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not o.hide_render]
    tree,owners,_=_tree(objects);materials=[];dg=bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        ev=obj.evaluated_get(dg);m=ev.to_mesh();m.calc_loop_triangles();materials.extend(m.materials[t.material_index].name if t.material_index<len(m.materials) and m.materials[t.material_index] else '<none>' for t in m.loop_triangles);ev.to_mesh_clear()
    assert len(materials)==len(owners)
    counts=Counter();rows=[];colors=np.array(source[:,:,:3]);legend={};palette=[(240,80,70),(70,200,200),(200,160,40),(130,100,220),(90,210,80),(220,100,160)]
    yy,xx=np.nonzero(rect&~known)
    for y,x in zip(yy,xx):
        hit,normal,index,distance=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
        obj=owners[index] if hit is not None else None
        asset=obj.get('asset_group') or obj.get('source_node') or obj.name if obj else '<no hit>'
        material=materials[index] if obj else '<none>';role='fence' if fence[y,x] else 'native-foreground'
        counts[(role,asset,material)]+=1
        if asset not in legend:legend[asset]=palette[len(legend)%len(palette)]
        colors[y,x]=legend[asset]
        rows.append(dict(pixel=[int(x),int(y)],source_role=role,native_masks=[n for n,a in native.items() if a[y,x]],asset=asset,object=obj.name if obj else None,material=material,hit=list(hit) if hit is not None else None))
    crop=(1018,811,1170,963);sheet=Image.new('RGB',(912,970),(35,35,35));draw=ImageDraw.Draw(sheet)
    for i,(title,array) in enumerate([('Original source',source),('Approved initial ground atlas',base),('Physical first hit in excluded domain',colors),('Frozen coherent scene',np.array(Image.open(OUT/'restart2-textures/batch-v3-coherent-scene-review-v1/native.png').convert('RGBA'))) ]):
        x=(i%2)*456;y=(i//2)*485;draw.text((x+5,y+5),title,fill='white');sheet.paste(Image.fromarray(array).convert('RGB').crop(crop).resize((456,456),Image.Resampling.NEAREST),(x,y+25))
    sheet.save(dest/'source-atlas-firsthit-scene.png')
    report=dict(status='Read-only classification; no ownership transfer or model edits',scene_sha256=assembly['model_sha256'],scene=str(model),source_sha256=sha(sourcepath),base_atlas_sha256=sha(basepath),known_sha256=sha(knownpath),fence_domain_sha256=sha(fencepath),rectangle=[1018,811,152,152],known_exact_pixels=int((rect&known).sum()),excluded_pixels=len(rows),fence_excluded_pixels=int((rect&~known&fence).sum()),counts=[dict(source_role=r,asset=a,material=m,pixels=n)for(r,a,m),n in counts.most_common()],legend=legend,samples=rows,limitations=['Pixel-center first hits can differ from multisample edge coverage.','Excluded source foreground is not authority to paint native fence or foliage onto floor.','Terminal artwork is applied-only.'])
    write_json(dest/'audit.json',report);print(json.dumps({k:v for k,v in report.items() if k!='samples'},indent=2),flush=True)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
