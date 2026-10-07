import * as THREE from "three";
import { decodeMaskAlphaCoverage } from "../../shared/src/mask-alpha-sampler.ts";

/** Independent GPU coverage rendering for the authored mask, before native encoding. */
export function maskReviewMesh(mask, convert) {
  const coverage = mask.alphaCoverage;
  if (coverage) decodeMaskAlphaCoverage(coverage, mask.triangles.length);
  const textures =
    coverage?.textures.map((source) => {
      const bytes = Uint8Array.from(atob(source.alphaBase64), (c) => c.charCodeAt(0));
      const texture = new THREE.DataTexture(bytes, source.width, source.height, THREE.RedFormat);
      texture.magFilter = THREE.NearestFilter;
      texture.minFilter = THREE.NearestFilter;
      texture.generateMipmaps = false;
      texture.unpackAlignment = 1;
      texture.needsUpdate = true;
      return texture;
    }) ?? [];
  const groups = new Map();
  for (const [index, triangle] of mask.triangles.entries()) {
    const rule = coverage?.triangles[index];
    const key = JSON.stringify(
      rule ? [rule.texture, rule.cutoff, rule.wrap, rule.doubleSided] : [],
    );
    let group = groups.get(key);
    if (!group) {
      group = { rule, positions: [], uv: [], alpha: [] };
      groups.set(key, group);
    }
    for (let vertex = 0; vertex < 3; vertex++) {
      group.positions.push(...convert(triangle[vertex]));
      group.uv.push(...(rule?.uv[vertex] ?? [0, 0]));
      group.alpha.push(rule?.alpha[vertex] ?? 1);
    }
  }
  const root = new THREE.Group();
  const wrap = (mode) => (mode === "clamp" ? 0 : mode === "repeat" ? 1 : 2);
  for (const { rule, positions, uv, alpha } of groups.values()) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
    geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uv, 2));
    geometry.setAttribute("coverageAlpha", new THREE.Float32BufferAttribute(alpha, 1));
    const material = new THREE.ShaderMaterial({
      side: mask.cullBackfaces && !rule?.doubleSided ? THREE.FrontSide : THREE.DoubleSide,
      uniforms: {
        alphaTexture: { value: rule?.texture === undefined ? null : textures[rule.texture] },
        hasTexture: { value: rule?.texture !== undefined },
        cutoff: { value: rule?.cutoff ?? 0 },
        wrapMode: { value: new THREE.Vector2(...(rule?.wrap.map(wrap) ?? [0, 0])) },
      },
      vertexShader: `
        attribute float coverageAlpha;
        varying vec2 maskUv;
        varying float maskAlpha;
        void main() {
          maskUv = uv;
          maskAlpha = coverageAlpha;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }`,
      fragmentShader: `
        uniform sampler2D alphaTexture;
        uniform bool hasTexture;
        uniform float cutoff;
        uniform vec2 wrapMode;
        varying vec2 maskUv;
        varying float maskAlpha;
        float wrapped(float value, float mode) {
          if (mode < 0.5) return clamp(value, 0.0, 1.0);
          if (mode < 1.5) return fract(value);
          float repeatValue = mod(value, 2.0);
          return repeatValue <= 1.0 ? repeatValue : 2.0 - repeatValue;
        }
        void main() {
          float alpha = maskAlpha;
          if (hasTexture) alpha *= texture2D(alphaTexture, vec2(
            wrapped(maskUv.x, wrapMode.x), wrapped(maskUv.y, wrapMode.y))).r;
          if (alpha < cutoff) discard;
          gl_FragColor = vec4(1.0);
        }`,
    });
    root.add(new THREE.Mesh(geometry, material));
  }
  return {
    root,
    dispose() {
      for (const mesh of root.children) {
        mesh.geometry.dispose();
        mesh.material.dispose();
      }
      for (const texture of textures) texture.dispose();
    },
  };
}
