"""Record independently inspected native appearance samples without inventing depth."""
import json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-source-role-audit-v1'
def main():
 O.mkdir(exist_ok=True);source=B.parent/'baseline/covered.png';im=Image.open(source).convert('RGB');crop=im.crop((0,0,175,175)).resize((700,700));draw=ImageDraw.Draw(crop)
 samples=[
  ('gold-leaf-01',[128,16,135,23],'visible gold foliage','high','Gold leaf cluster; geometry depth remains inferred.'),
  ('gold-leaf-02',[90,28,96,34],'visible gold foliage','high','Distinct bright leaves; not inferred from absent alpha.'),
  ('wood-01',[103,109,107,120],'visible woody stem','high','Continuous narrow stem with longitudinal bark; preserve independently of leaf samples.'),
  ('wood-02',[149,111,153,125],'visible woody stem','high','Right stem continuous through the lower canopy.'),
  ('shadow-01',[112,110,121,118],'dark backdrop or shadowed foliage','unresolved','Dark olive textured window; appearance is observed but single source does not settle whether it belongs to this crown or farther foliage.'),
  ('shadow-02',[120,96,126,103],'dark backdrop or shadowed foliage','unresolved','Do not turn into bright gold fill or claim physically empty space without further source-role evidence.'),
  ('shadow-03',[41,31,48,38],'dark backdrop or shadowed foliage','unresolved','Dark window among upper leaves; retain explicit ownership uncertainty.'),
  ('neighbor-green',[7,152,20,165],'neighbor green foliage','high','Lower green plant is excluded from the gold crown source proposal.'),
 ]
 records=[]
 for index,(name,box,role,confidence,reason)in enumerate(samples):
  color='yellow'if role=='visible gold foliage'else'cyan'if role=='visible woody stem'else'lime'if role=='neighbor green foliage'else'magenta';draw.rectangle(tuple(v*4 for v in box),outline=color,width=2);draw.text((box[0]*4,box[1]*4-12),str(index),fill=color);sample=im.crop(box);records.append(dict(id=name,native_box=box,role=role,confidence=confidence,reason=reason,rgb_sha256=hashlib.sha256(sample.tobytes()).hexdigest()))
 crop.save(O/'independent-role-samples.png');report={'status':'PARTIAL_SOURCE_ROLE_AUDIT_NOT_GEOMETRY_AUTHORITY','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'samples':records,'confirmed_physical_empty_background_pixels':0,'selected_domain_complement_is_background':False,'limits':['Appearance samples are independent of selected leaf alpha; not a complete segmentation.','Dark samples may be shadowed own foliage or farther foliage; no inferred geometry deletion is authorized solely by their low brightness.','Keep source wood, native animation and off-map volume separate; no changes to canonical ownership.']};(O/'roles.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
