"""Expose the local native-RGBA phase GLB in a standard Three.js viewer."""
import json
from catalog import ROOT,OUT
from build_tree41_animation_viewer import HTML

def main():
 root='/@fs/'+str(ROOT/'level-editor/app/node_modules/three')
 text=HTML.replace('"THREE"',json.dumps(root+'/build/three.module.js')).replace('"ADDONS"',json.dumps(root+'/examples/jsm/'))
 text=text.replace('Tree41 measured animation proof','Tree41 native phase appearance proof').replace('Tree41 measured 3D animation proof — HOLD for phase appearance','Tree41 native phase appearance — isolated candidate')
 text=text.replace('Existing crown mesh, measured native leaf motion, stationary wood and fixed source-ray depth. Phase0 geometry is unchanged. Native RGB/alpha changes remain incomplete; this is not final animation parity.', 'Unchanged full-depth crown geometry and stationary wood. Own native overlays composite over the painted static crown. Phase0 is unchanged. New temporal-edge ownership and production integration remain unapproved.')
 text=text.replace('animated-tree41.glb','phase-appearance-tree41.glb').replace('representative-phases.png','actual-phase-comparison.png').replace('temporal-coverage.json','proof.json').replace('Coverage hold','Source-alpha audit').replace('Loaded standard morph animation','Loaded standard phase appearance animation')
 text=text.replace("const bounds=new THREE.Box3().setFromObject(gltf.scene),center", "gltf.scene.updateMatrixWorld(true);const bounds=new THREE.Box3();gltf.scene.traverse(node=>{if(!node.isMesh)return;for(let parent=node;parent;parent=parent.parent){if(parent.scale.length()<.01)return}const positions=node.geometry.attributes.position;for(let i=0;i<positions.count;i++)bounds.expandByPoint(new THREE.Vector3().fromBufferAttribute(positions,i).applyMatrix4(node.matrixWorld))});const center")
 text=text.replace('window.animationProof={loaded:true,animations:gltf.animations.length,duration:clip.duration}',"window.animationProof={loaded:true,animations:gltf.animations.length,duration:clip.duration,bounds:{min:bounds.min.toArray(),max:bounds.max.toArray()},setTime:t=>{mixer.setTime(t);phase.value=Math.floor(t/.1)},weights:()=>{const values=[];gltf.scene.traverse(n=>{if(n.name.includes('Crown')&&n.type==='Group')values.push({name:n.name,scale:n.scale.toArray()})});return values},pause:()=>{playing=false;play.textContent='Play'}}")
 text=text.replace('position:absolute;z-index:2;max-width:700px','position:relative;z-index:2;max-width:none')
 text=text.replace('renderer.setSize(innerWidth,innerHeight);renderer.setPixelRatio',"const viewHeight=()=>Math.max(200,innerHeight-document.querySelector('header').getBoundingClientRect().height);renderer.setSize(innerWidth,viewHeight());renderer.setPixelRatio")
 text=text.replace('new THREE.PerspectiveCamera(40,innerWidth/innerHeight','new THREE.PerspectiveCamera(40,innerWidth/viewHeight()')
 text=text.replace('camera.aspect=innerWidth/innerHeight','camera.aspect=innerWidth/viewHeight()').replace('renderer.setSize(innerWidth,innerHeight)','renderer.setSize(innerWidth,viewHeight())')
 text=text.replace('30 native ticks/s', '25 native ticks/s; four ticks per phase').replace('*.1+', '*.16+').replace('/.1)', '/.16)').replace('t/.1', 't/.16')
 path=OUT/'tree41-phase-appearance-proof-v2/index.html';path.write_text(text);print(path)
if __name__=='__main__':main()
