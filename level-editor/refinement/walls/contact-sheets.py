import json,math
from pathlib import Path
from PIL import Image,ImageDraw,ImageFont
root=Path('work/needs-review/wall-presets');rows=json.loads((root/'candidates.json').read_text())
font=ImageFont.load_default(size=12)
for source in sorted(set(x['source_map'] for x in rows)):
 entries=[x for x in rows if x['source_map']==source and (root/'candidates'/f"{x['id']}.webp").exists()]
 for page in range(math.ceil(len(entries)/18)):
  group=entries[page*18:page*18+18];out=Image.new('RGB',(1200,math.ceil(len(group)/3)*168),'#171c22');draw=ImageDraw.Draw(out)
  for i,row in enumerate(group):
   x=(i%3)*400;y=(i//3)*168
   im=Image.open(root/'candidates'/f"{row['id']}.webp").resize((400,140))
   out.paste(im,(x,y+28));draw.text((x+4,y+2),row['id'],fill='white',font=font)
   draw.text((x+4,y+15),row['name'][:53],fill='#aaaaaa',font=font)
  out.save(root/f'{source.lower()}-{page+1}.jpg')
 print(source,len(entries))
