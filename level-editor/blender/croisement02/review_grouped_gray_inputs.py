"""Assemble explicit diagnostic scope overlays for four pending bark inputs."""
import hashlib,json
from pathlib import Path
from PIL import Image
R=Path.cwd();B=R/'level-editor/work/active/croisement02';OUT=B/'four-gray-wood-inputs-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);cards=[];pins={}
 for n in (18,19,25,39):
  d=B/(f'tree{n}-gray-fill-input-oct12-v'+('2' if n==39 else '1'));scope=json.loads((d/'scope.json').read_text());base=Image.open(d/'context/textured.png').convert('RGBA');mask=Image.new('L',base.size);
  for i in range(8):mask.paste(Image.open(d/f'packet/views/view-{i}-solid.png').getchannel('A'),((i%4)*384,(i//4)*384))
  overlay=Image.new('RGBA',base.size,(255,45,180,0));overlay.putalpha(mask.point(lambda x:190 if x else 0));base=Image.alpha_composite(base,overlay);target=OUT/f'tree{n}-scope-overlay.png';base.save(target)
  resources=[d/'scope.json',d/'worker.blend',d/'packet/views.json',d/'packet/solid.png',d/'packet/textured.png',d/'context/textured.png',target]
  resources.extend(sorted((d/'packet/views').glob('*.png')))
  resources.extend(Path(r['file']) for r in scope['permitted_material_references'])
  for p in resources:pins[str(p)]=sha(p)
  cards.append({'asset_id':f'croisement02-tree-{n}','scope':'Texture-fill input only. Existing untextured constant-gray wood primitives; no geometry or existing source/color/crown changes.','input':str(d/'packet/textured.png'),'solid':str(d/'packet/solid.png'),'context':str(d/'context/textured.png'),'overlay':str(target),'overlay_warning':'Magenta projects isolated editable fallback surfaces, including surfaces occluded by retained wood. Diagnostic scope only, never a texture input.','input_model_sha256':scope['input_model_sha256'],'first_view':'Original art camera at top left','supplementary_references':scope['permitted_material_references']})
 doc={'status':'SELF_REVIEW_PENDING','cards':cards,'resource_pins':pins,'synthesis_started':False,'canonical_writes':False};(OUT/'packet.json').write_text(json.dumps(doc,indent=2)+'\n')
 html=['<!doctype html><meta charset="utf-8"><title>Four bark fallback input scopes</title><style>body{background:#181818;color:#eee;font:18px sans-serif;max-width:1500px;margin:30px auto}img{width:100%}section{margin:50px 0}p{max-width:1000px}</style><h1>Four bark texture-fill input scopes</h1><p>Approve inputs only. Geometry and existing textures remain unchanged. Magenta marks isolated unsupported wood surfaces, including hidden surfaces; it is a diagnostic overlay. Original camera is always top-left. Synthesis will use matching full wood context plus the two permitted Leicester material references.</p>']
 for c in cards:
  html.append('<section><h2>'+c['asset_id']+'</h2><p>'+c['scope']+'</p>')
  for label,key in [('Existing wood context','context'),('Editable scope overlay — includes hidden surfaces','overlay'),('Isolated editable input','input'),('Isolated solid lighting','solid')]:html.append('<h3>'+label+'</h3><img src="/\u0040fs'+c[key]+'">')
  html.append('</section>')
 for r in cards[0]['supplementary_references']:html.append('<h2>Supplementary material reference: '+r['asset_id']+'</h2><img src="/@fs'+r['file']+'">')
 (OUT/'index.html').write_text(''.join(html));print(OUT)
if __name__=='__main__':main()
