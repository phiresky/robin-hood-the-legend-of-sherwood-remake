"""Read-only native bank samples with the approved tree06 assembly."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_initial_fence_contact import link
from refinement_review import _tree
from tree_geometry import SIN,RAY
from review_bank_candidate import camera
D=OUT/'restart4-bank131-current-audit-v2'
def main():
 assert not D.exists();D.mkdir()
 bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 ground=OUT/'restart4-fence14-ground-candidate-v1/model.blend'
 auth=json.loads((OUT/'restart2-textures/batch-v9-texture-approval-v1/approved-addition-authority.json').read_text())
 wood=json.loads((OUT/'restart2-textures/tree06-retained-wood-fill-v1/approved-preparation/experiment/original-atlas-bake-v2/assembly-authority.json').read_text())
 pins={bank:'69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641',ground:'4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'}
 for a in [auth['base'],auth['approved_texture_model'],wood['wood_derivative']]:pins[Path(a['path'])]=a['sha256']
 assert all(sha(p)==h for p,h in pins.items())
 bpy.ops.wm.open_mainfile(filepath=str(bank));scene=bpy.data.scenes.new('Bank131 current context');bpy.context.window.scene=scene
 objects=[o for o in bpy.data.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank']
 for o in objects:link(scene,o);o.hide_render=False
 def load(path,names=None,exclude=()):
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.objects=names or list(src.objects)
  chosen=[o for o in dst.objects if o and o.type=='MESH'and o.name not in exclude and (names or o.name.startswith('Northwest Tree 06 /'))]
  for o in chosen:link(scene,o);o.hide_render=False
  objects.extend(chosen)
 load(ground,['Croisement02 Terrain'])
 load(Path(auth['base']['path']),exclude=wood['wood_derivative']['only_receivers'])
 load(Path(wood['wood_derivative']['path']),wood['wood_derivative']['only_receivers'])
 load(Path(auth['approved_texture_model']['path']),[auth['addition_object']])
 bpy.context.view_layer.update();tree,owners,_=_tree(objects)
 source=np.array(Image.open(OUT/'source-states/covered.png').convert('RGBA'))
 mask=np.array(Image.open(OUT/'restart3-northern-source-audit/region-8-bank_domain_residual.png').convert('L'))>0
 ys,xs=np.where(mask);assert len(xs)==131
 # Reuse exact prior bank-ray material observations; bank bytes are identical.
 prior=json.loads((OUT/'restart2-bank321/neutral-material-comparison-v1/report.json').read_text());assert prior['model_sha256']==pins[bank]
 samples={tuple(r['pixel']):r for r in prior['rows']};rows=[]
 for x,y in zip(xs.tolist(),ys.tolist()):
  p,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000,-RAY);obj=owners[i]if p is not None else None
  old=samples.get((x,y));color=np.rint(np.array(old['sample_rgba'])*255).astype(int).tolist()if old else None
  rows.append(dict(pixel=[x,y],current_first_hit=obj.name if obj else None,bank_first_hit=bool(obj and obj.get('asset_group')=='croisement02-north-woodland-bank'),world=list(p)if p else None,source_rgba=source[y,x].tolist(),bank_nearest_rgba=color,bank_native_rgb_exact=bool(color and color[:3]==source[y,x,:3].tolist()),bank_material=old['material']if old else None))
 counts=dict(total=131,bank_first_hit=sum(r['bank_first_hit']for r in rows),native_exact_bank=sum(r['bank_first_hit']and r['bank_native_rgb_exact']for r in rows),bank_mismatch=sum(r['bank_first_hit']and not r['bank_native_rgb_exact']for r in rows))
 write_json(D/'report.json',dict(status='Read-only current scoped first-hit and unchanged-bank nearest texture comparison',pins={str(p):h for p,h in pins.items()},prior_material_report_sha256=sha(OUT/'restart2-bank321/neutral-material-comparison-v1/report.json'),counts=counts,rows=rows,limitations=['Bank texture samples use identical69ec prior ray audit, nearest sampling; source/current render comparison still required.','Only approved bank, ground and tree06 assembly; no claim for every whole-scene neighbor.'],models_changed=False))
 marked=source.copy()
 for r in rows:x,y=r['pixel'];marked[y,x,:3]=[255,50,150]if r['bank_first_hit']else[20,220,230]
 sheet=Image.new('RGB',(1080,432),'#292929');draw=ImageDraw.Draw(sheet);box=(610,505,710,580)
 for i,(a,label)in enumerate([(source,'Native source'),(marked,'Pink bank / cyan foreground')]):sheet.paste(Image.fromarray(a).crop(box).resize((540,405),Image.Resampling.NEAREST),(540*i,27));draw.text((540*i+5,7),label,fill='white')
 sheet.save(D/'source-marked.png')
 center=Vector((660,-536/SIN,0))+RAY*(44/RAY.z)
 # Moving along native ray leaves the source projection unchanged.
 for label,direction in [('native',RAY),('oblique',Vector((.55,-.65,.5)).normalized())]:
  camera(scene,center,direction,768,576,140);scene.cycles.transparent_max_bounces=512;scene.render.filepath=str(D/(label+'.png'));bpy.ops.render.render(write_still=True,scene=scene.name)
 assert all(sha(p)==h for p,h in pins.items());print(counts,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
