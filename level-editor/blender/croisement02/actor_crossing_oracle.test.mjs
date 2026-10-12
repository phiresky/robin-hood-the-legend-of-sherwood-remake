import test from 'node:test';
import assert from 'node:assert/strict';
import {actorCrossingOracle,opaqueBodyOverlap} from './actor_crossing_oracle.mjs';
const pixel=rgba=>({width:1,height:1,data:new Uint8Array(rgba)});
function fixture(){
 const source={camera:{elevation_deg:35},level:{animations:[{sprite:{elevation:0}}],patches:[],masks:[]},data:{mission_patches:[],civilians:[],pcs_to_rescue:[],soldiers:[{}],targets:[]}};
 const element={source:{kind:'map-animation',index:0},display_order:9999,sort_position:[1,1],polyline:[]};
 const loop={origin:[0,0],background:{width:64,height:64,data:new Uint8Array(64*64*4)},draws:[{element,pixels:pixel([0,255,0,255]),x:1,y:1}]};
 const sprite={position:[1,0,1/Math.sin(35*Math.PI/180)],bounds:{left:0,top:0,width:1,height:1},frame:{pixels:pixel([255,0,0,255])}};
 return{source,loop,sprite,binding:{identity:'soldiers:0',layer:0,outlineColor:0xf800,drawHidden:false}};
}
test('background effect precedes an overlapping actor regardless of scalar order',()=>{
 const f=fixture(),r=actorCrossingOracle(f.source,f.loop,f.sprite,f.binding),i=(64+1)*4;
 assert.deepEqual(r.receipt.identities,['map-animation:0','soldiers:0']);
 assert.equal(opaqueBodyOverlap(r.actor,r.ordered[0]),1);
 assert.deepEqual([...r.pixels.data.slice(i,i+4)],[255,0,0,255]);
 assert.deepEqual([...r.compose([...r.ordered].reverse()).data.slice(i,i+4)],[0,255,0,255]);
});
test('authored display polyline, rather than an effect anchor order, controls foreground crossing',()=>{
 const f=fixture();f.source.level.animations[0].sprite.elevation=10;
 f.loop.draws[0].element.polyline=[[0,2],[10,2]];f.loop.draws[0].element.display_order=-9999;
 let r=actorCrossingOracle(f.source,f.loop,f.sprite,f.binding);
 assert.deepEqual(r.receipt.identities,['soldiers:0','map-animation:0']);
 f.sprite.position[2]=3/Math.sin(35*Math.PI/180);
 r=actorCrossingOracle(f.source,f.loop,f.sprite,f.binding);
 assert.deepEqual(r.receipt.identities,['map-animation:0','soldiers:0']);
});
