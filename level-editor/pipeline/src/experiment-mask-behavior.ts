/** Controlled mask-only location test; never composites or resizes API outputs. */
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import sharp from "sharp";
import {requireEnv} from "./env.ts";

const hash=(data:Uint8Array)=>crypto.createHash("sha256").update(data).digest("hex");
async function main(){
  const directory=path.resolve(process.argv[2]??"../work/derby-refinement/mask-behavior");
  await fs.mkdir(directory,{recursive:true});
  const width=1024,height=1024;
  const original=Buffer.alloc(width*height*4);
  const squares=[{x:128,y:128},{x:640,y:128},{x:128,y:640},{x:640,y:640}];
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const i=(y*width+x)*4;
    const square=squares.some(s=>x>=s.x&&x<s.x+256&&y>=s.y&&y<s.y+256);
    original[i]=original[i+1]=original[i+2]=square?160:0;original[i+3]=255;
  }
  const input=await sharp(original,{raw:{width,height,channels:4}}).png().toBuffer();
  await fs.writeFile(path.join(directory,"input.png"),input);
  const parameters={model:"gpt-image-2.5-sunburst",quality:"high",size:"1024x1024",n:"1",output_format:"png",
    prompt:"Use the mask. Change only the masked square to solid bright magenta (#FF00FF). Keep the other three gray squares and the black background exactly unchanged. Preserve all positions, edges, and image dimensions."};
  const reports=await Promise.all([0,3].map(async selected=>{
    const name=selected===0?"top-left":"bottom-right";
    const out=path.join(directory,name);await fs.mkdir(out,{recursive:true});
    const mask=Buffer.alloc(original.length,255),display=Buffer.alloc(original.length);
    const square=squares[selected]!;
    for(let y=0;y<height;y++)for(let x=0;x<width;x++){
      const i=(y*width+x)*4,editable=x>=square.x&&x<square.x+256&&y>=square.y&&y<square.y+256;
      mask[i+3]=editable?0:255;
      display[i]=display[i+1]=display[i+2]=editable?255:0;display[i+3]=255;
    }
    const maskPng=await sharp(mask,{raw:{width,height,channels:4}}).png().toBuffer();
    await fs.writeFile(path.join(out,"mask.png"),maskPng);
    await sharp(display,{raw:{width,height,channels:4}}).png().toFile(path.join(out,"mask-visible.png"));
    const form=new FormData();for(const [key,value]of Object.entries(parameters))form.append(key,value);
    form.append("image",new Blob([new Uint8Array(input)],{type:"image/png"}),"input.png");
    form.append("mask",new Blob([new Uint8Array(maskPng)],{type:"image/png"}),"mask.png");
    // Round-trip the actual serialized multipart body before attaching credentials.
    const request=new Request("https://api.openai.com/v1/images/edits",{method:"POST",body:form});
    const parsed=await request.clone().formData();
    const fields=[];
    for(const [key,value]of parsed.entries())fields.push(typeof value==="string"?{key,value}:{key,name:value.name,type:value.type,size:value.size,sha256:hash(new Uint8Array(await value.arrayBuffer()))});
    const uploadedMask=parsed.get("mask");
    if(!(uploadedMask instanceof File)||hash(new Uint8Array(await uploadedMask.arrayBuffer()))!==hash(maskPng))throw new Error("Serialized mask differs from saved PNG");
    await fs.writeFile(path.join(out,"request.json"),JSON.stringify({parameters,fields,editablePixels:65536,protectedPixels:width*height-65536},null,2));
    let response:{status:number;body:{data?:{b64_json?:string}[]}};
    try{response=JSON.parse(await fs.readFile(path.join(out,"response.json"),"utf8"));}
    catch(error){
      if((error as NodeJS.ErrnoException).code!=="ENOENT")throw error;
      request.headers.set("Authorization",`Bearer ${requireEnv("OPENAI_API_KEY")}`);
      const raw=await fetch(request);response={status:raw.status,body:await raw.json() as typeof response.body};
      await fs.writeFile(path.join(out,"response.json"),JSON.stringify(response));
    }
    if(response.status!==200||!response.body.data?.[0]?.b64_json)throw new Error(JSON.stringify(response));
    const png=Buffer.from(response.body.data[0].b64_json,"base64");await fs.writeFile(path.join(out,"result.png"),png);
    const {data:result,info}=await sharp(png).ensureAlpha().raw().toBuffer({resolveWithObject:true});
    if(info.width!==width||info.height!==height)throw new Error("Unexpected output dimensions");
    let magentaInside=0,magentaOutside=0,changedOutside=0,outsideError=0;
    const squareMagenta=squares.map(()=>0);
    for(let y=0;y<height;y++)for(let x=0;x<width;x++){
      const i=(y*width+x)*4,editable=mask[i+3]===0;
      const magenta=result[i]!>180&&result[i+1]!<90&&result[i+2]!>180;
      if(magenta){if(editable)magentaInside++;else magentaOutside++;squares.forEach((s,j)=>{if(x>=s.x&&x<s.x+256&&y>=s.y&&y<s.y+256)squareMagenta[j]!++;});}
      if(!editable){if(!result.subarray(i,i+3).equals(original.subarray(i,i+3)))changedOutside++;for(let c=0;c<3;c++)outsideError+=Math.abs(result[i+c]!-original[i+c]!);}
    }
    const report={name,selectedSquare:selected,magentaInside,magentaOutside,squareMagenta,changedOutside,meanAbsoluteErrorOutside:outsideError/(3*(width*height-65536)),multipartMaskHashVerified:true};
    await fs.writeFile(path.join(out,"report.json"),JSON.stringify(report,null,2));return report;
  }));
  await fs.writeFile(path.join(directory,"report.json"),JSON.stringify(reports,null,2));console.log(JSON.stringify(reports,null,2));
}
main().catch(error=>{console.error(error);process.exitCode=1;});
