"""CPU source sections and explicit depth hypotheses for four separate wood parts."""
import json,math,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/tree01-width-depth-constraints-v1'
def main():
 assert not O.exists();O.mkdir();level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());source=Image.open(B/'baseline/covered.png').convert('RGB');mark=source.copy();draw=ImageDraw.Draw(mark);s=math.sin(math.radians(35));c=math.cos(math.radians(35))
 # Visible approximate wood extents, not a pixel ownership mask; omit leaf crossings.
 sections={2:[(224,77,82),(233,79,86),(243,80,88),(250,81,89)],3:[(153,76,82),(163,74,81),(172,73,80),(190,73,80),(198,74,81)],4:[(142,104,110),(152,105,112),(162,108,115),(181,112,120),(191,115,124),(202,119,126),(213,122,130),(223,125,132)],5:[(132,148,154),(142,146,153),(151,145,151),(163,144,150),(174,142,149),(183,142,148),(191,141,147),(203,139,146),(214,137,144)]}
 colors={2:'#00ffff',3:'#ff55ff',4:'#44aaff',5:'#ffffff'};rows=[]
 for node,samples in sections.items():
  native=level['sight_obstacles'][node];depth=sum(p['y']for p in native['points'])/len(native['points']);records=[]
  for sy,left,right in samples:
   x=(left+right)/2;width=right-left;z=depth-sy;center=[x,-depth/s,z/c]
   assert abs(-center[1]*s-center[2]*c-sy)<1e-10
   records.append({'native_y':sy,'left_x':left,'right_x':right,'center_world':center,'observed_width':width,'inferred_horizontal_depth':width,'boundary_uncertainty_pixels':1.5})
   draw.line((left,sy,right,sy),fill=colors[node],width=1)
  rows.append({'source_obstacle_context':node,'authored_depth_reference':depth,'sections':records,'gameplay_metadata':native,'unknowns':['Exact foreground/background boundary under leaves','Hidden continuation below last visible bark','Ground/root contact and whether neighboring stems join outside view']})
 box=(45,100,170,270);sheet=Image.new('RGB',(1000,680))
 for i,im in enumerate((source,mark)):sheet.paste(im.crop(box).resize((500,680),Image.Resampling.NEAREST),(500*i,0))
 sheet.save(O/'source-width-sections.png')
 report={'status':'CPU_GEOMETRY_CONSTRAINTS_ONLY','parts':rows,'depth_rule':'Horizontal depth equals visible width initially; preserve projection by placing section centers on native rays. Authored depth is a starting hypothesis, not evidence of exact physical bark depth.','continuation_rule':'Do not cap at last visible sample. Continue behind foliage only after local bank/root contact review; no arbitrary flat ground or merged fork.','scope':'Wood-only four separate source context parts. Canopy belongs to a separate source-role investigation.','protected':['No source RGB/alpha edits','No gameplay metadata changes','No frozen Tree02/shared crest changes'],'next':'Build bounded swept wood sections, compare exact native source and eight views; adjust section depth along rays only where source/bank contacts justify it.'}
 (O/'constraints.json').write_text(json.dumps(report,indent=2)+'\n');print(O/'constraints.json')
if __name__=='__main__':main()
