"""Independent visible rock-face proposals; occluded outline portions stay inferred."""
import json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-visible-face-traces-v1'
def main():
 O.mkdir(exist_ok=True);source=B.parent/'baseline/covered.png';im=Image.open(source).convert('RGB')
 rows=[
 ('south-upper-boulder',54,[(392,361),(405,357),(413,359),(414,364),(411,368),(402,371),(394,369),(390,365)],'Broad light face below dark joint; left edge partly foliage-obscured.'),
 ('south-middle-boulder',54,[(377,380),(390,376),(400,372),(411,371),(413,375),(409,381),(398,385),(386,387),(377,385)],'Separate sloping face; upper dark joint separates it from the light upper boulder.'),
 ('south-ivy-boulder',54,[(391,388),(402,385),(411,387),(411,394),(406,401),(392,407),(383,404),(382,396)],'Partially visible rock under ivy; outer continuation is inferred, not a source ownership mask.'),
 ('south-low-boulder',54,[(350,421),(359,422),(370,427),(378,429),(378,438),(373,442),(359,436),(350,430)],'Lower exposed block; top and right outline partly under foreground leaves.'),
 ('east-top-right',52,[(596,191),(609,192),(614,199),(615,207),(610,212),(604,208),(595,204)],'Rounded top-right lip; lower dark crack is visible.'),
 ('east-upper-right',52,[(593,213),(600,212),(611,218),(612,232),(605,237),(595,237),(590,229)],'Vertical right-hand stone face beneath upper crack.'),
 ('east-middle-knob',52,[(585,240),(591,238),(597,242),(599,250),(596,256),(590,257),(586,253)],'Distinct small convex knob bordering deep right-hand recess.'),
 ('east-lower-right',52,[(603,258),(610,258),(615,264),(615,282),(610,289),(599,288),(595,278),(595,268)],'Broad lower stone face; projecting shoulder and dark left/up joint.'),
 ('east-left-column',52,[(566,238),(577,233),(585,239),(589,253),(588,277),(582,291),(572,294),(566,281)],'Long exposed left face; foliage obscures parts of its left boundary.'),
 ]
 records=[]
 for name,owner,poly,reason in rows:records.append({'id':name,'owner':owner,'native_outline':[list(p)for p in poly],'evidence':reason,'outline_status':'Visual proposal; visible interior supported, partly occluded boundary inferred.','uncertainty_native_px':3,'depth_status':'Unknown; infer convex volume behind source face with independently checked contacts.'})
 for label,box in [('south',(335,333,426,448)),('east',(548,183,636,356))]:
  raw=im.crop(box).resize(((box[2]-box[0])*5,(box[3]-box[1])*5));marked=raw.copy();draw=ImageDraw.Draw(marked)
  for i,row in enumerate(records):
   if not row['id'].startswith(label):continue
   poly=[((x-box[0])*5,(y-box[1])*5)for x,y in row['native_outline']];draw.line(poly+[poly[0]],fill='cyan',width=2);cx=sum(x for x,y in poly)/len(poly);cy=sum(y for x,y in poly)/len(poly);draw.text((cx,cy),str(i),fill='white')
  sheet=Image.new('RGB',(raw.width*2,raw.height));sheet.paste(raw);sheet.paste(marked,(raw.width,0));sheet.save(O/(label+'-full-face-proposals.png'))
 report={'status':'PRIVATE_VISIBLE_FACE_PROPOSALS_NOT_OWNERSHIP','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'faces':records,'legacy_trace_interpretation':'Retain original2D sample paths. Paired upper/lower paths do not establish a complete physical rock course; do not stretch their strip into a broad planar face.','geometry_recipe':'Remesh front surfaces around these complete face regions, preserving exact existing perimeter/shared crest and neighboring volumes; convex face interiors and recessed inter-face joints need saved native and oblique review.','limits':['No source pixels assigned by these outlines.','Hidden foliage-covered surfaces and depths remain inferred.','Independent first-hit/source, topology, crest and neighbor checks mandatory.']};(O/'faces.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
