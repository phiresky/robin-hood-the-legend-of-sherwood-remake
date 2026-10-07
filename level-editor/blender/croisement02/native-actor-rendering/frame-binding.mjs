// Private physical-preview candidate. Frame resources remain owned by the loader.
export function createActorFrameBinding(THREE, body, identity) {
  if (!identity || typeof identity !== 'string' || !body.isMesh || Array.isArray(body.material))
    throw new Error('A stable identity and a single-material body mesh are required');
  let ownedShadow = null;
  let disposed = false;
  function release(shadow) {
    if (!shadow) return;
    shadow.removeFromParent();
    shadow.geometry.dispose();
    shadow.material.dispose();
  }
  return {
    get shadow() { return ownedShadow; },
    apply(snapshot) {
      if (disposed) throw new Error('Actor binding is disposed');
      const { frame, anchor, rotation, active, elevation, supportHeight, shadowStyle } = snapshot;
      if (snapshot.identity !== identity) throw new Error('Actor identity mismatch');
      if (typeof active !== 'boolean' || !Array.isArray(anchor) || anchor.length !== 3 ||
          !anchor.every(Number.isFinite) || !Number.isFinite(rotation))
        throw new Error('Explicit activity, anchor and rotation are required');
      if (frame !== null && (!frame?.geometry?.isBufferGeometry || !frame.texture?.isTexture))
        throw new Error('Missing validated body frame');
      let next = null;
      // Finish fallible support projection before changing either visible resource.
      if (active && frame?.shadow) {
        if (!frame.shadow.isTexture || !Number.isFinite(elevation) || elevation <= 0 || elevation >= Math.PI / 2 ||
            typeof supportHeight !== 'function' || !shadowStyle || !Number.isFinite(shadowStyle.opacity) ||
            shadowStyle.opacity < 0 || shadowStyle.opacity > 1 || !Number.isFinite(shadowStyle.color))
          throw new Error('Explicit projection and shadow style are required');
        const {left, top, width, height} = frame.bounds ?? {};
        if (![left, top, width, height].every(Number.isFinite) || width <= 0 || height <= 0)
          throw new Error('Invalid common frame bounds');
        const geometry = new THREE.PlaneGeometry(width, height);
        let material;
        try {
          const points = geometry.getAttribute('position');
          const sin = Math.sin(elevation), cos = Math.cos(elevation);
          const baseZ = anchor[1] * cos, mapY = anchor[2] * sin - baseZ;
          for (let i = 0; i < points.count; i++) {
            const x = points.getX(i) + left + width / 2;
            const up = points.getY(i) + top - height / 2;
            const z = supportHeight(anchor[0] + x, mapY - up);
            if (!Number.isFinite(z)) throw new Error('Invalid shadow support height');
            const dz = z - baseZ;
            points.setXYZ(i, x, dz / cos + 0.15, (dz - up) / sin);
          }
          geometry.computeBoundingSphere();
          material = new THREE.MeshBasicMaterial({ map: frame.shadow, color: shadowStyle.color,
            opacity: shadowStyle.opacity,
            transparent: true, depthWrite: false, side: THREE.DoubleSide,
            polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1 });
          next = new THREE.Mesh(geometry, material);
          next.name = 'current authored shadow';
          next.rotation.y = -rotation;
        } catch (error) {
          geometry.dispose();
          material?.dispose();
          throw error;
        }
      }
      const previous = ownedShadow;
      if (frame) { body.geometry = frame.geometry; body.material.map = frame.texture; }
      body.material.needsUpdate = true;
      body.position.fromArray(anchor);
      body.rotation.y = rotation;
      body.visible = active && frame !== null;
      ownedShadow = next;
      if (next) body.add(next);
      release(previous);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      release(ownedShadow);
      ownedShadow = null;
    },
  };
}
