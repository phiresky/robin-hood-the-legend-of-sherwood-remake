"""Coordinate-level native layer attribution for private hidden foliage endpoints."""
import ast,hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
ROOT=BASE/'restart14-hidden-archer/audit-v1'
REFERENCE=Path(__file__).with_name('restart7_reference_remaining_patches.py')
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    # Reuse only the frozen source compositor declarations, never its run loop.
    tree=ast.parse(REFERENCE.read_text());declarations=[]
    for node in tree.body:
        if isinstance(node,(ast.Import,ast.ImportFrom,ast.FunctionDef)):declarations.append(node)
        elif isinstance(node,ast.Assign) and not any(isinstance(t,ast.Name) and t.id=='records' for t in node.targets):declarations.append(node)
        else:break
    env={};exec(compile(ast.Module(body=declarations,type_ignores=[]),str(REFERENCE),'exec'),env)
    image,frame,phase_frame,order,paint=[env[k] for k in ['image','frame','phase_frame','order','paint']]
    authority=json.loads((ROOT/'source-authority.json').read_text())
    source_dir=BASE/'restart7-source-patch-delivery/contracts-v1/source-review-v1'
    refs=json.loads((source_dir/'manifest.json').read_text())
    output=ROOT/'source-role-v1';output.mkdir(exist_ok=False);records=[]
    for profile in authority['profiles']:
        contract=json.loads(Path(profile['source_contract']).read_text());native=contract['native']
        selected=next(p for p in native['patch_states'] if p['id']==contract['focus_patch_id'])
        for state in ['initial','applied']:
            reference=next(r for r in refs['images'] if r['profile']==profile['profile'] and r['label'].lower()==state)
            tick=reference['tick'];dst=image(native['background']).copy();owners=np.zeros(dst.shape[:2],dtype=np.int16);layers=[dict(id='background',kind='base-art',path=native['background']['path'])]
            def layer(f,pos,label,kind,metadata):
                if f is None:return
                layers.append(dict(id=label,kind=kind,frame=f,position=pos,**metadata));number=len(layers)-1
                paint(dst,f,pos,native['origin']);src=image(f);x=math.floor(pos[0]+f['offset'][0]-native['origin'][0]);y=math.floor(pos[1]+f['offset'][1]-native['origin'][1]);h,w=src.shape[:2];H,W=owners.shape
                x0,y0=max(0,x),max(0,y);x1,y1=min(W,x+w),min(H,y+h)
                if x1>x0 and y1>y0:
                    part=owners[y0:y1,x0:x1];alpha=src[y0-y:y1-y,x0-x:x1-x,3];part[alpha>0]=number
            if state=='applied':layer(selected['transition'][-1],selected['display_position'],selected['id']+'-stamp','applied-background-stamp',{})
            elements=[]
            for e in native['elements']:
                f=e.get('initial_frame') if not e['active'] else frame(e['frames'],max(0,tick),e['loop'])
                if e['active'] or f:elements.append({**e,'frame':f})
            for p in native['patch_states']:
                f=phase_frame(p,tick) if p['id']==selected['id'] else frame(p['initial'],max(0,tick),p['initial_loop'])
                if f is None:continue
                if p['layer']=='background':layer(f,p['display_position'],p['id'],'background-patch',{})
                else:elements.append({**p,'frame':f})
            for e in order(elements):layer(e['frame'],e['display_position'],e['id'],e.get('source',{}).get('kind','element'),dict(display_order=e['display_order'],creation_order=e['creation_order'],polyline=e['polyline']))
            assert hashlib.sha256(dst.tobytes()).hexdigest()==reference['full_rgba_sha256'],reference
            audit=BASE/f'restart14-hidden-archer/candidate-v2/profile-{int(profile["profile"][-2:]):02d}-context-v1/{state}-native-audit.json'
            d=json.loads(audit.read_text());rows=[]
            for sample in d['blocked_native_visible_centers']:
                x,y=sample['pixel'];receiver=layers[int(owners[y,x])]
                rows.append(dict(pixel=[x,y],physical_blocker=sample['first_hit'],physical_world=sample['world'],native_frontmost=receiver['id'],native_kind=receiver['kind'],native_rgba=dst[y,x].tolist()))
            crop=reference['crop'];x0,y0,x1,y1=crop;shown=Image.fromarray(dst[y0:y1,x0:x1]).convert('RGB');overlay=shown.copy();pix=overlay.load()
            for r in rows:pix[r['pixel'][0]-x0,r['pixel'][1]-y0]=(255,0,255)
            scale=4;sheet=Image.new('RGB',((x1-x0)*scale*2,(y1-y0)*scale+28),(25,25,25));sheet.paste(shown.resize(((x1-x0)*scale,(y1-y0)*scale),Image.Resampling.NEAREST),(0,28));sheet.paste(overlay.resize(((x1-x0)*scale,(y1-y0)*scale),Image.Resampling.NEAREST),((x1-x0)*scale,28));draw=ImageDraw.Draw(sheet);draw.text((5,5),'Exact native '+state,fill='white');draw.text(((x1-x0)*scale+5,5),'Magenta: visible source blocked by static crown',fill='white');path=output/f'{profile["profile"][-2:]}-{state}.png';sheet.save(path)
            records.append(dict(profile=profile['profile'],state=state,reference=reference,source_compositor_full_rgba_exact=True,blocked=len(rows),native_frontmost_counts={key:sum(r['native_frontmost']==key for r in rows) for key in sorted({r['native_frontmost'] for r in rows})},samples=rows,layers=layers,image=str(path),image_sha256=sha(path),context_audit_sha256=sha(audit)))
    write=dict(status='Read-only exact native ordering proof; no crown ownership edits',records=records,compositor_recipe_sha256=sha(REFERENCE),limits=['Last nonzero alpha layer is recorded; exact composed RGBA is separately verified.','Observed material labels do not establish semantic crown ownership.','Native ordering is a presentation contract, not arbitrary-view physical depth parity.'])
    (output/'report.json').write_text(json.dumps(write,indent=2)+'\n')
    print([(r['profile'],r['state'],r['blocked'],r['native_frontmost_counts']) for r in records])
if __name__=='__main__':main()
