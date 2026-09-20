/** Prepare and fill eight masked, shaded projections in one Images API request. */
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import sharp from "sharp";
import { requireEnv } from "./env.ts";

const model = "gpt-image-2.5-sunburst";
async function main(): Promise<void> {
  if (!process.argv[2]) throw new Error("Supply experiment directory and --prepare or --generate");
  const directory = path.resolve(process.argv[2]);
  const manifest = JSON.parse(await fs.readFile(path.join(directory,"views.json"),"utf8")) as {
    source_image:string;
    source_pixel_audit?:{output_xy:[number,number];source_xy:[number,number];source_rgb:number[]}[];
    layout:{width:number;height:number}; views:{input:string;mask:string;camera_matrix_world:number[][];ortho_scale:number;crop:{left:number;top:number;width:number;height:number}}[];
  };
  if(process.argv.includes("--audit-input")) {
    const view=manifest.views[0];if(!view)throw new Error("Missing source view");
    const frame=await sharp(path.join(directory,view.input)).ensureAlpha().raw().toBuffer();
    const editMask=await sharp(path.join(directory,view.mask)).ensureAlpha().raw().toBuffer();
    const {data:source,info}=await sharp(manifest.source_image).ensureAlpha().raw().toBuffer({resolveWithObject:true});
    const audit=manifest.source_pixel_audit??[];let incorrect=0;
    for(const entry of audit){
      const si=(entry.source_xy[1]*info.width+entry.source_xy[0])*4;
      const oi=(entry.output_xy[1]*view.crop.width+entry.output_xy[0])*4;
      for(let c=0;c<3;c++)if(source[si+c]!==frame[oi+c]||source[si+c]!==entry.source_rgb[c]){incorrect++;break;}
    }
    const radians=35*Math.PI/180;
    const cx=view.camera_matrix_world[0]![3]!;
    const cy=-view.camera_matrix_world[1]![3]!*Math.sin(radians)-view.camera_matrix_world[2]![3]!*Math.cos(radians);
    const crop=Buffer.alloc(view.crop.width*view.crop.height*4);
    let compared=0,mismatches=0;
    for(let y=0;y<view.crop.height;y++)for(let x=0;x<view.crop.width;x++){
      const sx=Math.floor(cx+(x+.5-view.crop.width/2)*view.ortho_scale/view.crop.height);
      const sy=Math.floor(cy+(y+.5-view.crop.height/2)*view.ortho_scale/view.crop.height);
      const i=(y*view.crop.width+x)*4;
      if(sx>=0&&sy>=0&&sx<info.width&&sy<info.height)source.copy(crop,i,(sy*info.width+sx)*4,(sy*info.width+sx)*4+4);
      else crop[i+3]=255;
      const background=frame[i]===0&&frame[i+1]===0&&frame[i+2]===0;
      if(editMask[i+3]!==0&&!background){compared++;if(!frame.subarray(i,i+3).equals(crop.subarray(i,i+3)))mismatches++;}
    }
    await sharp(crop,{raw:{width:view.crop.width,height:view.crop.height,channels:4}}).png().toFile(path.join(directory,"original-source-framing.png"));
    const report={auditedPixels:audit.length,incorrectSourceBytes:incorrect,knownComparedToOriginalFraming:compared,framingMismatches:mismatches};
    await fs.writeFile(path.join(directory,"source-pixel-verification.json"),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
    if(incorrect||mismatches)throw new Error("Known input pixels differ from original source bytes");
    return;
  }
  if (process.argv.includes("--prepare")) {
    for (const name of ["input","mask"] as const) {
      const layers = manifest.views.map(view=>({input:path.join(directory,view[name]),left:view.crop.left,top:view.crop.top}));
      await sharp({create:{width:manifest.layout.width,height:manifest.layout.height,channels:4,background:{r:0,g:0,b:0,alpha:0}}})
        .composite(layers).png().toFile(path.join(directory,`${name}.png`));
    }
    console.log(JSON.stringify({input:path.join(directory,"input.png"),mask:path.join(directory,"mask.png"),manifest:path.join(directory,"views.json")},null,2));
    return;
  }
  if (!process.argv.includes("--generate")) throw new Error("Choose --prepare or --generate");
  const input=await fs.readFile(path.join(directory,"input.png"));
  const approval=JSON.parse(await fs.readFile(path.join(directory,"approval.json"),"utf8")) as {
    status?:string; approved_by?:string; input_sha256?:string; geometry_revision?:string;
  };
  const inputHash=crypto.createHash("sha256").update(input).digest("hex");
  if(approval.status!=="approved"||approval.approved_by!=="user"||
     approval.input_sha256!==inputHash||!approval.geometry_revision)
    throw new Error("Sunburst requires explicit user approval for this exact preview and geometry revision");
  const mask=await fs.readFile(path.join(directory,"mask.png"));
  const lightingIndex=process.argv.indexOf("--lighting-reference");
  const lightingPath=lightingIndex<0?null:process.argv[lightingIndex+1];
  if(lightingIndex>=0&&!lightingPath)throw new Error("Supply the pure-gray lighting reference path");
  const lighting=lightingPath?await fs.readFile(path.resolve(lightingPath)):null;
  const suffixIndex=process.argv.indexOf("--prompt-suffix");
  const promptSuffix=suffixIndex<0?"":process.argv[suffixIndex+1];
  if(suffixIndex>=0&&(!promptSuffix||promptSuffix.startsWith("--")))
    throw new Error("Supply the additional material instructions after --prompt-suffix");
  if(lighting){
    const info=await sharp(lighting).metadata();
    if(info.width!==manifest.layout.width||info.height!==manifest.layout.height)
      throw new Error("Lighting reference dimensions differ from the approved input sheet");
  }
  const variantIndex=process.argv.indexOf("--prompt-variant");
  const variant=variantIndex<0?"detailed":process.argv[variantIndex+1];
  if(variant!=="short"&&variant!=="detailed"&&variant!=="restore")throw new Error("Choose --prompt-variant short, detailed, or restore");
  const prompts={
    restore:`Restore the attached image using masked inpainting. It contains eight different views of the same medieval stone gatehouse, arranged in two rows of four on a black background. The untextured gray shaded surfaces indicate missing textures; their shading shows the building's 3D structure.

Use the mask. Treat every pixel outside the mask as locked. Replace only the pixels inside the mask, preserving all existing pixels outside the mask EXACTLY, including their colors, textures, sharpness, and positions. Preserve the original canvas dimensions, black background, object placement, spacing, silhouettes, and camera angles. Do not regenerate the entire image.

Reconstruct the missing content by using the surviving textured portions of all eight views as references for the same building. The gatehouse has two cylindrical, weathered stone towers with reddish-brown tiled roofs with curved tapered profiles, connected by a roofed stone wall above an arched passage. Continue the existing masonry courses, roof shingles, architectural trim, narrow windows, weathering, and shadows naturally into the missing regions.

Maintain consistent architecture and materials across all eight views while respecting each view’s distinct perspective, visible surfaces, occlusion, and lighting. Infer hidden details conservatively from the other views. Preserve the arched passage and any genuine empty space. Match the original rendered-game-asset style and texture scale; do not introduce a new artistic style or additional architectural features.

At every mask boundary, make the reconstructed content meet the existing image without visible seams, untextured gray fringes, or abrupt changes in texture or shading. Do not blend, smooth, recolor, sharpen, or otherwise alter pixels outside the mask.

Replace every masked untextured surface with the appropriate texture. Return the completed eight-view sheet at its original dimensions as a lossless PNG, with no text, borders, or extra objects.`,
    short:"Create an image from the provided reference sheet of 8 views of the same asset. The untextured gray shaded areas mark missing textures. Use the mask. Fill in these regions logically and consistently across all views, preserving all existing pixels outside the mask exactly. Keep the same asset design, textures, lighting, perspective, and black background.",
    detailed:"This image is a fixed 4-column by 2-row contact sheet of EIGHT orthographic views of ONE identical medieval gatehouse, azimuths 0,45,90,135 degrees on the top row and 180,225,270,315 on the bottom. Untextured gray shaded surfaces show existing 3D geometry where texture is missing. Use the mask. Preserve every pixel outside the mask, including the existing textured artwork and black background, exactly. Texture ONLY the editable shaded surfaces in ALL EIGHT views TOGETHER, deriving consistent weathered grey-brown masonry, small rounded reddish-brown roof shingles, metal roof caps, lighting and fine painterly pixel grain from the known views. Use the shading to understand the surface shape and depth. Preserve every tile's exact camera, silhouette, geometry, roof peaks, eaves, arches, occlusion edges, dimensions and pixel locations. Do not rearrange, resize, merge, crop, flip, rotate, or relayout views. Do not transfer the front camera to another tile. The building and material pattern must remain consistent across all eight azimuths. Continue small stone/shingle courses at the exact original physical scale; no new windows, doors, people, objects, lettering or geometry. Retain all existing image boundaries."
  };
  const omitMask=process.argv.includes("--no-mask");
  if(omitMask&&variant!=="short")throw new Error("The no-mask control currently requires --prompt-variant short");
  const outputDirectory=path.join(directory,`generation-${variant}${omitMask?"-no-mask":""}${lighting?"-with-lighting":""}`);
  await fs.mkdir(outputDirectory,{recursive:true});
  const prompt=omitMask?"Create an image from the provided reference sheet of 8 views of the same asset. The untextured gray shaded areas mark missing textures. Fill in these regions logically and consistently across all views, preserving all existing textured pixels exactly. Keep the same asset design, textures, lighting, perspective, and black background.":prompts[variant];
  const parameters={model,quality:"high",size:"1536x1024",n:"1",output_format:"png",
    prompt:prompt+" Follow the lighting and shading shown on the gray surfaces, preserving the same sun direction across all eight views."+
      (lighting?" The second image shows the same eight views entirely in gray; use it as the reference for lighting, shadows, and shape, and return only the completed first image.":"")+
      (promptSuffix?" "+promptSuffix:"")};
  const hash=crypto.createHash("sha256").update(input).update(lighting??Buffer.alloc(0)).update(omitMask?Buffer.alloc(0):mask).update(JSON.stringify(parameters)).digest("hex");
  const cache=path.join(directory,"api-cache",hash);await fs.mkdir(cache,{recursive:true});
  await fs.writeFile(path.join(cache,"request.json"),JSON.stringify({endpoint:"https://api.openai.com/v1/images/edits",parameters,input_sha256:crypto.createHash("sha256").update(input).digest("hex"),lighting_sha256:lighting?crypto.createHash("sha256").update(lighting).digest("hex"):null,mask_sha256:omitMask?null:crypto.createHash("sha256").update(mask).digest("hex")},null,2));
  await fs.writeFile(path.join(cache,"input.png"),input);await fs.writeFile(path.join(cache,"mask.png"),mask);
  if(lighting)await fs.writeFile(path.join(cache,"lighting.png"),lighting);
  let response:{status:number;body:unknown};
  try { response=JSON.parse(await fs.readFile(path.join(cache,"response.json"),"utf8")); }
  catch(error) {
    if((error as NodeJS.ErrnoException).code!=="ENOENT")throw error;
    const form=new FormData();for(const [key,value]of Object.entries(parameters))form.append(key,value);
    form.append(lighting?"image[]":"image",new Blob([new Uint8Array(input)],{type:"image/png"}),"input.png");
    if(lighting)form.append("image[]",new Blob([new Uint8Array(lighting)],{type:"image/png"}),"lighting.png");
    if(!omitMask)form.append("mask",new Blob([new Uint8Array(mask)],{type:"image/png"}),"mask.png");
    const raw=await fetch("https://api.openai.com/v1/images/edits",{method:"POST",headers:{Authorization:`Bearer ${requireEnv("OPENAI_API_KEY")}`},body:form});
    const text=await raw.text();let body:unknown;try{body=JSON.parse(text);}catch{body={text};}
    response={status:raw.status,body};await fs.writeFile(path.join(cache,"response.json"),JSON.stringify(response,null,2));
  }
  if(response.status<200||response.status>=300)throw new Error(JSON.stringify(response));
  const encoded=(response.body as {data?:{b64_json?:string}[]}).data?.[0]?.b64_json;
  if(!encoded)throw new Error("No generated image in response");
  const generated=Buffer.from(encoded,"base64");await fs.writeFile(path.join(outputDirectory,"generated-raw.png"),generated);
  const original=await sharp(input).ensureAlpha().raw().toBuffer();
  const editMask=await sharp(mask).ensureAlpha().raw().toBuffer();
  const generatedInfo=await sharp(generated).metadata();
  if(generatedInfo.width!==manifest.layout.width||generatedInfo.height!==manifest.layout.height)
    throw new Error(`Generated dimensions ${generatedInfo.width}x${generatedInfo.height} differ from input; refusing to rescale texture coordinates`);
  const pixels=await sharp(generated).ensureAlpha().raw().toBuffer();
  const result=Buffer.from(original);let filled=0;
  for(let i=0;i<result.length;i+=4)if(editMask[i+3]===0){pixels.copy(result,i,i,i+3);result[i+3]=255;filled++;}
  let changedProtected=0,rawChangedProtected=0,protectedTexturePixels=0,rawChangedTexturePixels=0,rawTextureAbsoluteError=0,rawChangedBackgroundPixels=0;
  for(let i=0;i<result.length;i+=4)if(editMask[i+3]!==0){
    if(!result.subarray(i,i+4).equals(original.subarray(i,i+4)))changedProtected++;
    const changed=!pixels.subarray(i,i+3).equals(original.subarray(i,i+3));
    if(changed)rawChangedProtected++;
    if(original[i]===0&&original[i+1]===0&&original[i+2]===0){
      if(changed)rawChangedBackgroundPixels++;
    }else{
      protectedTexturePixels++;
      if(changed)rawChangedTexturePixels++;
      for(let c=0;c<3;c++)rawTextureAbsoluteError+=Math.abs(pixels[i+c]!-original[i+c]!);
    }
  }
  await sharp(result,{raw:{width:manifest.layout.width,height:manifest.layout.height,channels:4}}).png().toFile(path.join(outputDirectory,"generated-preserved.png"));
  const report={model,quality:parameters.quality,variant,maskSent:!omitMask,lightingReferenceSent:!!lighting,prompt:parameters.prompt,status:response.status,filled,changedProtected,rawChangedProtected,
    protectedTexturePixels,rawChangedTexturePixels,rawChangedBackgroundPixels,
    rawProtectedTextureMeanAbsoluteError:protectedTexturePixels?rawTextureAbsoluteError/(3*protectedTexturePixels):0,cache,outputDirectory};
  await fs.writeFile(path.join(outputDirectory,"generation.json"),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}
main().catch(error=>{console.error(error instanceof Error?error.message:String(error));process.exitCode=1;});
