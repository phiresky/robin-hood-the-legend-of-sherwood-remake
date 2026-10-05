"""Freeze the corrected triangular log pile with its unchanged fallen endpoint."""
import json,sys
from pathlib import Path
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
    base=OUT/'restart2-state/log-triangular-pile-v5';model=base/'worker.blend';digest=sha(model);assert digest=='9ed5b9c26e29541d5116cf3d12ecd10b38869a485a7114400b54c67311af199f';audit=json.loads((base/'reopened-support-audit.json').read_text());assert audit['fallen_unchanged']
    review=dict(status='ROOT scoped triangular pile geometry PASS; exact user approval pending',model_sha256=digest,views=['actual-sheet.png','08-solid.png'],findings=['Six supporting courses form a triangular21-log mass.','Native-facing traced top preserved.','CPU silhouette219missing/456excess residual disclosed; unchanged fallen endpoint retained.'],scope='Only initial supporting-course geometry; not textures, temporal correspondence or complete state approval')
    write_json(base/'root-review.json',review)
    dest=OUT/'restart2-state/log-pile-geometry-review-v3';dest.mkdir(exist_ok=True)
    sheet=Image.new('RGB',(1024,544),(40,40,40));draw=ImageDraw.Draw(sheet)
    for i,(name,label)in enumerate([('00-solid.png','Original game camera / art view'),('08-solid.png','End-on support: six courses, broad base to top')]):
        im=Image.open(base/name).convert('RGBA');sheet.paste(im,(i*512,32),im);draw.text((i*512+8,8),label,fill='white')
    sheet.save(dest/'native-and-end-on-solid.png')
    old=OUT/'restart2-state/scoped-geometry-review-v2'
    endpoints=[dict(id='covered',status='Corrected triangular supporting courses; root scoped PASS',model_sha256=digest,solid=str(base/'solid-sheet.png'),textured=str(base/'actual-sheet.png'),context=str(OUT/'state-target-evidence/log-trap/tick--01-context.png'),review=str(base/'root-review.json'),validation=str(base/'reopened-support-audit.json'),ownership=str(base/'manifest.json')),dict(id='applied',status='Unchanged fallen endpoint; geometry/UV/material signature preserved',model_sha256=digest,solid=str(old/'log-trap-applied-solid-native-first.png'),textured=str(old/'log-trap-applied-actual-native-first.png'),context=str(OUT/'state-target-evidence/log-trap/tick-089-context.png'),review=str(base/'root-review.json'),validation=str(base/'reopened-support-audit.json'),ownership=str(base/'manifest.json'))]
    item=dict(id='croisement02-log-trap-endpoints',name='Log trap — corrected triangular pile and unchanged fallen logs',status='ready-for-user',technical_eligible=True,model=str(model),endpoint_reviews=endpoints,animation_reviews=[dict(id='support-cross-section',name='Triangular pile supporting courses',description='Original camera at left; end-on solid at right.21 complete logs in courses6/5/4/3/2/1, with14 inferred supporting bodies.',solid=str(dest/'native-and-end-on-solid.png'),textured=str(base/'actual-sheet.png'),context=str(OUT/'state-target-evidence/log-trap/tick--01-context.png'))],notes='The initial pile now has a broad supporting base narrowing to one top log; it is no longer a single inclined layer. Seven source-facing logs guide the fit;14 hidden supporting logs are inferred. One front base log has a short bank-edge overhang with its combined load center supported. Native silhouette residuals:219 missing and456 excess pixels in CPU projection, mostly edge/cap fit. All rear/unknown gray texture areas remain unfilled. The fallen endpoint is unchanged, verified by exact geometry/UV/material signature. Approve only these endpoint geometries. Motion identity, textures, sound and receiver transitions remain separate.',approval_scope='Exact initial and fallen endpoint geometry in model '+digest+' only; no texture or motion approval.')
    manifest=dict(map='Croisement02 corrected log pile geometry',review_kind='geometry',items=[item],scope='Every eight-view sheet starts with the original35-degree game camera at top-left. Supporting cross-section also keeps original camera first. Root scoped geometry PASS; new exact user decision required before log texture generation.',status_counts={'corrected endpoint geometry awaiting user approval':1})
    write_json(dest/'review-candidates.json',manifest);build(dest/'review-candidates.json',dest/'gallery')
    write_json(dest/'camera-contract-verification.json',dict(status='PASS',native_first=True,initial_render_manifest_sha256=sha(base/'manifest.json'),unchanged_fallen_evidence_sha256=sha(old/'camera-contract-verification.json'),cross_section_sha256=sha(dest/'native-and-end-on-solid.png'),model_sha256=digest))
    print(dest/'gallery/index.html')
if __name__=='__main__':main()
