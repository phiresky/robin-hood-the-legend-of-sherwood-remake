const cross = (a,b,p)=>(b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]);
const area = poly=>poly.reduce((sum,a,i)=>{const b=poly[(i+1)%poly.length];return sum+a[0]*b[1]-b[0]*a[1];},0)/2;
function clip(poly,a,b,sign=1) {
  const out=[];
  for(let i=0;i<poly.length;i++) {
    const p=poly[i],q=poly[(i+1)%poly.length];
    const dp=sign*cross(a,b,p),dq=sign*cross(a,b,q);
    if(dp>=0) out.push(p);
    if((dp>=0)!==(dq>=0)) {
      const t=dp/(dp-dq);
      out.push(p.map((v,k)=>v+t*(q[k]-v)));
    }
  }
  return out;
}
function intersection(subject,boundary) {
  let result=subject;
  const sign=Math.sign(area(boundary));
  for(let i=0;i<boundary.length && result.length;i++)
    result=clip(result,boundary[i],boundary[(i+1)%boundary.length],sign);
  const same=(a,b)=>a.every((value,i)=>Math.abs(value-b[i])<=1e-10);
  result=result.filter((point,i)=>i===0||!same(point,result[i-1]));
  if(result.length>1&&same(result[0],result[result.length-1]))result.pop();
  return result;
}

/** Private support projection. Receiver vertices are [worldX, worldY-height, height]. */
export function projectShadowReceivers({anchor,bounds,elevation,triangles}) {
  if(!Array.isArray(anchor)||anchor.length!==3||!anchor.every(Number.isFinite)||
      !Number.isFinite(elevation)||elevation<=0||elevation>=Math.PI/2)
    throw Error('Invalid projection anchor or elevation');
  const {left,top,width,height}=bounds??{};
  if(![left,top,width,height].every(Number.isFinite)||width<=0||height<=0)
    throw Error('Invalid frame bounds');
  if(!Array.isArray(triangles)||!triangles.length) throw Error('Missing support triangles');
  const sin=Math.sin(elevation),cos=Math.cos(elevation),baseZ=anchor[1]*cos;
  const mapY=anchor[2]*sin-baseZ;
  const x0=anchor[0]+left,x1=x0+width,y0=mapY-top,y1=y0+height;
  const footprint=[[x0,y0],[x1,y0],[x1,y1],[x0,y1]];
  const expectedArea=width*height,tolerance=Math.max(1,expectedArea)*1e-8;
  if(![x0,x1,y0,y1,expectedArea].every(Number.isFinite))throw Error('Projection extent overflow');
  const pieces=[];
  for(const [receiver,triangle]of triangles.entries()) {
    if(!Array.isArray(triangle)||triangle.length!==3||triangle.some(p=>!Array.isArray(p)||p.length!==3||!p.every(Number.isFinite)))
      throw Error(`Invalid receiver triangle ${receiver}`);
    if(Math.abs(area(triangle))<=Number.EPSILON)
      throw Error(`Vertical or degenerate receiver triangle ${receiver}`);
    const polygon=intersection(triangle,footprint);
    const covered=Math.abs(area(polygon));
    if(covered>tolerance) pieces.push({receiver,polygon,area:covered});
  }
  for(let i=0;i<pieces.length;i++)for(let j=0;j<i;j++) {
    if(Math.abs(area(intersection(pieces[i].polygon,pieces[j].polygon)))>tolerance)
      throw Error(`Overlapping support receivers ${pieces[j].receiver}/${pieces[i].receiver}`);
  }
  const coveredArea=pieces.reduce((sum,p)=>sum+p.area,0);
  if(Math.abs(coveredArea-expectedArea)>tolerance)
    throw Error(`Incomplete support coverage: ${coveredArea}/${expectedArea}`);
  const positions=[],uvs=[],indices=[],ranges=[];
  for(const piece of pieces) {
    const firstVertex=positions.length/3,firstIndex=indices.length;
    for(const [x,y,z]of piece.polygon) {
      const localX=x-anchor[0],up=mapY-y,dz=z-baseZ;
      positions.push(localX,dz/cos+0.15,(dz-up)/sin);
      uvs.push((localX-left)/width,(up-(top-height))/height);
    }
    for(let i=1;i+1<piece.polygon.length;i++)indices.push(firstVertex,firstVertex+i,firstVertex+i+1);
    // Each receiver owns separate vertices. A discontinuous height edge is never
    // welded and no generated face connects the high and low receiver surfaces.
    ranges.push({receiver:piece.receiver,firstVertex,vertexCount:piece.polygon.length,
      firstIndex,indexCount:indices.length-firstIndex});
  }
  return {positions,uvs,indices,ranges,coveredArea,expectedArea};
}
