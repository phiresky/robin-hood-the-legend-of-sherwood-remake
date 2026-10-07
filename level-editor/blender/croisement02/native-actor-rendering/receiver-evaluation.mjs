/** Evaluate explicitly selected physical receiver meshes in the current scene.
 * The caller owns logical receiver membership; visual/contact overlays are not inferred. */
export function evaluatePhysicalReceiver(THREE, mesh, owner, elevation) {
  if (!mesh?.isMesh || mesh.isSkinnedMesh || mesh.isInstancedMesh ||
      !owner || typeof owner !== 'string' || !Number.isFinite(elevation) ||
      elevation <= 0 || elevation >= Math.PI/2)
    throw Error('An explicit static physical receiver is required');
  const positions=mesh.geometry?.getAttribute('position'), index=mesh.geometry?.index;
  if (!positions || positions.itemSize!==3 || mesh.geometry.morphAttributes.position?.length)
    throw Error('Receiver needs evaluated static triangle positions');
  const count=index?.count ?? positions.count;
  if(count%3)throw Error('Receiver is not a triangle mesh');
  mesh.updateWorldMatrix(true,false);
  if (!mesh.matrixWorld.elements.every(Number.isFinite) || Math.abs(mesh.matrixWorld.determinant())<1e-12)
    throw Error('Invalid receiver world transform');
  const sin=Math.sin(elevation),cos=Math.cos(elevation),triangles=[];
  for(let i=0;i<count;i+=3){
    const points=[0,1,2].map(k=>{
      const vertex=index?index.getX(i+k):i+k;
      if(!Number.isSafeInteger(vertex)||vertex<0||vertex>=positions.count)throw Error('Invalid receiver index');
      const p=new THREE.Vector3().fromBufferAttribute(positions,vertex).applyMatrix4(mesh.matrixWorld);
      if(!p.toArray().every(Number.isFinite))throw Error('Invalid receiver vertex');return p;
    });
    const normal=new THREE.Vector3().subVectors(points[1],points[0]).cross(new THREE.Vector3().subVectors(points[2],points[0]));
    const length=normal.length();
    // A relative tolerance excludes nominally vertical sides despite float32
    // export transforms, while preserving any meaningful upward-facing slope.
    if(length===0||normal.y/length<=1e-5)continue;
    triangles.push({id:`${owner}:triangle-${i/3}`,points:points.map(p=>[p.x,p.z*sin,p.y*cos])});
  }
  if(!triangles.length)throw Error(`Receiver ${owner} has no upward physical triangles`);
  return triangles;
}
