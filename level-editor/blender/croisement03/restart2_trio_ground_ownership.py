"""Read-only ground/crown source comparison before a separately reviewed fill."""
from pathlib import Path
import hashlib,io,json,struct
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
OUT=B/'trio-tree-integration-v1/ownership-resolution-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);model=ROOT/'level-editor/library/3d-assets/croisement03/croisement03-terrain/model.glb';blob=model.read_bytes();n=struct.unpack_from('<I',blob,12)[0];g=json.loads(blob[20:20+n]);binary=blob[28+n:];v=g['bufferViews'][g['images'][0]['bufferView']];im=Image.open(io.BytesIO(binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']])).convert('RGB');terrain=np.array(im);rows=[]
 for tree in (12,13,14):
  if tree==13:
   source=B/'tree13-canopy-context-v1/provisional75-excluding-known-bark.png';box=[1006,0,1132,57]
  else:
   case=B/f'tree{tree}-canopy-fragment-source-v1';source=case/'000.png';box=json.loads((case/'scope.json').read_text())['absolute_interval']
  leaf=np.array(Image.open(source).convert('RGBA'));domain=leaf[:,:,3]>0;ground=np.array(im.crop(box));delta=np.abs(leaf[:,:,:3].astype(int)-ground.astype(int)).max(axis=2)
  crop=Image.new('RGB',(leaf.shape[1]*3,leaf.shape[0]),'#333333');crop.paste(Image.fromarray(ground),(0,0));source_img=Image.new('RGBA',(leaf.shape[1],leaf.shape[0]),'#333333');source_img.alpha_composite(Image.fromarray(leaf));crop.paste(source_img.convert('RGB'),(leaf.shape[1],0));marked=ground.copy();marked[domain]=[255,0,180];crop.paste(Image.fromarray(marked),(leaf.shape[1]*2,0));crop.resize((crop.width*3,crop.height*3),Image.Resampling.NEAREST).save(OUT/f'tree{tree}-ground-source-domain.png')
  mask=np.zeros(terrain.shape[:2],np.uint8);mask[box[1]:box[3],box[0]:box[2]][domain]=255;Image.fromarray(mask).save(OUT/f'tree{tree}-candidate-domain.png')
  rows.append(dict(tree=tree,source=str(source),source_sha256=sha(source),box=box,candidate_pixels=int(domain.sum()),maximum_rgb_delta_mean=float(delta[domain].mean()),ground_source_max_delta_le16=int((domain&(delta<=16)).sum()),ground_source_max_delta_le32=int((domain&(delta<=32)).sum()),status='CANDIDATE only: animation ownership does not by itself establish matching painted ground foliage'))
 scene=ROOT/'level-editor/library/scenes/croisement03.rhlos-map.json';runtime=ROOT/'level-editor/app/src/editor-viewport.ts';catalog=ROOT/'level-editor/library/mission-states/index.json';entries=json.loads(catalog.read_text());
 receipt=dict(status='HOLD terrain contains retained native painted appearance; no fill or live publication',terrain_model_sha256=sha(model),terrain_vertices=g['accessors'][g['meshes'][0]['primitives'][0]['attributes']['POSITION']]['count'],terrain_index_count=g['accessors'][g['meshes'][0]['primitives'][0]['indices']]['count'],terrain_image_size=im.size,ground_image_sha256=hashlib.sha256(terrain.tobytes()).hexdigest(),rows=rows,scene_sha256=sha(scene),runtime_sha256=sha(runtime),state_catalog_sha256=sha(catalog),limits=['Candidate domains require material classification and input review, not automatic replacement.','The ground is a full quad; native projected pixels are not excluded by geometry.','Physical and native-art presentations are distinct; no active Arbre06 sprite overlay was found in the static scene. Baked ground pixels are a separate issue.','All approved tree artifacts remain unchanged. No terrain texture synthesis performed.'])
 (OUT/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(rows,indent=2))
if __name__=='__main__':main()
