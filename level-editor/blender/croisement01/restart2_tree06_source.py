"""Prepare conservative native wood ownership excluding the crossing leaf cluster."""
import argparse,hashlib,json
from pathlib import Path
from PIL import Image,ImageDraw,ImageChops
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--separate-root-fragments',action='store_true');p.add_argument('--preserve-native-basal-wood',action='store_true');a=p.parse_args();dest=a.output;dest.mkdir(exist_ok=False);source=R/'tree06-source-v1'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def pixels(image):return sum(value>0 for value in image.get_flattened_data())
base=Image.open(source/'proposed-wood-domain.png').convert('L');excluded=Image.new('L',base.size)
polygons={'crossing_leaf_cluster':[(19,140),(44,143),(66,146),(76,153),(77,202),(63,224),(47,239),(28,224),(20,195)],'root_fern_mixed':[(34,346),(58,349),(70,353),(82,362),(89,384),(81,398),(65,385),(56,369),(35,365)]}
assert not (a.separate_root_fragments and a.preserve_native_basal_wood)
if a.preserve_native_basal_wood:polygons.pop('root_fern_mixed')
if a.separate_root_fragments:polygons['lower_fern_mixed_fragments']=[(0,359),(89,359),(89,394),(0,394)]
for points in polygons.values():ImageDraw.Draw(excluded).polygon(points,fill=255)
wood=ImageChops.subtract(base,excluded);deferred=ImageChops.subtract(base,wood)
wood.save(dest/'wood-domain.png');deferred.save(dest/'mixed-foliage-domain.png')
if not a.separate_root_fragments and not a.preserve_native_basal_wood:assert sha(dest/'wood-domain.png')=='6dede38d8f60ff94178f355449bad1c8cc7ce0d2e53a247f9f98a699f0849384'
for name,mask in [('known-wood.png',wood),('deferred-mixed-source.png',deferred)]:
 image=Image.open(source/'source-isolated.png').convert('RGBA');image.putalpha(mask);image.save(dest/name)
(dest/'ownership.json').write_text(json.dumps(dict(status='Private semantic source proposal; conservative mixed boundaries, not full-mask ownership',native_mask=6,input_domain_sha256=sha(source/'proposed-wood-domain.png'),known_wood_pixels=pixels(wood),additional_mixed_foliage_pixels=pixels(deferred),polygons_local_native_pixels=polygons,source_sha256=sha(source/'source-isolated.png'),wood_domain_sha256=sha(dest/'wood-domain.png'),rationale=('Native8/83 foreground subtraction already separates observed fern fronds. Retain continuous mossy basal bark/root; only crossing gold leaf cluster remains manually deferred. Source RGB unchanged.' if a.preserve_native_basal_wood else 'Crossing gold leaves and basal fern obscure the woody surface. Mixed pixels remain separate; underlying wood continues behind them. Known source RGB stays unchanged.')),indent=2)+'\n')
