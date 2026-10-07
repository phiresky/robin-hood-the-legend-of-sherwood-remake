"""Bind exact original crops beside pinned endpoint context renders."""
import json,hashlib
from pathlib import Path
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer/candidate-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    for number in [1,2,5]:
        destination=ROOT/f'profile-{number:02d}-source-comparison-v1'
        destination.mkdir(exist_ok=False);records=[]
        for state in ['initial','applied']:
            context=ROOT/f'profile-{number:02d}-context-v1'
            audit=json.loads((context/f'{state}-native-audit.json').read_text())
            original=Path(audit['source_reference']);assert sha(original)==audit['source_reference_sha256']
            render=context/f'{state}-native.png'
            old=Image.open(original).convert('RGB');new=Image.open(render).convert('RGB')
            assert new.size==(old.width*4,old.height*4)
            sheet=Image.new('RGB',(new.width*2,new.height+32),(25,25,25))
            sheet.paste(old.resize(new.size,Image.Resampling.NEAREST),(0,32));sheet.paste(new,(new.width,32))
            draw=ImageDraw.Draw(sheet);draw.text((8,8),'Original native source',fill='white')
            draw.text((new.width+8,8),'Private endpoint with unmodified static neighbors',fill='white')
            output=destination/f'{state}.png';sheet.save(output)
            records.append(dict(state=state,source=str(original),source_sha256=sha(original),render=str(render),render_sha256=sha(render),comparison=str(output),comparison_sha256=sha(output)))
        (destination/'evidence.json').write_text(json.dumps(dict(records=records,scope='Exact native crop comparison; unresolved static crown blockers remain visible.'),indent=2)+'\n')
if __name__=='__main__':main()
