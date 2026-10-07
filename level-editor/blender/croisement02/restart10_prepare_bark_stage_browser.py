"""Prepare the production Editor proof against a private, exact library overlay."""
import argparse, hashlib, json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/croisement02-refinement/restart2-textures'
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', type=Path)
    parser.add_argument('--fixture-name', default='editor-review')
    args = parser.parse_args()
    stage = args.stage.resolve()
    candidate = json.loads((stage / 'candidate.json').read_text())
    assert not candidate['inherited_derivative_holds']
    assert '/' not in args.fixture_name and args.fixture_name.startswith('editor-review')
    out = stage / args.fixture_name
    assert not out.exists()
    out.mkdir()
    template = WORK / 'installed-static-v2'
    (out / 'editor-review.html').write_bytes((template / 'editor-review.html').read_bytes())
    fixture = (template / 'editor-review.tsx').read_text()
    fixture += TERRAIN_PROOF
    (out / 'editor-review.tsx').write_text(fixture)
    runner = (WORK / 'review_installed_static_v2.mjs').read_text()
    runner = runner.replace("'../../../app/tests/cdp.mjs'", repr(str(ROOT / 'level-editor/app/tests/cdp.mjs')))
    runner = runner.replace("'../../../app/node_modules/vite/dist/node/index.js'", repr(str(ROOT / 'level-editor/app/node_modules/vite/dist/node/index.js')))
    runner = runner.replace('level-editor/work/croisement02-refinement/restart2-textures/installed-static-v2', str(out))
    runner = runner.replace("import {spawn} from 'node:child_process';", "import {spawn} from 'node:child_process';import {createHash} from 'node:crypto';import {createReadStream} from 'node:fs';import {stat} from 'node:fs/promises';")
    middleware = """plugins:[{name:'exact-private-library-overlay',enforce:'pre',configureServer(server){server.middlewares.use(async(req,res,next)=>{try{const pathname=new URL(req.url,'http://localhost').pathname;if(!pathname.startsWith('/library/'))return next();const relative=decodeURIComponent(pathname.slice(9));if(relative.split('/').includes('..'))throw Error('Invalid library path');let candidate;if(relative==='scenes/croisement02.rhlos-map.json')candidate=STAGE+'/croisement02.rhlos-map.json';else if(relative==='3d-assets/index.json')candidate=STAGE+'/promotion-library-index.json';else candidate=STAGE+'/map-assets/'+relative;let info;try{info=await stat(candidate)}catch{return next()}if(!info.isFile())return next();await fs.appendFile(path.join(out,'served-overlay.jsonl'),JSON.stringify({request:relative,source:candidate})+'\\n');res.setHeader('Content-Type',candidate.endsWith('.json')?'application/json':candidate.endsWith('.glb')?'model/gltf-binary':'application/octet-stream');res.setHeader('Content-Length',info.size);res.setHeader('Cache-Control','no-store');createReadStream(candidate).pipe(res);}catch(error){next(error)}})}}],""".replace('STAGE', json.dumps(str(stage)))
    runner = runner.replace('createServer({configFile:', 'createServer({' + middleware + 'configFile:')
    preflight = f"""const routeChecks={{'scenes/croisement02.rhlos-map.json':{json.dumps(candidate['map_sha256'])},'3d-assets/croisement02/croisement02-terrain/preview.glb':{json.dumps(candidate['terrain_preview_repair']['output_sha256'])}}};for(const [resource,expected]of Object.entries(routeChecks)){{const response=await fetch(origin+'/library/'+resource);if(!response.ok)throw Error('Overlay preflight HTTP '+response.status);const actual=createHash('sha256').update(Buffer.from(await response.arrayBuffer())).digest('hex');if(actual!==expected)throw Error('Overlay preflight SHA mismatch '+resource);}}await fs.writeFile(path.join(out,'overlay-preflight.json'),JSON.stringify(routeChecks,null,2));"""
    runner = runner.replace("const url=origin+", preflight + "\nconst url=origin+")

    marker = " await fs.writeFile(path.join(out,'loaded.png')"
    captures = """ const terrainReply=await call('Runtime.evaluate',{expression:'window.captureTerrainPreviewProof()',returnByValue:true,awaitPromise:true});if(terrainReply.exceptionDetails)throw Error(JSON.stringify(terrainReply.exceptionDetails));const terrain=terrainReply.result.value;for(const [name,data]of Object.entries(terrain.images))await fs.writeFile(path.join(out,name+'.png'),Buffer.from(data.split(',')[1],'base64'));delete terrain.images;await fs.writeFile(path.join(out,'terrain-preview-proof.json'),JSON.stringify(terrain,null,2));
"""
    assert marker in runner
    runner = runner.replace(marker, captures + marker)
    runner = runner.replace('installed normal HTTP library, without staged resource routing', 'private staged library overlay with current production runtime')
    (out / 'runner.mjs').write_text(runner)
    print(out)

TERRAIN_PROOF = r'''
import * as THREE from "three";
import {createGltfLoader} from "/@fs/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/level-editor/app/src/gltf-loader.ts";
import {TextureDisplay} from "/@fs/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/level-editor/app/src/texture-display.ts";
Object.assign(window,{captureTerrainPreviewProof:async()=>{
 const renderer=new THREE.WebGLRenderer({antialias:true,preserveDrawingBuffer:true});renderer.setSize(896,576);renderer.setClearColor(0x303030,1);renderer.outputColorSpace=THREE.SRGBColorSpace;renderer.toneMapping=THREE.NoToneMapping;
 const images={},models={};let camera;
 for(const [name,file] of [['terrain-preview','preview.glb'],['terrain-lossy','lossy.glb']]){
  const bytes=await(await fetch('/library/3d-assets/croisement02/croisement02-terrain/'+file)).arrayBuffer();const gltf=await createGltfLoader().parseAsync(bytes,'');const scene=new THREE.Scene();scene.add(gltf.scene);scene.updateMatrixWorld(true);let meshes=0,triangles=0;scene.traverse(o=>{if(o.isMesh){meshes++;triangles+=(o.geometry.index?.count??o.geometry.attributes.position.count)/3}});
  const box=new THREE.Box3().setFromObject(scene),center=box.getCenter(new THREE.Vector3());if(!camera){const scale=box.getSize(new THREE.Vector3()).length()*1.08;camera=new THREE.OrthographicCamera(-scale/2,scale/2,scale*576/896/2,-scale*576/896/2,.01,scale*10);camera.position.copy(center).add(new THREE.Vector3(0,-Math.cos(35*Math.PI/180),Math.sin(35*Math.PI/180)).multiplyScalar(scale*3));camera.up.set(0,Math.sin(35*Math.PI/180),Math.cos(35*Math.PI/180));camera.lookAt(center);camera.updateMatrixWorld(true)}new TextureDisplay().apply(scene,renderer.capabilities.getMaxAnisotropy());renderer.render(scene,camera);images[name]=renderer.domElement.toDataURL('image/png');models[name]={meshes,triangles,bounds:[box.min.toArray(),box.max.toArray()],bytes:bytes.byteLength,sha256:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('')};scene.traverse(o=>{if(o.isMesh){o.geometry.dispose();for(const m of(Array.isArray(o.material)?o.material:[o.material]))m.dispose()}});
 }
 renderer.dispose();return{status:'PASS both exact terrain models loaded through production createGltfLoader and TextureDisplay',images,models,nativeDirectionFirst:true};
}});
'''
if __name__ == '__main__':
    main()
