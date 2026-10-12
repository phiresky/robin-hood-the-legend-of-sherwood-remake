"""Extend plant envelopes to visible contacts and reserve interior source windows."""
import argparse,hashlib,json,copy
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';W=B/'restart2'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;out.mkdir(exist_ok=True);assert not(out/'receipt.json').exists();pins={}
 def pin(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 old=json.loads(pin(W/'tree18-regional-plants-source-v1/receipt.json').read_text());src=Image.open(pin(B/'baseline/covered.png')).convert('RGB');catalog=json.loads(pin(B/'catalog.json').read_text());pin(B/'asset-hierarchy.json');union=np.array(Image.open(pin(W/'tree18-regional-plants-source-v1/native-union-priority-only.png')))>0
 rows=copy.deepcopy(old['regions']);left=rows[0]
 left['outline']=[[151,399],[170,390],[192,388],[215,384],[237,374],[254,353],[271,352],[285,364],[302,361],[317,378],[335,396],[342,416],[329,438],[332,460],[351,479],[356,507],[340,527],[321,535],[307,548],[296,561],[279,576],[259,573],[239,563],[218,552],[203,543],[191,530],[176,517],[173,501],[152,496],[143,488],[145,477],[151,465],[139,454],[128,444],[135,431],[145,423]]
 left['contacts']={'upper_bank_contact':[[151,399],[170,390],[192,388],[215,384],[237,374],[254,353]],'west_stone_and_recess_contact':[[151,399],[145,423],[135,431],[128,444],[139,454],[151,465],[145,477],[143,488],[152,496],[173,501]],'right_stone_edge':[[335,396],[342,416],[329,438],[332,460],[351,479],[356,507]],'lower_leaf_cover_to_grass':[[176,517],[191,530],[203,543],[218,552],[239,563],[259,573],[279,576]],'foreground_bright_shrub_occlusion':[[279,576],[296,561],[307,548],[321,535],[340,527]]}
 left['rationale']='Full-map source shows a bounded broad climbing leafy mass with a lower brown-green fine-branched shrub lobe. Its west edge turns around exposed boulder/recess at x128..173,y423..501; no straight x143 study edge remains. Lower-left leaf fringe meets grass, while lower-right disappears behind the separate brighter foreground shrub. Source continuity supports a joined visual envelope but does not prove one biological plant or catalog owner.'
 left['uncertainty']='Lower-right closure is an observed occlusion boundary, not the plant physical termination. Hidden continuation behind the bright foreground shrub is inferred. Brown lower lobe may be a separate touching shrub; retain a possible subpart seam.'
 left['interior']=None;left.pop('interior_semantics',None)
 left['proposed_subpart_seam']=[[178,492],[203,487],[230,491],[260,493],[288,503],[321,518]]
 for row in rows:
  m=Image.new('L',src.size);ImageDraw.Draw(m).polygon([tuple(q)for q in row['outline']],fill=255);m=np.array(m)>0;row['envelope_pixels']=int(m.sum());row['native_union_overlap']=int((m&union).sum());row.pop('native_per_phase_overlap',None)
 reserves=[
 {'id':'left-west-exposed-boulder','outline':[[108,459],[127,455],[140,464],[143,478],[142,491],[155,500],[164,511],[152,526],[134,532],[118,522],[107,503]],'role':'Exposed boulder with dark side; exterior contact, not a foliage donor.','required_action':'Keep plant west indentation and do not bridge opaque leaves across this stone.'},
 {'id':'left-right-stone-window','outline':[[340,438],[363,437],[369,448],[378,455],[370,470],[355,473],[344,461]],'role':'Visible stone ledge with a mixed ivy fringe.','required_action':'Keep source-visible rock window; exact boundary ownership remains pending.'},
 {'id':'right-shrub-main-stem','line':[[523,439],[521,419],[519,403],[522,388],[525,371]],'review_width_px':6,'role':'Visible narrow gray-brown woody stem; crossing leaves remain mixed.','required_action':'Reserve exposed twig/stem corridor from opaque front foliage; future woody twig receiver separate.'},
 {'id':'right-shrub-right-branch','line':[[521,420],[537,409],[552,406],[568,402]],'review_width_px':5,'role':'Diagonal woody branch visible through shrub.','required_action':'Retain source-facing branch opening; inferred back foliage may lie behind only.'},
 {'id':'right-shrub-upper-branch','line':[[520,403],[533,388],[542,374]],'review_width_px':5,'role':'Upward gray twig in green leaf cover.','required_action':'Reserve from opaque leaves; width is review corridor, not measured bark mask.'},
 {'id':'right-shrub-inner-dark-gap','outline':[[525,398],[535,397],[541,401],[535,408],[526,412]],'role':'Dark gap between branches; stone versus deep foliage not independently known.','required_action':'Keep unresolved window open in observed front; do not assign dark pixels to leaf or rock by complement.'},
 {'id':'right-upper-stone-window','outline':[[490,343],[499,338],[509,344],[512,353],[502,360],[493,355]],'role':'Pale bank stone framed by gold/green leaves.','required_action':'Do not stretch shrub upper surface across this window.'},
 {'id':'root-left-wood-and-shadow','outline':[[412,465],[425,469],[432,480],[435,493],[430,504],[421,509],[414,500],[410,485]],'role':'Source-visible dark longitudinal trunk/root-like wood beside ivy, with unresolved shadow.','required_action':'Separate wood/shadow corridor from ivy front; do not infer solid foliage because dark.'},
 {'id':'root-main-bark-continuation','outline':[[435,478],[445,482],[449,491],[444,500],[438,497],[433,489]],'role':'Bark/ivy interleaving below the conservative650pixel bark proposal. Not newly approved bark.','required_action':'Preserve visual strip until finer boundary proof; root collar cannot be a horizontal opaque cap.'},
 {'id':'root-central-dark-window','outline':[[430,508],[443,508],[448,517],[442,525],[430,522]],'role':'Deep dark recess enclosed by ivy; exact material and contact unknown.','required_action':'Leave observed source opening; hidden depth cannot be derived from black pixels.'},
 {'id':'root-ground-and-diagonal-root','outline':[[399,525],[415,523],[430,530],[453,529],[470,528],[466,540],[442,547],[416,541],[399,538]],'role':'Mixed illuminated earth, root-like diagonal and green fringe.','required_action':'Root ivy stops along uneven contact; no all-leaf assignment or inferred ground height from this neighborhood.'}]
 marked=src.copy();d=ImageDraw.Draw(marked)
 for i,e in enumerate(rows):
  d.line([tuple(q)for q in e['outline']+[e['outline'][0]]],fill='#ffac40',width=1);d.text(tuple(e['outline'][0]),str(i+1),fill='white')
  for line in e['contacts'].values():d.line([tuple(q)for q in line],fill='#ff55cc',width=1)
 for e in reserves:
  if 'outline'in e:d.line([tuple(q)for q in e['outline']+[e['outline'][0]]],fill='#44ffff',width=1)
  else:d.line([tuple(q)for q in e['line']],fill='#44ffff',width=1)
 def sheet(name,box,scale):
  w=(box[2]-box[0])*scale;h=(box[3]-box[1])*scale;pw=w+55;im=Image.new('RGB',(pw*2,h+65),'#222222');dr=ImageDraw.Draw(im)
  for i,(title,source)in enumerate([('UNMARKED SOURCE',src),('ORANGE ENVELOPE / CYAN RESERVED WINDOWS',marked)]):
   ox=i*pw;im.paste(source.crop(box).resize((w,h),Image.Resampling.NEAREST),(ox+45,45));dr.text((ox+4,4),title,fill='white');dr.text((ox+4,18),str(box),fill='white')
   for x in range(box[0],box[2]):
    if x%25==0:dr.text((ox+45+(x-box[0])*scale,32),str(x),fill='white')
   for y in range(box[1],box[3]):
    if y%25==0:dr.text((ox+7,45+(y-box[1])*scale),str(y),fill='white')
  im.save(out/name)
 sheet('complete-left-natural-contacts.png',(95,340,390,600),3);sheet('right-shrub-retained-openings.png',(460,330,590,450),5);sheet('root-collar-retained-openings.png',(395,455,495,552),6)
 report={'status':'COMPLETE_VISIBLE_PLANT_CONTACT_PROPOSALS_NO_SOURCE_ASSIGNMENT','pins':pins,'regions':rows,'reserved_openings_and_context':reserves,'native_priority':'All22406 actual union pixels retain priority. Exact14 phase source proof remains pinned in v1; no clipping or changes.','inventory_evidence':{'catalog_groups':len(catalog['groups']),'conclusion':'Catalog and hierarchy contain obstacle-based tree/rock groups, but inspected records do not establish a separate exact static ivy or neighboring bright-shrub source owner for this visible boundary. Do not invent independent ownership from rock group names.','source_scope':'Full map covered image inspected across x0..660,y300..700; upper source context retained from v1.'},'prototype_guidance':['Use natural exposed-stone west indentation and lower leaf-to-grass boundary, not a study rectangle.','Lower-right bright foreground shrub is a visible occluder; close hidden inferred plant volume behind it, not at the source occlusion line as measured physical end.','Allow lower brown-green lobe as separately editable inferred subpart connected visually to upper climbing cover.','Green shrub and root collar need open source fronts around listed twig/wood/stone windows; a single opaque envelope face is invalid.','Reserved review corridors include mixed pixels and are constraints to inspect, not exact approved material masks.','Closed inferred rear volume and porous source-facing geometry are separate: preserve round bulk behind openings without filling observed holes.'],'output_pins':{p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in out.iterdir()if p.is_file()}}
 (out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print({'left_bounds':[min(q[0]for q in left['outline']),min(q[1]for q in left['outline']),max(q[0]for q in left['outline']),max(q[1]for q in left['outline'])],'reserves':len(reserves)})
if __name__=='__main__':main()
