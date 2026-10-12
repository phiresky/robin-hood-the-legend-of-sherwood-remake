"""Whole visible plant envelopes and source contacts, without pixel ownership."""
import argparse, hashlib, json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement'
W=B/'restart2'

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output
 assert not out.exists();out.mkdir();pins={}
 def pin(path):
  pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 src=Image.open(pin(B/'baseline/covered.png')).convert('RGBA')
 inv=json.loads(pin(B/'source-inventory.json').read_text())
 pin(W/'tree18-source-taxonomy-v1/receipt.json');pin(W/'tree18-priority-source-proposals-v1/receipt.json')
 anim=next(a for a in inv['animations'] if a['index']==4);union=np.zeros((src.height,src.width),bool);phases=[]
 for i,row in enumerate(anim['frames']):
  raw=np.array(Image.open(pin(Path(row['source']))).convert('RGBA'));assert pins[row['source']]==row['sha256']
  normalized=np.array(Image.open(pin(Path(row['image']))).convert('RGBA'));expected=raw.copy();expected[np.all(raw[:,:,:3]==[0,251,0],axis=2)]=0;assert np.array_equal(expected,normalized)
  x,y,w,h=row['bbox'];assert normalized.shape==(h,w,4);alpha=np.zeros_like(union);alpha[y:y+h,x:x+w]=normalized[:,:,3]>0;union|=alpha;phases.append(alpha)
 assert int(union.sum())==22406
 data=[
 {'id':'left-bank-climbing-ivy','class':'green ivy with gold tips and connected lower shrub','outline':[[143,395],[166,385],[191,389],[215,384],[237,374],[254,353],[271,352],[285,364],[302,361],[317,378],[335,396],[342,416],[329,438],[332,460],[351,479],[356,507],[342,525],[325,537],[327,557],[300,564],[276,553],[253,558],[236,548],[213,551],[196,531],[174,518],[169,487],[153,465],[143,444]],'interior':[[164,403],[204,399],[234,393],[256,374],[280,377],[305,385],[326,403],[321,427],[316,451],[329,480],[337,502],[317,525],[299,540],[273,532],[249,538],[224,529],[203,509],[193,485],[172,461],[158,439]],'rationale':'Small connected leaf clusters spread across the lower left bank and climb into the gold-tipped hanging foliage. The source-facing leaf cover is a broad connected mass, with branchlets and dark inter-leaf gaps. Lower-left continuity extends beyond this bank study; boundary at x143 is a study cut, not a measured plant termination.','contacts':{'upper_bank_contact':[[166,385],[191,389],[215,384],[237,374],[254,353]],'right_stone_edge':[[335,396],[342,416],[329,438],[332,460],[351,479],[356,507]],'lower_ground_transition':[[174,518],[196,531],[213,551],[253,558],[300,564]]},'uncertainty':'Outer twigs, individual leaf silhouette and dark interiors remain mixed; enclosed pixels are not an ivy assignment. Lower continuation may be a separate foreground shrub.'},
 {'id':'right-bank-shrub','class':'green shrub with visible woody twig network','outline':[[477,363],[490,350],[508,352],[526,346],[542,357],[551,371],[568,373],[582,389],[589,408],[582,423],[563,433],[549,442],[526,444],[508,437],[487,440],[474,429],[471,409],[463,392]],'interior':[[482,378],[497,363],[515,366],[529,361],[539,379],[561,386],[575,398],[571,415],[553,425],[530,433],[511,424],[490,426],[480,409]],'rationale':'A continuous small-leaf shrub occupies the right bank face behind and around the slender branching stem. Its lower silhouette meets grass near y438; a connected crown exists despite internal dark/branch gaps. It is not proven to belong to Tree18.','contacts':{'bank_back_contact':[[477,363],[490,350],[508,352],[526,346],[542,357]],'grass_contact':[[474,429],[487,440],[508,437],[526,444],[549,442],[563,433]],'visible_twig_axis':[[524,443],[521,422],[532,399],[539,378]]},'uncertainty':'Visible twig structure requires separate wood role. Dark inter-leaf windows and stone glimpses are not filled by this envelope.'},
 {'id':'tree18-root-ivy-collar','class':'green ivy around trunk foot and bank base','outline':[[425,466],[436,476],[443,488],[451,486],[455,471],[470,466],[485,475],[491,493],[484,514],[473,531],[452,540],[426,540],[411,529],[412,511],[418,496],[414,481]],'interior':[[423,496],[437,499],[452,498],[463,487],[475,480],[480,495],[471,516],[455,528],[435,530],[420,522]],'rationale':'Green cover wraps around the lower illuminated trunk and continues right toward the bank/ground foot. Source bark remains visible above y488; the ivy rises unevenly at both sides rather than defining a flat cut through the trunk.','contacts':{'bark_ivy_boundary':[[425,466],[436,476],[443,488],[451,486],[455,471]],'ground_contact':[[411,529],[426,540],[452,540],[473,531]],'bank_left_transition':[[414,481],[418,496],[412,511]]},'uncertainty':'Trunk-root junction and dark left bank recess are unresolved; no actual ground elevation or independent root ownership is measured. Separate this collar from observed bark and lower-left diagonal root-like form.'},
 {'id':'gold-tree18-crown-envelope','class':'gold leaf and twig crown envelope','outline':[[224,267],[247,234],[259,219],[286,205],[310,193],[337,185],[366,176],[393,184],[420,175],[448,181],[472,197],[497,201],[522,220],[548,228],[563,247],[581,264],[592,288],[582,315],[568,337],[549,351],[527,358],[509,374],[487,379],[470,394],[450,393],[435,374],[418,361],[401,349],[382,356],[365,376],[345,393],[328,419],[307,447],[288,459],[271,450],[277,427],[279,400],[261,388],[250,370],[234,360],[227,339],[212,319],[214,289]],'interior':None,'rationale':'Broad crown envelope follows the visibly connected gold sprays around the main fork, together with full native phase extents. Lower-left hanging spray connects downward but must remain separate from the green ground cover. This is an inferred 3D envelope, not a static leaf pixel domain.','contacts':{'main_fork_connection':[[431,361],[434,339],[449,316]],'left_branch_connection':[[434,339],[403,321],[376,304]],'right_branch_connection':[[449,316],[477,286],[496,257]],'lower_left_hanging_spray':[[345,393],[328,419],[307,447],[288,459]]},'uncertainty':'Full actual native alpha may extend beyond hand envelope. Use all14 extents as observed constraints, never clip native to polygon. Background rock, bark, small gaps and unidentified static leaves are not crown material assignments.'}
 ]
 gaps=[{'id':'central-stone-ledge-chain','outline':[[354,348],[391,349],[416,370],[412,401],[431,421],[421,445],[405,469],[368,467],[363,449],[386,427],[377,407],[363,389]],'role':'Visible rock ledges, dark joints, crossing twigs and ivy; retain mixed context, no blanket stone assignment.'},{'id':'main-trunk-reserve','outline':[[422,352],[442,343],[455,369],[452,406],[457,448],[452,479],[445,489],[432,481],[430,450],[421,414]],'role':'Observed wood with leaves crossing upper fork; exact650 bark proposal separate. Never place opaque inferred foliage front over visible trunk.'},{'id':'left-fork-stone-mixture','outline':[[347,325],[369,325],[383,344],[366,363],[351,357],[345,343]],'role':'Pale stone visible between many gold twigs;31pixel prior proposal is separate, rest unresolved.'}]
 rows=[]
 for entry in data:
  mask=Image.new('L',src.size);ImageDraw.Draw(mask).polygon([tuple(q)for q in entry['outline']],fill=255);m=np.array(mask)>0
  entry.update(status='REGIONAL_ENVELOPE_PROPOSAL_NOT_PIXEL_DOMAIN',physical_owner='INFERRED_UNASSIGNED',envelope_pixels=int(m.sum()),native_union_overlap=int((m&union).sum()),native_per_phase_overlap=[int((m&a).sum())for a in phases])
  if entry['interior']:entry['interior_semantics']='Connected observed leaf-cover neighborhood, including unresolved tiny holes; not an exact source mask.'
  rows.append(entry)
 marked=src.convert('RGB');d=ImageDraw.Draw(marked)
 for i,e in enumerate(rows):
  d.line([tuple(q)for q in e['outline']+[e['outline'][0]]],fill='#ffab44',width=1)
  if e['interior']:d.line([tuple(q)for q in e['interior']+[e['interior'][0]]],fill='#44ffff',width=1)
  d.text(tuple(e['outline'][0]),str(i+1),fill='white')
  for line in e['contacts'].values():d.line([tuple(q)for q in line],fill='#ff55cc',width=1)
 for gap in gaps:d.line([tuple(q)for q in gap['outline']+[gap['outline'][0]]],fill='white',width=1)
 nativepanel=src.convert('RGB');na=np.array(nativepanel);na[union]=[160,70,235];nativepanel=Image.fromarray(na)
 def sheet(name,box,scale):
  w=(box[2]-box[0])*scale;h=(box[3]-box[1])*scale;pw=w+55;im=Image.new('RGB',(pw*3,h+65),'#242424');draw=ImageDraw.Draw(im)
  for i,(label,panel)in enumerate([('UNMARKED SOURCE',src),('ENVELOPES / CONTACTS, NOT OWNERS',marked),('ACTUAL NATIVE UNION PRIORITY',nativepanel)]):
   x=i*pw;im.paste(panel.crop(box).convert('RGB').resize((w,h),Image.Resampling.NEAREST),(x+45,45));draw.text((x+3,4),label,fill='white');draw.text((x+3,18),f'XYXY {box}',fill='white')
   for px in range(box[0],box[2]):
    if px%25==0:draw.text((x+45+(px-box[0])*scale,32),str(px),fill='white')
   for py in range(box[1],box[3]):
    if py%25==0:draw.text((x+6,45+(py-box[1])*scale),str(py),fill='white')
  im.save(out/name)
 sheet('whole-regional-source.png',(135,165,610,575),2)
 sheet('left-ivy-source-and-contacts.png',(140,345,370,565),3)
 sheet('right-shrub-source-and-contacts.png',(457,340,600,455),4)
 sheet('root-collar-source-and-contacts.png',(400,455,500,550),6)
 sheet('gold-crown-source-and-gaps.png',(205,168,600,465),2)
 unionimg=Image.fromarray(union.astype('uint8')*255);unionimg.save(out/'native-union-priority-only.png')
 yy,xx=np.where(union)
 report={'status':'COMPLETE_REGIONAL_SOURCE_ROLE_AND_GEOMETRY_PROPOSAL_ONLY','pins':pins,'regions':rows,'reserved_mixed_context':gaps,'native_union':{'count':int(union.sum()),'bbox_xyxy':[int(xx.min()),int(yy.min()),int(xx.max()+1),int(yy.max()+1)],'all14_exact_rgba_conversion':True},'geometry_recipe_constraints':['Construct separate coherent closed plant volumes for left climbing mass, right shrub and root collar; retain connected leaf-cover fronts while leaving source-visible rock/wood corridors uncovered. Physical grouping remains inferred.','Round crown hidden depth must be at least its width unless independent source evidence changes that hypothesis. Use only own native artwork and the two authorized Leicester trees as shape references.','Anchor source-facing boundaries by native projection; choose hidden depth and contact elevation only after joined bank/wood/terrain review. No editor animation plane is a measured plant-depth constraint.','Root collar must wrap around bark, not truncate wood at a uniform source row. Lower-left root-like diagonal remains separate unresolved context.','Never use envelope or interior neighborhood as a projected opaque texture mask. Static leaf ownership needs separate exact source-domain review.','Observed native all14 alpha keeps one existing owner and priority. Inferred rear foliage must pass exact native view guard and preserve confirmed stone/bark windows.'],'limits':['No mesh, assignments, texture synthesis, donor mixing or canonical edits.','Orange broad outlines, cyan connected cover neighborhoods and magenta source contacts are hand-traced visual proposals; border width and tiny gaps are not measured pixel domains.','White context boundaries reserve mixed stone/wood windows; neither all their pixels nor complement pixels are assigned.','Left mass reaches study boundary and extends farther left/down in source; that study cut cannot become a mesh edge.'],'output_pins':{p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in out.iterdir()if p.is_file()}}
 (out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['id'],r['envelope_pixels'],r['native_union_overlap'])for r in rows])
if __name__=='__main__':main()
