import * as THREE from "three";
import type { NativePixels } from "./native-state-presentation.ts";
import type { NativeShadowKey } from "../../shared/src/native-state-presentation.ts";

// TODO: Connect to ordered viewport passes only after byte-exact GPU and dynamic-entity proofs.
/** Byte-space compositor for native artwork. Callers own frame selection and draw order. */
export class NativePixelCompositor {
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.Camera();
  private readonly geometry = new THREE.PlaneGeometry(2, 2);
  private readonly material = new THREE.ShaderMaterial({
    glslVersion: THREE.GLSL3,
    uniforms: {
      destination: { value: null },
      source: { value: null },
      sourceOrigin: { value: new THREE.Vector2() },
      sourceSize: { value: new THREE.Vector2() },
      keyed: { value: false },
      keyColor: { value: new THREE.Vector3() },
      factor: { value: 256 },
      greenBits: { value: 6 },
    },
    vertexShader: "void main(){gl_Position=vec4(position.xy,0.,1.);}",
    fragmentShader: `uniform sampler2D destination, source;
      uniform vec2 sourceOrigin, sourceSize;
      uniform bool keyed;
      uniform vec3 keyColor;
      uniform int factor, greenBits;
      out vec4 result;
      void main(){
        ivec2 p=ivec2(gl_FragCoord.xy), q=p-ivec2(sourceOrigin);
        ivec4 d=ivec4(floor(texelFetch(destination,p,0)*255.+.5));
        if(any(lessThan(q,ivec2(0)))||any(greaterThanEqual(q,ivec2(sourceSize)))){result=vec4(d)/255.;return;}
        ivec4 s=ivec4(floor(texelFetch(source,q,0)*255.+.5));
        if(s.a==0){result=vec4(d)/255.;return;}
        if(keyed&&all(equal(s.rgb,ivec3(keyColor)))){
          for(int c=0;c<3;c++){
            int bits=c==1?greenBits:5;
            int value=((d[c]>>(8-bits))*factor)>>8;
            d[c]=(value<<(8-bits))|(value>>(2*bits-8));
          }
          result=vec4(d)/255.;return;
        }
        if(s.a==255){result=vec4(s)/255.;return;}
        float remaining=float(d.a*(255-s.a))/255.;
        float combined=float(s.a)+remaining;
        result=vec4(floor((vec3(s.rgb)*float(s.a)+vec3(d.rgb)*remaining)/combined+.5),floor(combined+.5))/255.;
      }`,
    depthTest: false,
    depthWrite: false,
    blending: THREE.NoBlending,
    toneMapped: false,
  });
  private disposed = false;
  constructor() {
    const quad = new THREE.Mesh(this.geometry, this.material);
    quad.frustumCulled = false;
    this.scene.add(quad);
  }
  /** Raw top-to-bottom byte rows; no sRGB conversion, filtering, premultiplication or scene-depth mutation. */
  compose(
    renderer: THREE.WebGLRenderer,
    background: NativePixels,
    draws: readonly { pixels: NativePixels; x: number; y: number; shadow?: NativeShadowKey }[],
  ): NativePixels {
    if (this.disposed) throw new Error("Native compositor is disposed");
    const validate = (p: NativePixels) => {
      if (
        !Number.isSafeInteger(p.width) ||
        !Number.isSafeInteger(p.height) ||
        p.width < 1 ||
        p.height < 1 ||
        p.data.length !== p.width * p.height * 4
      )
        throw new Error("Invalid native pixels");
    };
    validate(background);
    for (const draw of draws) {
      validate(draw.pixels);
      if (!Number.isFinite(draw.x) || !Number.isFinite(draw.y))
        throw new Error("Invalid native origin");
      if (draw.shadow) {
        const s = draw.shadow;
        if (
          !["rgb555", "rgb565"].includes(s.pixel_format) ||
          !Number.isInteger(s.strength_percent) ||
          s.strength_percent < 0 ||
          s.strength_percent > 100 ||
          s.rgb.length !== 3 ||
          s.rgb.some((v) => !Number.isInteger(v) || v < 0 || v > 255)
        )
          throw new Error("Invalid native shadow key");
        for (let i = 0; i < draw.pixels.data.length; i += 4)
          if (
            s.rgb.every((v, c) => draw.pixels.data[i + c] === v) &&
            ![0, 255].includes(draw.pixels.data[i + 3]!)
          )
            throw new Error("Native shadow key must have binary alpha");
      }
    }
    if (renderer.xr.enabled)
      throw new Error("Native pixel compositor requires a non-XR render pass");
    const cubeFace = renderer.getActiveCubeFace(),
      mipLevel = renderer.getActiveMipmapLevel();
    const target = renderer.getRenderTarget(),
      viewport = renderer.getViewport(new THREE.Vector4()),
      scissor = renderer.getScissor(new THREE.Vector4()),
      scissorTest = renderer.getScissorTest(),
      autoClear = renderer.autoClear;
    const targets: THREE.WebGLRenderTarget[] = [],
      textures: THREE.DataTexture[] = [];
    try {
      const upload = (p: NativePixels) => {
        const t = new THREE.DataTexture(p.data, p.width, p.height, THREE.RGBAFormat);
        textures.push(t);
        t.flipY = false;
        t.colorSpace = THREE.NoColorSpace;
        t.generateMipmaps = false;
        t.minFilter = t.magFilter = THREE.NearestFilter;
        t.needsUpdate = true;
        return t;
      };
      const a = new THREE.WebGLRenderTarget(background.width, background.height, {
        depthBuffer: false,
        stencilBuffer: false,
        minFilter: THREE.NearestFilter,
        magFilter: THREE.NearestFilter,
      });
      targets.push(a);
      const b = a.clone();
      targets.push(b);
      let previous: THREE.Texture = upload(background),
        output = a;
      // A transparent draw also copies an empty sequence into the output target.
      const sequence = draws.length
        ? draws
        : [{ pixels: { width: 1, height: 1, data: new Uint8Array(4) }, x: 0, y: 0 }];
      renderer.autoClear = false;
      renderer.setScissorTest(false);
      for (const draw of sequence) {
        const u = this.material.uniforms;
        u.destination!.value = previous;
        u.source!.value = upload(draw.pixels);
        u.sourceOrigin!.value.set(Math.floor(draw.x), Math.floor(draw.y));
        u.sourceSize!.value.set(draw.pixels.width, draw.pixels.height);
        u.keyed!.value = !!draw.shadow;
        u.keyColor!.value.fromArray(draw.shadow?.rgb ?? [0, 0, 0]);
        u.factor!.value = Math.floor(((100 - (draw.shadow?.strength_percent ?? 0)) * 256) / 100);
        u.greenBits!.value = draw.shadow?.pixel_format === "rgb555" ? 5 : 6;
        renderer.setRenderTarget(output);
        renderer.setViewport(0, 0, background.width, background.height);
        renderer.render(this.scene, this.camera);
        previous = output.texture;
        output = output === a ? b : a;
      }
      const data = new Uint8Array(background.data.length);
      renderer.readRenderTargetPixels(
        output === a ? b : a,
        0,
        0,
        background.width,
        background.height,
        data,
      );
      return { width: background.width, height: background.height, data };
    } finally {
      this.material.uniforms.destination!.value = null;
      this.material.uniforms.source!.value = null;
      for (const texture of textures) texture.dispose();
      for (const t of targets) t.dispose();
      renderer.setRenderTarget(target, cubeFace, mipLevel);
      renderer.setViewport(viewport);
      renderer.setScissor(scissor);
      renderer.setScissorTest(scissorTest);
      renderer.autoClear = autoClear;
    }
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.geometry.dispose();
    this.material.dispose();
    this.scene.clear();
  }
}
