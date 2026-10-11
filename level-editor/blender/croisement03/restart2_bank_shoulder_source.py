"""Source-supported shoulder breaks for the next complete bank revision."""
import json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/bank-shoulder-source-v1'
def main():
 O.mkdir(exist_ok=True);assert not(O/'traces.json').exists();source=B/'baseline/covered.png';src=Image.open(source).convert('RGB');im=src.copy();d=ImageDraw.Draw(im)
 traces=[{'id':'east-upper-outer-shoulder','owner':52,'points':[[604,189],[610,195],[613,205],[612,213],[616,223],[614,234],[612,243],[609,251]],'color':'#00ffff'}, {'id':'east-lower-outer-shoulder','owner':52,'points':[[603,257],[610,260],[613,269],[610,278],[613,286],[608,296],[602,307]],'color':'#ff66ff'}, {'id':'south-upper-short-shoulder','owner':54,'points':[[402,345],[410,348],[416,354],[412,361],[418,367],[414,374],[409,382]],'color':'#00ffff'}, {'id':'south-low-tip-shoulder','owner':54,'points':[[365,409],[376,414],[379,421],[376,427],[380,432],[374,438]],'color':'#ff66ff'}]
 for trace in traces:d.line(trace['points'],fill=trace['color'],width=1)
 for name,box in [('east',(550,180,640,355)),('south',(300,330,440,465))]:
  w,h=box[2]-box[0],box[3]-box[1];sheet=Image.new('RGB',(w*8,h*4))
  for i,image in enumerate((src,im)):sheet.paste(image.crop(box).resize((w*4,h*4),Image.Resampling.NEAREST),(i*w*4,0))
  sheet.save(O/f'{name}-source-traces.png')
 p=B/'restart2/bank-whole-geometry-plan-v4/geometry.json';record={'status':'SOURCE_TRACE_PROPOSAL_NOT_OWNERSHIP','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'retained_geometry_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'traces':traces,'geometry_recipe':['Retain all34 existing source crease anchors and entire fixed crest/53 baseline.','Use short shoulder segments to split the east front sheet into connected stone faces and recessed joints. Do not turn each shoulder into a full circular band.','Keep body behind strata and retain closed lower and rear surfaces.','Fit hidden depth by bank-native intersection constraints; never raise the ridge to expose lower traces.','Check silhouette uncertainty1–3native pixels in saved native render before assigning source material.'],'reservations':['Ivy and exposed shrub pixels remain unassigned.','Right edge of south shoulder touches foreground tree; do not extend source ownership onto bark.','The gap251–257 on east face is a recessed joint, not permission to remove supporting bulk.'],'next':'Add these constraints to a new closed52/54 geometry array, preserving all existing valid traces and neighbor guards.'}
 (O/'traces.json').write_text(json.dumps(record,indent=2)+'\n');print(O/'traces.json')
if __name__=='__main__':main()
