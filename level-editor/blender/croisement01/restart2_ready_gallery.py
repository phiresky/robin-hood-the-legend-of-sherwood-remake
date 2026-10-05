"""Freeze the two independently reviewed forest trees for user geometry review."""
import hashlib
import json
import shutil
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
from restart2_review_labels import labeled_sheet
OUT=ROOT/'level-editor/work/croisement01-refinement/restart2'
CASES=[('tree18-v4','croisement01-tree-18','Northeast Forked Forest Tree','95b1fc167963cc59bb2722f26296b8e10b11fe9ec7984218ef129930f5ca97cb'),('tree20-v3','croisement01-tree-20','Northern Shaded Forest Tree','58060c730823ea1ac2bf31e82e5bb20d923fb68f1cb10b1517a5f691fc035472')]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    dest=OUT/'ready-trees18-20-v2';dest.mkdir(exist_ok=False)
    (dest/'models').mkdir();(dest/'receipts').mkdir();items=[];archive=[]
    for folder,asset,name,expected in CASES:
        w=OUT/folder/'assets'/asset;receipt_path=w/'inspection/root-review.json';receipt=json.loads(receipt_path.read_text())
        assert sha(w/'model.blend')==expected==receipt.get('model_sha256',receipt.get('hashes',{}).get('model.blend'))
        assert json.loads((w/'validation.json').read_text())['status']=='PASS'
        evidence={}
        for relative in ['model.blend','validation.json','modified/views.json','modified/solid.png','modified/textured.png','inspection/actual-materials/sheet.png','inspection/native-source/comparison.png','inspection/terrain-contact/sheet.png','inspection/terrain-contact/evidence.json','inspection/root-review.json','inspection/self-review.json','inspection/saved-tree-geometry.json','inspection/native-geometry-coverage/report.json']:
            evidence[relative]=sha(w/relative)
        bound=receipt.get('files',receipt.get('hashes',{}))
        for relative,digest in bound.items():assert sha(w/relative)==digest
        frozen=dest/'models'/f'{asset}-{expected[:16]}.blend';shutil.copy2(w/'model.blend',frozen);assert sha(frozen)==expected
        notes=['Geometry review only. Hidden bark and the complete inferred off-map crown are gray because appearance fill is pending.','The first/top-left tile is the original game orthographic camera in every geometry, actual-material and contact sheet.','Crown depth is greater than width. Only the Leicester southeast cottage and moat bank trees informed external tree construction references.','Foreground foliage/neighboring branch ownership and final scene integration remain separate unfinished work. Approval here covers this tree geometry, not those joints or whole-map completion.']
        review=dest/'receipts'/f'{asset}.json';review.write_text(json.dumps(dict(status='ready-for-user-geometry-review',model_sha256=expected,root_receipt=receipt,self_review=json.loads((w/'inspection/self-review.json').read_text()),evidence=evidence,limitations=notes,user_approved=False,texture_approved=False),indent=2)+'\n')
        ownership=[p for p in (w/'projection').glob('*/ownership.json') if p.parent.name!='input'];assert len(ownership)==1
        items.append(dict(id=asset,name=name,status='ready-for-user',technical_eligible=True,model=str(frozen),solid=str(labeled_sheet(w,'modified/solid.png')),textured=str(labeled_sheet(w,'modified/textured.png')),context=str(w/'modified/context.png'),stored_material_textured=str(labeled_sheet(w,'inspection/actual-materials/sheet.png')),source_comparison=str(w/'inspection/native-source/comparison.png'),source_comparison_label='Original game artwork, actual saved material and overlay',source_comparison_secondary=str(labeled_sheet(w,'inspection/terrain-contact/sheet.png')),source_comparison_secondary_label='Ground contact against provisional archival terrain; original camera first',validation=str(w/'validation.json'),ownership=str(ownership[0]),review=str(review),notes=notes))
        archive.append(dict(asset_id=asset,model=str(frozen.relative_to(dest)),model_sha256=expected,worker=str(w),evidence=evidence))
    manifest=dest/'review-candidates.json';manifest.write_text(json.dumps(dict(map='Crossings01 tree geometry',items=items,status_counts={'ready for geometry review':2,'user approved':0}),indent=2)+'\n')
    build(manifest,dest/'gallery',pending_only=True)
    files={str(p.relative_to(dest)):sha(p) for p in dest.rglob('*') if p.is_file()}
    (dest/'archive.json').write_text(json.dumps(dict(status='immutable user geometry review snapshot',assets=archive,files=files),indent=2)+'\n')
    print(dest/'gallery/index.html')
if __name__=='__main__':main()
