"""Build an inspectable browser proof for planar native appearance clips."""
import json
from pathlib import Path
from catalog import OUT

TEMPLATE='''<!doctype html><meta charset="utf-8"><title>Native appearance proof</title>
<style>body{background:#333;color:white;font:16px system-ui;margin:24px}canvas{image-rendering:pixelated;max-width:90vw}button,input{margin:8px}</style>
<h1>ASSEMBLY native appearance</h1><p>Exact timed native artwork on a plane. This is an appearance fallback, not recovered 3D body motion. Endpoint geometry and scene integration remain separate.</p><button id="play">Play</button><input id="tick" type="range" min="0" max="TERMINAL" value="0"><span id="status">Loading</span><div id="view"></div>
<script type="importmap">{"imports":{"three":"THREE/build/three.module.js","three/addons/":"THREE/examples/jsm/"}}</script><script type="module">
import * as THREE from 'three';import{GLTFLoader}from'three/addons/loaders/GLTFLoader.js';
const manifest=await(await fetch('manifest.json')).json(),renderer=new THREE.WebGLRenderer({alpha:true,antialias:false,preserveDrawingBuffer:true});renderer.setPixelRatio(1);renderer.setSize(manifest.width,manifest.height);renderer.domElement.style.width=manifest.width*3+'px';renderer.domElement.style.height=manifest.height*3+'px';document.querySelector('#view').append(renderer.domElement);renderer.setClearColor(0,0);renderer.toneMapping=THREE.NoToneMapping;
const scene=new THREE.Scene(),camera=new THREE.OrthographicCamera(-manifest.width/2,manifest.width/2,manifest.height/2,-manifest.height/2,.1,1000);camera.position.z=100;
try{const gltf=await new GLTFLoader().loadAsync('native-appearance.glb');scene.add(gltf.scene);if(gltf.animations.length!==1)throw Error('Expected one clip');const mixer=new THREE.AnimationMixer(gltf.scene),action=mixer.clipAction(gltf.animations[0]);action.setLoop(THREE.LoopOnce,1);action.clampWhenFinished=true;action.play();let playing=false,current=0;
function show(tick){current=tick;action.enabled=true;action.paused=false;mixer.setTime(tick/25+0.000001);gltf.scene.updateMatrixWorld(true);renderer.render(scene,camera);document.querySelector('#tick').value=tick;document.querySelector('#status').textContent='Tick '+tick+' / '+(tick/25).toFixed(2)+' s';}
show(0);document.querySelector('#tick').oninput=e=>{playing=false;show(+e.target.value)};document.querySelector('#play').onclick=()=>{if(current>=manifest.terminal_tick)show(0);playing=!playing};setInterval(()=>{if(playing){if(current>=manifest.terminal_tick)playing=false;else show(current+1)}},40);
window.nativeAppearance={loaded:true,show,capture:()=>renderer.domElement.toDataURL(),async verify(){playing=false;const samples=[];for(let i=0;i<manifest.source_bindings.length;i++){const row=manifest.source_bindings[i];show(row.first_tick);const active=gltf.scene.children.filter(o=>o.scale.x>.5);if(active.length!==1||active[0].userData.native_first_tick!==row.first_tick)throw Error('Wrong visible phase '+i);const image=await new Promise((resolve,reject)=>{const im=new Image();im.onload=()=>resolve(im);im.onerror=reject;im.src='SOURCE/'+row.image});const canvas=document.createElement('canvas');canvas.width=manifest.width;canvas.height=manifest.height;const ctx=canvas.getContext('2d',{willReadFrequently:true});ctx.drawImage(image,0,0);const expected=ctx.getImageData(0,0,canvas.width,canvas.height).data;ctx.clearRect(0,0,canvas.width,canvas.height);ctx.drawImage(renderer.domElement,0,0);const actual=ctx.getImageData(0,0,canvas.width,canvas.height).data;let alphaMismatch=0,maxRgbError=0,visiblePixels=0;for(let p=0;p<actual.length;p+=4){if(actual[p+3]!==expected[p+3])alphaMismatch++;if(expected[p+3]){visiblePixels++;for(let c=0;c<3;c++)maxRgbError=Math.max(maxRgbError,Math.abs(actual[p+c]-expected[p+c]));}}if(alphaMismatch||maxRgbError>1)throw Error('Native pixel mismatch '+JSON.stringify({i,alphaMismatch,maxRgbError}));samples.push({phase:i,tick:row.first_tick,visiblePixels,alphaMismatch,maxRgbError});}show(manifest.terminal_tick+25);const active=gltf.scene.children.filter(o=>o.scale.x>.5);if(active.length!==1||active[0].userData.native_first_tick!==manifest.source_bindings.at(-1).first_tick)throw Error('Terminal did not clamp');show(0);return{status:'PASS',samples,terminalClamped:true,channels:gltf.animations[0].tracks.length,representation:'planar native appearance, not recovered 3D motion'};}};
}catch(error){window.nativeAppearance={loaded:false,error:String(error)};document.querySelector('#status').textContent=String(error)}
</script>'''


def main():
    root=Path(__file__).resolve().parents[3]
    for assembly in ['log-trap','rock-trap']:
        dest=OUT/'restart2-state'/f'{assembly}-native-appearance-v1';m=json.loads((dest/'manifest.json').read_text())
        html=TEMPLATE.replace('ASSEMBLY',assembly).replace('TERMINAL',str(m['terminal_tick'])).replace('THREE/','/@fs/'+str(root/'level-editor/app/node_modules/three')+'/').replace('SOURCE','/@fs/'+str(OUT/'state-target-evidence'/assembly/'full-motion'))
        (dest/'index.html').write_text(html)


if __name__=='__main__':main()
