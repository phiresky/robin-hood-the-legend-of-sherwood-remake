"""Publish independently reviewed combined geometry with its unclipped supplemental packet."""
import argparse,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from restart2_composed_selections import selected_workspace,validate

def main():
 p=argparse.ArgumentParser();p.add_argument('index',type=int);p.add_argument('--wide',type=Path);a=p.parse_args();old=OUT/'restart2-wood/composed-selections'/f'tree-{a.index:02}.json';receipt=json.loads(old.read_text());w=Path(receipt['worker']);validate(w);digest=sha(w/'model.blend');paths=[]
 if a.wide:
  wide=a.wide.resolve();e=json.loads((wide/'evidence.json').read_text())
  if e['model_sha256']!=digest or not e['native_first']:raise ValueError('Wrong wide review')
  directory=w/'inspection/full-crown';directory.mkdir(exist_ok=False)
  for source,name in [('solid.png','solid.png'),('textured.png','textured.png'),('views.json','cameras.json')]:
   if receipt['files'].get(str(wide/source))!=sha(wide/source):raise ValueError('Wide packet not independently bound')
   shutil.copyfile(wide/source,directory/name)
  proof=dict(model_sha256=digest,original_cameras_sha256=sha(w/'modified/views.json'),supplemental_cameras_sha256=sha(directory/'cameras.json'),solid_sha256=sha(directory/'solid.png'),textured_sha256=sha(directory/'textured.png'),native_first=True,source_evidence=str(wide/'evidence.json'),source_evidence_sha256=sha(wide/'evidence.json'));write_json(directory/'evidence.json',proof);paths.extend(directory.iterdir())
 review=dict(model_sha256=digest,sheet_sha256=sha(w/'inspection/actual-materials/sheet.png'),ready_for_geometry_review=True,preservation_evidence=str(w/'inspection/crown-wood-composition.json'),preservation_evidence_sha256=sha(w/'inspection/crown-wood-composition.json'),self_review_packet=str(w/'inspection/composed-root-review.json'),self_review_packet_sha256=sha(w/'inspection/composed-root-review.json'),notes=['NEW combined crown and separately reviewed continuous wood; exact component preservation verified.','Gray unseen bark is intentionally unknown and awaits texture completion after new geometry approval.','Previous user approval does not apply to this new combined geometry.'])
 if a.wide:review['full_crown_evidence_sha256']=sha(w/'inspection/full-crown/evidence.json');review['notes'].append('Use the supplemental full-crown material view for complete unclipped geometry; frozen comparison cameras are retained.')
 visual=w/'inspection/visual-review.json'
 if visual.exists():raise FileExistsError(visual)
 write_json(visual,review);paths.append(visual);receipt['files'].update({str(p.resolve()):sha(p) for p in paths});receipt['previous_receipt_sha256']=sha(old);new=old.with_name(old.stem+'-gallery-v2.json')
 if new.exists():raise FileExistsError(new)
 write_json(new,receipt)
 if selected_workspace(OUT,a.index,reviewed_catalog())!=w:raise ValueError('Gallery selection invalid')
 print(new)
if __name__=='__main__':main()
