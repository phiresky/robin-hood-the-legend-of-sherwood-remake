"""Freeze one paired net03 geometry card for a later grouped review batch."""
import json,sys
from pathlib import Path
from PIL import Image,ImageDraw
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
 base=OUT/'restart3-net03/endpoints-v4';endpoints=[]
 for suffix,label in [('e','Empty'),('i','Occupied')]:
  p=base/suffix;root=json.loads((p/'root-review.json').read_text());model=sha(p/'model.blend');audit=json.loads((p/'reopened-audit.json').read_text());context=json.loads((p/'context-budget-v1/validation.json').read_text())
  if root['status']!='ready-for-user' or root['model_sha256']!=model:raise ValueError('Exact root review missing')
  if audit['model_sha256']!=model or not audit['native_rgba_unchanged'] or audit['bag_wood_overlap_volume']>.001:raise ValueError('Exact guards failed')
  if audit['source_air_blocked'] or context['final_changed_pixels'] or not context['all_imported_geometry_uv_world_exact']:raise ValueError('Air or neighbor context guard failed')
  write_json(p/'review-validation.json',dict(status='PASS',model_sha256=model,source_audit=audit,context_validation=context,source_classification=json.loads((p/'source-classification.json').read_text())))
  panel=Image.new('RGB',(1536,1280),'#303030');draw=ImageDraw.Draw(panel)
  images=[(p/'source-native.png','Native artwork / exact model projection'),(p/'context-budget-v1'/f"native-{context['final_budget']}.png",'Original camera: all four current neighboring trees'),(p/'context-budget-v1/wood-only.png','Inferred support contact: canopy hidden'),(p/'context-budget-v1/contact-1.png','Converged oblique context')]
  for i,(path,title)in enumerate(images):
   im=Image.open(path).convert('RGBA');bg=Image.new('RGBA',im.size,'#303030');bg.alpha_composite(im);im=bg.convert('RGB');im.thumbnail((744,596));x=(i%2)*768;y=(i//2)*640;draw.text((x+12,y+10),title,fill='white');panel.paste(im,(x+(768-im.width)//2,y+32+(596-im.height)//2))
  panel.save(p/'paired-context-review.png')
  endpoints.append(dict(id=label.lower(),asset_id='croisement02-net-piege03-'+suffix+'-endpoint',status='Geometry checks passed; user approval pending',model=str(p/'model.blend'),model_sha256=model,solid=str(p/'solid8.png'),textured=str(p/'actual8.png'),context=str(p/'paired-context-review.png'),validation=str(p/'review-validation.json'),ownership=str(p/'manifest.json'),review=str(p/'root-review.json')))
 item=dict(id='croisement02-net-piege03-endpoints',name='Northeast net trap — empty and occupied endpoint geometry',status='ready-for-user',technical_eligible=True,user_approval=None,model=endpoints[0]['model'],review_scope='paired endpoint geometry only',endpoint_reviews=endpoints,notes=[
  'One geometry decision covers both exact final-phase-0 models: the thin empty net and the fuller occupied net, their wooden counterweights, cords and separate inferred support branch. Initial rigging, animation, captured actors and runtime integration remain separate.',
  'Existing tree37–40 geometry is unchanged. The small inferred support joins current tree39 wood continuously beneath its canopy; the branch is hidden-shape inference, not observed sprite artwork.',
  'Admitted native RGBA is exact; 44 empty and 46 occupied strap-air pixels remain physically open. The source audit classifies 67 empty and 22 occupied pixel-center residuals. Of these, 31 empty pixels form a detached native fragment retained in the original state imagery without an invented physical identity.',
  'Small contour wedges and gray support, back and cord boundaries remain appearance/inference limitations. This is not texture approval or full-motion completion; cord appearance still needs later fill after geometry approval.',
  'Every eight-view sheet starts with the original game camera. All four imported neighbor transforms and meshes match their frozen workers, and 256/512-bounce native context images are pixel-identical.',
 ])
 index=base/'review-candidates.json';write_json(index,dict(map='Croisement02 northeast net endpoints',items=[item],without_packets=[],status_counts={'pending paired geometry review':1}));build(index,base/'gallery')
 page=base/'gallery/index.html';document=page.read_text().replace('approval covers both initial and applied models.','approval covers both empty and occupied final endpoint models.').replace('original endpoint artwork','native artwork and attachment evidence');page.write_text(document)
if __name__=='__main__':main()
