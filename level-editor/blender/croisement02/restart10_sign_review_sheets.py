"""Present pinned sign playback and matching native-order crops for review."""
from pathlib import Path
import json, math, hashlib
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
BASE=ROOT/'restart10-physical-signs'

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()

def main():
    src=BASE/'browser-v5'; report=json.loads((src/'verification.json').read_text())
    out=BASE/'review-v2';out.mkdir(exist_ok=False)
    files={str(src/'verification.json'):sha(src/'verification.json')}
    for target in range(4,9):
        sheet=Image.new('RGB',(4*512,2*540),(45,45,45));d=ImageDraw.Draw(sheet)
        comparison=Image.new('RGB',(4*384,2*410),(45,45,45));cd=ImageDraw.Draw(comparison)
        native_path=ROOT/f'state-sign-candidate/native-order-reference-v3/target-{target}-motion.gif'
        files[str(native_path)]=sha(native_path);native=Image.open(native_path)
        for col,phase in enumerate((0,8,16,24)):
            for row,angle in enumerate(('native','opposite')):
                p=src/f'target-{target}-phase-{phase}-{angle}.png'
                expected=next(v['sha256'] for v in report['views'] if v['file']==p.name)
                assert sha(p)==expected;files[str(p)]=expected
                sheet.paste(Image.open(p).convert('RGB'),(col*512,row*540+28));d.text((col*512+8,row*540+8),f'Target {target}, pose {phase}, {angle}',fill='white')
            native.seek(phase);comparison.paste(native.convert('RGB').resize((384,384),Image.Resampling.NEAREST),(col*384,26))
            image=Image.open(src/f'target-{target}-phase-{phase}-native.png').convert('RGB')
            # Camera targets the physical display anchor plus 16 units upward.
            shift=16*math.cos(math.radians(35)); scale=512/160
            box=((80-48)*scale,(80-64+shift)*scale,(80+48)*scale,(80+32+shift)*scale)
            crop=image.transform((384,384),Image.Transform.EXTENT,box,Image.Resampling.BILINEAR)
            comparison.paste(crop,(col*384,436));cd.text((col*384+5,8),f'Native-order source: pose {phase}',fill='white');cd.text((col*384+5,418),'Current physical runtime / static context',fill='white')
        motion=[];timeline=Image.new('RGB',(4*256,8*276),(45,45,45));td=ImageDraw.Draw(timeline)
        for phase in range(32):
            p=src/f'target-{target}-phase-{phase}-native.png';assert sha(p)==next(v['sha256']for v in report['views']if v['file']==p.name);files[str(p)]=sha(p)
            frame=Image.open(p).convert('RGB').resize((256,256),Image.Resampling.LANCZOS);motion.append(frame)
            x=(phase%4)*256;y=(phase//4)*276;timeline.paste(frame,(x,y+20));td.text((x+5,y+5),f'Pose {phase}: native camera',fill='white')
        p=out/f'target-{target}-physical-loop.gif';motion[0].save(p,save_all=True,append_images=motion[1:],duration=80,loop=0);files[str(p)]=sha(p)
        p=out/f'target-{target}-all32.png';timeline.save(p);files[str(p)]=sha(p)
        for name,image in [(f'target-{target}-playback8.png',sheet),(f'target-{target}-source-comparison.png',comparison)]:
            p=out/name;image.save(p);files[str(p)]=sha(p)
    result={'status':'Prepared for visual inspection; not an automatic appearance pass','runtime_proof':report['status'],'model_sha256':report['model_sha256'],'files':files,'notes':['Top-left is always the original native camera direction. Bottom playback row is the opposite direction.','Native source GIF retains the archived controlled ambient phase. Physical fixture currently contains static context only; ambient phase/physical occlusion differences are not timing failures.','Source comparison uses identical display-anchor crop coordinates; bilinear resampling is presentation only and is not an exact RGB guard.']}
    (out/'report.json').write_text(json.dumps(result,indent=2)+'\n')

if __name__=='__main__':main()
