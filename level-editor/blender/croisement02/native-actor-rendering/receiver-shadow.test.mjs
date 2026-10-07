import test from 'node:test';
import assert from 'node:assert/strict';
import {projectShadowReceivers} from './receiver-shadow.mjs';
const base={anchor:[0,0,0],bounds:{left:0,top:0,width:10,height:10},elevation:Math.PI/4};
const rectangle=(x0,y0,x1,y1,height)=>{
  const p=(x,y)=>[x,y,typeof height==='function'?height(x,y):height];
  return [[p(x0,y0),p(x1,y0),p(x1,y1)],[p(x0,y0),p(x1,y1),p(x0,y1)]];
};
const near=(a,b)=>assert.ok(Math.abs(a-b)<1e-8,`${a} != ${b}`);
test('sloped receivers clip to exact frame footprint with matching UVs and heights',()=>{
  const got=projectShadowReceivers({...base,triangles:rectangle(-5,-5,15,15,(x,y)=>2*x+3*y)});
  near(got.coveredArea,100);
  for(let i=0;i<got.positions.length/3;i++) {
    const x=got.positions[i*3],u=got.uvs[i*2],v=got.uvs[i*2+1],y=10*(1-v);
    near(u,x/10);assert.ok(u>=0&&u<=1&&v>=0&&v<=1);
    near(got.positions[i*3+1],(2*x+3*y)/Math.cos(base.elevation)+0.15);
    near(got.positions[i*3+2],(2*x+4*y)/Math.sin(base.elevation));
  }
});
test('interior peak and breaklines survive; the four-corner plane limitation is removed',()=>{
  const c=[5,5,8],corners=[[0,0,0],[10,0,0],[10,10,0],[0,10,0]];
  const got=projectShadowReceivers({...base,triangles:corners.map((p,i)=>[p,corners[(i+1)%4],c])});
  assert.equal(got.ranges.length,4);
  const peaks=[];
  for(let i=0;i<got.positions.length/3;i++)if(got.uvs[i*2]===0.5&&got.uvs[i*2+1]===0.5)peaks.push(got.positions[i*3+1]);
  assert.equal(peaks.length,4);for(const y of peaks)near(y,8/Math.cos(base.elevation)+0.15);
});
test('raised receiver edge remains disconnected with no invented vertical or ramp faces',()=>{
  const got=projectShadowReceivers({...base,triangles:[...rectangle(0,0,5,10,0),...rectangle(5,0,10,10,7)]});
  const heights=new Set();
  for(let i=0;i<got.indices.length;i+=3) {
    const ys=got.indices.slice(i,i+3).map(n=>got.positions[n*3+1]);
    near(ys[0],ys[1]);near(ys[1],ys[2]);heights.add(ys[0]);
  }
  assert.equal(heights.size,2);near(got.coveredArea,100);
});
test('holes and overlapping ambiguous receivers fail even when total areas cancel',()=>{
  assert.throws(()=>projectShadowReceivers({...base,triangles:rectangle(0,0,9,10,0)}),/Incomplete support/);
  const lower=rectangle(0,0,5,10,0);
  assert.throws(()=>projectShadowReceivers({...base,triangles:[...lower,...lower]}),/Overlapping/);
  assert.throws(()=>projectShadowReceivers({...base,triangles:[]}),/Missing support/);
  assert.throws(()=>projectShadowReceivers({...base,triangles:[[[0,0,0],[0,0,2],[0,10,2]]]}),/Vertical or degenerate/);
});
test('reversed winding and elevated nonzero anchor preserve native footprint coordinates',()=>{
  const elevation=Math.PI/6,anchor=[100,20,80],mapY=80*Math.sin(elevation)-20*Math.cos(elevation);
  const bounds={left:-2,top:7,width:12,height:9};
  const triangles=rectangle(98,mapY-7,110,mapY+2,(x,y)=>x/5+y/3).map(t=>t.toReversed());
  const got=projectShadowReceivers({anchor,bounds,elevation,triangles});
  near(got.coveredArea,108);
  for(let i=0;i<got.positions.length/3;i++) {
    const u=got.uvs[i*2],v=got.uvs[i*2+1],x=98+12*u,y=mapY+2-9*v;
    near(got.positions[i*3],x-100);
    near(got.positions[i*3+1],(x/5+y/3-20*Math.cos(elevation))/Math.cos(elevation)+0.15);
  }
  assert.equal(got.indices.length,6);
});
