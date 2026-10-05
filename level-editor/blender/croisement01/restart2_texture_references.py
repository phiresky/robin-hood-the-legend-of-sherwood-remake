"""Attach only permitted Leicester material crops, without copying their layout."""
import json
from pathlib import Path
from PIL import Image
from restart2_ready_gallery import ROOT,OUT,sha
REFERENCES=[('leicester-southeast-cottage-tree','9a8974c11e28d00c9729745d71fdc25d00385fb48a002e8b51833e5809fd5312',(78,78,153,128)),('leicester-moat-bank-tree','6a18f955edc12e45fc5f05892022419f07aa527b9960c16a651645ce8c593392',(96,83,157,133))]
def main():
    for asset in ('croisement01-tree-18','croisement01-tree-20'):
        e=OUT/'approved-tree-fills-v1'/asset/'experiment';folder=e/'material-references';folder.mkdir(exist_ok=False);refs=[]
        for name,revision,box in REFERENCES:
            source=ROOT/'level-editor/work/leicester-refinement/round-1/texture-review/approved-evidence'/name/revision/'textured.png'
            target=folder/(name+'-leaf-material.png');Image.open(source).crop(box).save(target)
            refs.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=name,role='Observed leafy texture and palette example only. Do not copy geometry or layout. Use the target silhouette, branch structure, camera layout, lighting and existing native pixels. This crop excludes gray unknown patches.',parent_image=str(source),parent_sha256=sha(source),crop_box=list(box),crop_method='Exact existing pixels; no resampling or recoloring.',authorization='User explicitly permitted this named Leicester tree as supplementary reference.'))
        (e/'auxiliary-references.json').write_text(json.dumps(dict(version=1,input_sha256=sha(e/'input.png'),lighting_sha256=sha(e/'solid.png'),references=refs),indent=2)+'\n')
if __name__=='__main__':main()
