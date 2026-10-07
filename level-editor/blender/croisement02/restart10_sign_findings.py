"""Bind runtime success separately from sign visibility defects in pinned scenery."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from restart10_prepare_physical_signs import OUT,ROOT,sha


def main():
    dst=ROOT/'findings-v1';dst.mkdir(exist_ok=False)
    runtime=ROOT/'browser-v5/verification.json';r=json.loads(runtime.read_text());context=json.loads((ROOT/'input-v2/manifest.json').read_text())
    files={str(runtime):sha(runtime)};findings=[]
    for target in [5,8]:
        source=OUT/f'state-sign-candidate/native-order-reference-v3/target-{target}-visible-phase0.png';files[str(source)]=sha(source)
        mask=np.asarray(Image.open(source))>127;yy,xx=np.where(mask)
        u=np.rint((xx+32+.5)*3.2-.5).astype(int);v=np.rint((yy+16+16*math.cos(math.radians(35))+.5)*3.2-.5).astype(int)
        root=ROOT/'occlusion-v1';iso=np.asarray(Image.open(root/f'target-{target}-omit-all.png').convert('RGBA'))[v,u]
        base_path=root/f'target-{target}-omit-none.png';base=np.asarray(Image.open(base_path).convert('RGBA'))[v,u]
        assert np.array_equal(np.asarray(Image.open(base_path)),np.asarray(Image.open(ROOT/f'browser-v5/target-{target}-phase-0-native.png')))
        valid=iso[:,3]>=254;bad=valid&(np.max(abs(base[:,:3].astype(int)-iso[:,:3].astype(int)),axis=1)>20)
        cases=[]
        for dirname in (['occlusion-v1']if target==5 else ['occlusion-v1','occlusion-v2']):
            d=ROOT/dirname;proof=d/'report.json';files[str(proof)]=sha(proof)
            j=json.loads(proof.read_text())
            for row in j['views']:
                if row['target']!=target:continue
                p=d/row['file'];assert sha(p)==row['sha256'];files[str(p)]=sha(p)
                a=np.asarray(Image.open(p).convert('RGBA'))[v,u]
                recovered=bad&(np.max(abs(a[:,:3].astype(int)-iso[:,:3].astype(int)),axis=1)<=5)
                cases.append(dict(omitted=row['omit'],file=str(p),recovered=int(recovered.sum()),remaining=int((bad&~recovered).sum())))
        rows=[dict(source_x=int(context['instances'][target-4]['native_target']['position_x']-48+x),source_y=int(context['instances'][target-4]['native_target']['position_y']-64+y),render_pixel=[int(a),int(b)],isolated_opaque=bool(c),mismatch=bool(e))for x,y,a,b,c,e in zip(xx,yy,u,v,valid,bad)]
        findings.append(dict(target=target,source_visible=int(mask.sum()),isolated_opaque_samples=int(valid.sum()),baseline_mismatch=int(bad.sum()),samples=rows,omissions=cases))
        best=next(c for c in cases if c['omitted']==('croisement02-shrub-77'if target==5 else 'croisement02-tree-15+croisement02-tree-16'))
        comparison=Image.new('RGB',(1536,540),(40,40,40));draw=ImageDraw.Draw(comparison)
        for index,(p,label)in enumerate([(base_path,'Complete current context — native view'),(Path(best['file']),'Diagnostic omission: '+best['omitted']),(root/f'target-{target}-omit-all.png','Isolated exact physical sign — diagnostic only')]):
            rgba=Image.open(p).convert('RGBA');back=Image.new('RGBA',rgba.size,(55,55,55,255));back.alpha_composite(rgba);comparison.paste(back.convert('RGB'),(index*512,28));draw.text((index*512+5,8),label,fill='white')
        image=dst/f'target-{target}-attribution.png';comparison.save(image);files[str(image)]=sha(image)
    contacts=[]
    for row in r['grounding']:
        hit=row['surface_hits'][0];delta=abs(row['anchor'][1]-hit['point'][1]);assert delta<.001
        contacts.append(dict(target=row['target'],anchor=row['anchor'],surface=hit,vertical_error=delta))
    for name in ['input-v2/manifest.json','input-v2/resources-v1.json','browser-v5/physical-contract.json','review-v2/report.json']:
        p=ROOT/name;files[str(p)]=sha(p)
    review=json.loads((ROOT/'review-v2/report.json').read_text())
    for name,digest in review['files'].items():assert sha(Path(name))==digest
    report=dict(status='Runtime and placement PASS; source-order visibility HOLD for targets 5 and 8',model_sha256=r['model_sha256'],runtime_actions=[0,210,211],poses=32,cycle_ticks=64,instances=5,actual_views=180,contacts=contacts,findings=findings,files=files,
        reviewed_images=[str(ROOT/f'review-v2/target-{target}-{suffix}.png')for target in range(4,9)for suffix in ['playback8','source-comparison','all32']],
        decisions=['All five unchanged physical signs play every pose, independently clone, wrap and reset at exact native display anchors. Interaction coordinates are preserved separately.',
            'Ground uses the editor ground-only Z-up to Y-up transform; bank and other reusable GLBs retain their existing Y-up placement. Earlier browser-v4/review-v1 ground presentation is superseded, not a readiness authority.',
            'Targets 4, 6 and 7 look coherent in native and opposite views; target 7 rests on the raised bank within 0.00035 units.',
            'Target 5 native-visible post/shadow is obstructed by shrub77. Diagnostic omission recovers all 102 mismatching sampled visible centers; removing scenery is not a proposed fix.',
            'Target 8 native-visible board/post is obstructed by overlapping tree15/tree16, with a smaller bank/contact contribution. Pair omission recovers396/417 samples;21 remain. No full closure claim.',
            'Do not relocate native sign anchors, inflate their geometry or suppress whole approved neighbors to conceal the source-order issue. A bounded native-ray foliage/contact solution needs separate reviewed geometry authority.'],
        limits=['Counts sample nearest pixels of a 512px render, not exact source-center BVH rays. They exclude source-visible samples outside the isolated opaque rendered sign and do not prove complete coverage.',
            'Omission diagnosis is phase0 only; native source overlay uses its archived controlled phase. Runtime context has no newly modeled ambient motion.',
            'The unchanged application state layer is exercised in a private browser fixture, not a live catalog or installed full-editor binding.',
            'Existing sign geometry/export has scoped root review; no user approval inherited from source-only signposts. No shared catalog, runtime, source model, geometry or material edits were made.'])
    (dst/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(report['status']);print('Report',sha(dst/'report.json'))

if __name__=='__main__':main()
