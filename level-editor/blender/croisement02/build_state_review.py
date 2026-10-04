"""Build a native-frame state inspector; never synthesize endpoint artwork."""
import argparse
import json
from pathlib import Path
from catalog import OUT

HTML = '''<!doctype html><meta charset="utf-8"><title>Croisement02 native state review</title>
<style>body{background:#20241f;color:#eee;font:16px system-ui;margin:24px}select,input{margin:8px;padding:8px}canvas{max-width:100%;background:#555}pre{white-space:pre-wrap} .note{max-width:1000px;color:#ccd}</style>
<h1>Croisement02 native state review</h1><p class="note">Source evidence only. Mission patches are mission-scoped, not permanent scenery. The preview composites one selected sprite onto the source background; it does not simulate actor sorting, other active patches or script timing. A missing final state disables the sprite. Applied preview retains the last transition frame only when integrate_in_background is true. Frame delay and sound IDs are retained exactly, without assuming a real-time tick rate.</p>
<label>Mission <select id="mission"></select></label><label>Patch <select id="patch"></select></label><label>State <select id="state"></select></label><br>
<label>Frame <input id="frame" type="range" min="0" value="0"></label><label><input id="context" type="checkbox" checked>Source context</label><pre id="info"></pre><canvas id="view"></canvas>
<script type="module">
const data=await (await fetch('../source/layers.json')).json();
const mission=document.querySelector('#mission'),patch=document.querySelector('#patch'),state=document.querySelector('#state'),frame=document.querySelector('#frame'),context=document.querySelector('#context'),info=document.querySelector('#info'),canvas=document.querySelector('#view');
const bg=new Image(); bg.src='covered.png'; await bg.decode();
const options=(node,values)=>node.replaceChildren(...values.map(([value,label])=>Object.assign(document.createElement('option'),{value,textContent:label})));
options(mission,[...new Set(data.mission_patches.map(p=>p.mission))].map(x=>[x,x]));
const selected=()=>data.mission_patches.find(p=>p.id===patch.value);let token=0;
async function render(){const request=++token,p=selected(),frames=p.states[state.value==='applied'?'final':state.value]?.frames??[];frame.max=Math.max(0,frames.length-1);frame.value=Math.min(+frame.value,+frame.max);const f=frames[+frame.value];
 const all=Object.values(p.states).flatMap(s=>s.frames);const left=Math.max(0,Math.min(...all.map(f=>f.bbox[0]))-50),top=Math.max(0,Math.min(...all.map(f=>f.bbox[1]))-50),right=Math.min(bg.width,Math.max(...all.map(f=>f.bbox[0]+f.bbox[2]))+50),bottom=Math.min(bg.height,Math.max(...all.map(f=>f.bbox[1]+f.bbox[3]))+50);
 let baked;if(state.value==='applied'&&p.state.integrate_in_background){const last=p.states.transition?.frames.at(-1);if(last){baked={frame:last,image:new Image()};baked.image.src='../source/'+last.image;await baked.image.decode();}}let image;if(f){image=new Image();image.src='../source/'+f.image;await image.decode();}if(request!==token)return;
 canvas.width=right-left;canvas.height=bottom-top;const c=canvas.getContext('2d');if(context.checked)c.drawImage(bg,left,top,canvas.width,canvas.height,0,0,canvas.width,canvas.height);if(baked)c.drawImage(baked.image,baked.frame.bbox[0]-left,baked.frame.bbox[1]-top);if(image)c.drawImage(image,f.bbox[0]-left,f.bbox[1]-top);
 info.textContent=JSON.stringify({patch:p.id,profile:p.name,runtime_patch_index:p.runtime_patch_index,state:state.value,frame:+frame.value,frames:frames.length,delay:f?.delay,sound_id:f?.sound_id,bbox:f?.bbox,definitive:p.state.definitive,integrate_in_background:p.state.integrate_in_background,missing_state:!f},null,2);}
function choosePatch(){const p=selected();options(state,['initial','transition','final','applied'].map(s=>[s,`${s} (${p.states[s==='applied'?'final':s]?.frames.length??0} frames)`]));frame.value=0;render();}
function chooseMission(){options(patch,data.mission_patches.filter(p=>p.mission===mission.value).map(p=>[p.id,`${p.runtime_patch_index}: ${p.name}`]));choosePatch();}
mission.onchange=chooseMission;patch.onchange=choosePatch;state.onchange=()=>{frame.value=0;render()};frame.oninput=render;context.onchange=render;chooseMission();
</script>'''

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('candidate',type=Path)
    args=parser.parse_args()
    root=args.candidate
    assert (root/'verification.json').is_file(), 'Verify candidate before building review'
    dest=root/'review';dest.mkdir(exist_ok=False)
    (dest/'covered.png').write_bytes((OUT/'baseline/covered.png').read_bytes())
    (dest/'index.html').write_text(HTML)
    print(dest/'index.html')

if __name__=='__main__':main()
