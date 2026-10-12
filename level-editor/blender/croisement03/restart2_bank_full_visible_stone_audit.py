"""Manual source-visible stone proposal, with foreground and uncertain boundaries retained."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_erosion
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';W=B/'restart2'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);out=p.parse_args().output;assert not(out/'receipt.json').exists();out.mkdir(exist_ok=True);pins={}
 def pin(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return path
 def read(path):return json.loads(pin(path).read_text())
 def image(path):return np.array(Image.open(pin(path)).convert('RGBA'))
 src=image(B/'baseline/covered.png');revealed=image(B/'baseline/revealed.png');shape=src.shape[:2];size=(shape[1],shape[0]);inventory=read(B/'source-inventory.json');manifest=read(B/'baseline/masks/manifest.json');prior=read(W/'bank-main-masses-source-v1/projection.json');assert pins[str(B/'baseline/covered.png')]==prior['source_sha256'];old=np.zeros(shape,bool)
 for x,y,*_ in prior['protected_firsthits']:old[y,x]=True
 for x,y in prior['accepted_pixels']:old[y,x]=True
 assert int(old.sum())==3446+7811
 for path in [W/'bank-left-exposed-stone-proposal-v1/proposal.json',W/'bank-face-source-proposal-v1/proposal.json',W/'bank-visible-face-traces-v1/faces.json',W/'bank-foreground-source-audit-v2/receipt.json']:read(path)
 def polygon(points):
  im=Image.new('L',size);ImageDraw.Draw(im).polygon([tuple(p)for p in points],fill=255);return np.array(im)>0
 # Independently reviewed visible source patches. Patch borders are NOT whole-rock 3D seams.
 regions=[
 ('west-cap',[[151,207],[165,188],[182,174],[197,168],[220,171],[245,178],[265,186],[277,199],[273,221],[262,240],[244,254],[220,267],[198,265],[183,254],[168,245],[157,231]],'Rounded west cap and long pale front bands; moss coloration belongs to surface where grain remains continuous.'),
 ('upper-central-cap',[[248,177],[276,166],[306,161],[334,164],[360,168],[382,166],[405,168],[431,163],[457,161],[480,165],[505,160],[530,157],[547,161],[560,173],[565,187],[553,201],[532,209],[506,216],[474,217],[446,214],[415,213],[383,216],[352,211],[321,214],[294,207],[270,195]],'Exposed plateau stone continues between moss and foreground branches; twig and foliage exclusions are deliberately conservative.'),
 ('west-upper-stack',[[137,275],[144,264],[154,259],[164,260],[174,266],[187,273],[198,281],[203,289],[198,296],[182,301],[166,298],[148,292],[138,285]],'Small rounded face below cap; left shoulder ownership against neighboring cap remains unresolved.'),
 ('west-middle-stack',[[126,298],[139,290],[153,289],[171,295],[187,302],[199,309],[204,319],[198,328],[181,334],[161,336],[144,332],[130,326],[124,314]],'Wide banded stone face; preserve weathering and dark mineral marks, exclude true clefts separately.'),
 ('west-low-stack',[[128,353],[138,346],[154,344],[174,347],[188,353],[198,361],[201,372],[192,382],[175,389],[155,392],[139,387],[129,378],[123,367]],'Broad lower stacked mass, upper moss continuous with stone; leafy basal edge withheld.'),
 ('east-broad-shoulder',[[549,190],[564,193],[580,199],[594,209],[593,220],[589,231],[577,233],[561,225],[550,214],[544,202]],'Broad mossy shoulder between plateau and right cheeks; source texture exists beyond prior narrow face samples.'),
 ('east-cap-rim',[[562,181],[579,188],[598,190],[611,191],[616,197],[616,206],[612,214],[599,213],[587,207],[570,202]],'Exposed far-right cap lip and rounded shoulder.'),
 ('east-upper-cheek',[[590,212],[601,212],[611,217],[615,226],[613,239],[607,244],[596,242],[590,233]],'Visible continuous right cheek below top joint.'),
 ('east-middle-knob',[[585,239],[591,237],[598,241],[601,248],[599,256],[591,260],[584,255]],'Small shoulder next to vertical recess.'),
 ('east-lower-cheek',[[600,257],[611,256],[616,262],[616,282],[611,293],[598,294],[592,284],[592,269]],'Lower right exposed face; dark side crevice is not filled.'),
 ('east-inner-column',[[567,235],[577,231],[585,237],[590,249],[590,276],[583,293],[573,298],[566,286]],'Inner column stone partly behind gold leaves and ivy; exact native reservations plus manual twig exclusion apply.'),
 ('south-upper-window',[[385,358],[398,353],[410,355],[416,360],[415,367],[405,373],[392,374],[385,367]],'Source-visible stone window beneath branches; outer continuation under leaves stays ambiguous.'),
 ('south-middle-window',[[374,379],[388,374],[402,368],[413,369],[416,377],[409,384],[396,390],[382,392],[373,385]],'Sloped middle stone face between branches and ivy.'),
 ('south-lower-window',[[383,389],[395,384],[408,385],[414,390],[412,399],[404,406],[391,411],[379,406],[378,398]],'Partially exposed lower face; ivy and trunk-right side withheld.'),
 ('south-foot-window',[[347,420],[357,419],[370,425],[380,429],[382,437],[374,445],[359,439],[349,433]],'Low rock face before basal vegetation; no inferred hidden side source claim.')]
 exclusions=[
 ('cap-gold-twig-left',[[312,179],[327,176],[346,183],[351,200],[336,207],[316,198]],'Manually localized small gold twig/leaf cluster; all remaining alpha-clear flecks withheld, not stone by complement.'),
 ('cap-gold-twig-central',[[396,183],[416,176],[441,181],[451,198],[449,215],[426,219],[406,207]],'Fine gold foliage and twig network at plateau lower edge; includes non-native/static uncertain flecks.'),
 ('cap-gold-twig-right',[[451,180],[470,175],[482,184],[480,202],[466,215],[449,208]],'Sparse highlighted leaf/twig microfeatures; broad exclusion intentionally retains ambiguity between flecks.'),
 ('cap-gold-twig-far-right',[[520,189],[542,188],[558,204],[553,224],[530,221],[516,208]],'Right gold spray crosses mossy face; uncertain alpha-clear source specks withheld.'),
 ('west-cap-fringe-twig',[[254,202],[267,198],[281,211],[279,233],[263,240],[253,224]],'Small twig and gold leaf fringe on west cap right boundary.'),
 ('northeast-tree-base',[[566,155],[581,153],[586,176],[584,190],[580,203],[572,199],[568,181]],'Visible ivy-clad tree base crossing cap silhouette; not stone or an inferred texture donor.'),
 ('west-cap-left-dark-cleft',[[142,254],[153,254],[157,269],[153,281],[147,291],[140,287]],'Dark seam and uncertain adjacent boulder owner; includes prior102 ownership-disagreement neighborhood.'),
 ('west-stack-joint',[[127,334],[144,336],[166,338],[185,334],[188,344],[169,349],[144,346],[127,343]],'Deep gap/ivy between middle and lower faces.'),
 ('west-basal-ivy',[[124,383],[148,389],[173,389],[197,379],[207,398],[197,417],[141,417],[126,403]],'Leafy basal boundary; hidden rock continuation cannot be projected from this art.'),
 ('central-twig-and-leaf-network',[[281,211],[316,213],[352,226],[389,231],[425,226],[465,222],[503,223],[541,217],[558,246],[558,278],[539,310],[495,339],[443,348],[396,344],[350,338],[310,331],[269,319],[253,285],[263,248]],'Dense static/animated twig and leaf network. Native-alpha holes here are not automatically stone. Fine exposed windows remain unresolved.'),
 ('west-front-gold-ivy',[[211,278],[240,260],[264,253],[278,277],[270,317],[248,343],[218,349],[207,325]],'Gold ivy obscures smaller front faces; no whole-region bark/stone classification.'),
 ('east-ivy-and-twig-strip',[[548,245],[568,248],[576,272],[572,302],[558,329],[541,335],[533,318]],'Gold twig/ivy strip on inner east boundary.'),
 ('south-left-ivy',[[347,349],[375,347],[389,369],[378,391],[361,415],[342,418],[332,396]],'Ivy masking left edges of southern rock windows.'),
 ('south-trunk-and-shadow',[[415,335],[432,337],[445,361],[458,386],[462,428],[451,448],[428,444],[414,409],[412,379]],'Tree18 trunk, root and shadow; no bark or rock inferred from grayscale.'),
 ('south-foot-leaves',[[345,407],[367,407],[383,415],[389,433],[379,440],[376,429],[356,421],[346,423]],'Foreground small leaves crossing low stone shoulder.')]
 # Narrow visually identified twigs across otherwise broad visible plateau patches.
 twigs=[('plateau-diagonal-left',[(270,174),(282,194),(297,212)],4),('plateau-central-left',[(324,166),(330,187),(342,211)],4),('plateau-central',[(397,162),(406,187),(412,214)],5),('plateau-right',[(446,158),(443,183),(444,214)],5)]
 native=np.zeros(shape,bool);native_proof=[];phase_masks={}
 for index in [4,7]:
  animation=next(a for a in inventory['animations']if a['index']==index);phase_masks[index]=[]
  for i,row in enumerate(animation['frames']):
   raw=image(Path(row['source']));assert pins[row['source']]==row['sha256'];key=np.all(raw[:,:,:3]==[0,251,0],axis=2);raw[key]=0;rgba=image(Path(row['image']));assert np.array_equal(raw,rgba);x0,y0,w,h=row['bbox'];assert (w,h)==(rgba.shape[1],rgba.shape[0]);mask=np.zeros(shape,bool);mask[y0:y0+h,x0:x0+w]=rgba[:,:,3]>0;native|=mask;phase_masks[index].append(mask);native_proof.append({'animation':index,'phase':i,'native_box_xywh':row['bbox'],'raw_sha256':row['sha256'],'normalized_sha256':pins[row['image']],'full_rgba_exact':True})
 ambiguous=np.zeros(shape,bool);amb_rows=[]
 for name,points,why in exclusions:
  mask=polygon(points);ambiguous|=mask;amb_rows.append({'id':name,'polygon':points,'reason':why,'pixels':int(mask.sum())})
 for name,points,width in twigs:
  im=Image.new('L',size);ImageDraw.Draw(im).line([tuple(p)for p in points],fill=255,width=width);mask=np.array(im)>0;ambiguous|=mask;amb_rows.append({'id':name,'polyline':points,'width':width,'reason':'Conservative twig/shadow corridor, not measured bark domain.','pixels':int(mask.sum())})
 envelope=np.zeros(shape,bool);boundary=np.zeros(shape,bool);rows=[]
 for name,points,why in regions:
  mask=polygon(points);edge=mask&~binary_erosion(mask,structure=np.ones((3,3),bool));envelope|=mask;boundary|=edge;rows.append({'id':name,'outline':points,'evidence':why,'raw_pixels':int(mask.sum()),'native_foreground_pixels':int((mask&native).sum()),'manual_ambiguous_pixels':int((mask&ambiguous&~native).sum()),'source_patch_not_component_owner':True})
 boundary&=~native&~ambiguous;candidate=envelope&~native&~ambiguous&~boundary;additional=candidate&~old
 scope=np.zeros(shape,bool)
 for index in [96,108,109]:
  m=next(m for m in manifest['masks']if m['index']==index);im=np.array(Image.open(pin(B/'baseline/masks'/m['png'])))>0;x,y=m['box_top_left'];scope[y:y+im.shape[0],x:x+im.shape[1]]|=im
 scope|=envelope;classes=np.zeros(shape,np.uint8);classes[scope]=1;classes[scope&candidate]=2;classes[scope&boundary]=3;classes[scope&ambiguous]=4;classes[scope&native]=5
 counts={name:int((classes==idx).sum())for idx,name in [(1,'unresolved_not_background'),(2,'visible_stone_interior_proposal'),(3,'manual_outline_boundary_review'),(4,'manual_ambiguous_foreground_or_seam'),(5,'actual_native_foreground_any_phase')]};assert sum(counts.values())==int(scope.sum())
 for name,mask in [('visible-interior-proposal',candidate),('additional-interior-proposal',additional),('boundary-review',boundary),('manual-ambiguous',ambiguous&scope),('native-foreground',native&scope),('unresolved',classes==1)]:Image.fromarray(mask.astype('uint8')*255).save(out/f'{name}.png')
 Image.fromarray(classes).save(out/'accounting-classes.png');colors={1:[100,100,100],2:[30,220,240],3:[255,90,200],4:[255,180,30],5:[90,80,245]};marked=src[:,:,:3].copy()
 for k,c in colors.items():
  if k==1:continue
  m=classes==k;marked[m]=(.45*marked[m]+.55*np.array(c)).astype('uint8')
 for name,box,scale in [('whole-bank',(110,140,630,450),2),('west',(120,155,285,410),3),('east',(535,170,630,335),4),('south',(335,335,470,452),4)]:
  w=(box[2]-box[0])*scale;h=(box[3]-box[1])*scale;sheet=Image.new('RGB',(w*2,h+45),'#252525');sheet.paste(Image.fromarray(src).convert('RGB').crop(box).resize((w,h),Image.Resampling.NEAREST),(0,25));sheet.paste(Image.fromarray(marked).crop(box).resize((w,h),Image.Resampling.NEAREST),(w,25));d=ImageDraw.Draw(sheet);d.text((4,6),'Unmarked covered source',fill='white');d.text((w+4,6),'Cyan proposal / pink boundary / amber uncertain / violet native',fill='white');sheet.save(out/f'{name}-source-and-overlay.png')
 contexts=Image.new('RGB',(1040,645),'#252525');box=(110,140,630,450)
 for i,(name,im)in enumerate([('Covered',src),('Revealed',revealed)]):contexts.paste(Image.fromarray(im).convert('RGB').crop(box),(i*520,25));ImageDraw.Draw(contexts).text((i*520+4,7),name,fill='white')
 for i,phase in enumerate([0,7]):
  base=Image.fromarray(src).convert('RGBA');animation=next(a for a in inventory['animations']if a['index']==4);row=animation['frames'][phase];base.alpha_composite(Image.open(row['image']).convert('RGBA'),tuple(row['bbox'][:2]));contexts.paste(base.convert('RGB').crop(box),(i*520,335));ImageDraw.Draw(contexts).text((i*520+4,317),f'Full native Arbre05 phase{phase:02}',fill='white')
 contexts.save(out/'covered-revealed-native-context.png')
 for row in rows:
  mask=polygon(row['outline']);row.update(candidate_pixels=int((mask&candidate).sum()),additional_candidate_pixels=int((mask&additional).sum()),retained_prior_samples_inside=int((mask&old).sum()))
 report={'status':'HUMAN_REVIEW_SOURCE_PROPOSALS_NOT_COMPLETE_OWNERSHIP','pins':pins,'native_proof':native_proof,'regions':rows,'ambiguous_regions':amb_rows,'scope':'Union of bank masks96/108/109 and manual stone envelopes; masks delimit audit extent only, never material ownership. Neighbor boulders outside this scope not silently assigned to bank52/53/54.','counts':counts,'scope_pixels':int(scope.sum()),'prior_samples_unchanged':11257,'candidate_overlap_prior':int((candidate&old).sum()),'additional_interior_proposals':int(additional.sum()),'proposed_boundary_pixels':int(boundary.sum()),'covered_revealed_different_pixels_in_scope':int((scope&np.any(src!=revealed,axis=2)).sum()),'prior_samples_crossing_current_native_reservation':int((old&native).sum()),'interpretation':['Gray model regions coincident with visible-interior proposals have source artwork available: lack of source projection/receiver coverage must be tested before calling them hidden or requesting synthesis.','West cap margins and broad stacked faces, east cheeks and several southern windows visibly expose stone beyond sparse original seed dots; retain surface moss/weathering where texture is continuous.','Pink manual outline boundaries require pixel-level review. They are not approved segmentation or measured physical boulder seams.','Central canopy gaps, ivy edges and dark clefts remain unresolved: alpha complement is not stone, background or evidence of a particular mesh owner.','Rear/underside and foliage-obscured continuation may need inferred geometry/texture, but source-visible gray fronts are a separate unfinished projection problem. No source-image pixel count can measure hidden3D surface area.'],'next':['Root reviews adjacent unmarked and overlay images per patch. Narrow/refine any leaf/twig ambiguity before acceptance.','After source review, exact saved receiver test each accepted candidate and boundary pixel against full joined bank/tree scene; preserve old3446 and7811 lists, record all mismatches rather than reassign automatically.','Complete central fine-window segmentation and external neighbor ownership remain open; this packet explicitly accounts for unresolved pixels instead of claiming100% material ownership.'],'limits':['No source assignment, geometry/model changes or synthesis. All proposed candidates require independent review.','Native reservations verified against all14 full Arbre05 and Arbre08 frames; no occupancy-mask or color-only expansion.','Existing source samples are immutable evidence, not proof that all manually uncertain pixels belong to stone.']};(out/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'counts':counts,'additional':int(additional.sum()),'covered_revealed_diff':report['covered_revealed_different_pixels_in_scope']}))
if __name__=='__main__':main()
