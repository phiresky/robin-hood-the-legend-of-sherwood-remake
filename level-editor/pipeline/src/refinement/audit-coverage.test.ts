import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import sharp from "sharp";
import { auditCoverage, type ObjectCoverage } from "./audit-coverage.ts";

const hash = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex");
const png = (pixels: number[], width: number) => sharp(Buffer.from(pixels), {
  raw: { width, height: 1, channels: 4 },
}).png().toBuffer();

test("actual CLI audits black object pixels and excludes blue or black background by coverage", async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "texture-audit-coverage-"));
  try {
    const coverage = await png([255,255,255,255, 255,255,255,0, 255,255,255,255, 255,255,255,255],4);
    await fs.writeFile(path.join(directory,"coverage.png"),coverage);
    await fs.writeFile(path.join(directory,"mask.png"),await png([255,255,255,255, 255,255,255,255, 255,255,255,255, 255,255,255,0],4));
    const manifest = {
      source_image: path.join(directory,"source.png"), layout:{width:4,height:1},
      views:[{input:"frame.png",mask:"mask.png",object_coverage:{file:"coverage.png",sha256:hash(coverage),channel:"alpha"},
        camera_matrix_world:[[1,0,0,2],[0,1,0,-.5/Math.sin(35*Math.PI/180)],[0,0,1,0],[0,0,0,1]],
        ortho_scale:1,crop:{left:0,top:0,width:4,height:1}}],
    };
    await fs.writeFile(path.join(directory,"views.json"),JSON.stringify(manifest));
    const driver=fileURLToPath(new URL("./generate-textures.ts",import.meta.url));
    for (const background of [[24,66,95],[0,0,0]]) {
      await fs.writeFile(path.join(directory,"frame.png"),await png([0,0,0,255,...background,255,128,128,128,255,90,90,90,255],4));
      for (const changedBlackObject of [false,true]) {
        await fs.writeFile(path.join(directory,"source.png"),await png([changedBlackObject?10:0,0,0,255,200,100,50,255,128,128,128,255,1,2,3,255],4));
        const child=spawnSync(process.execPath,[driver,directory,"--audit-input"],{encoding:"utf8"});
        assert.equal(child.status,changedBlackObject?1:0,child.stdout+child.stderr);
        const report=JSON.parse(await fs.readFile(path.join(directory,"source-pixel-verification.json"),"utf8"));
        assert.equal(report.knownComparedToOriginalFraming,2);
        assert.equal(report.framingMismatches,changedBlackObject?1:0);
      }
    }
    delete (manifest.views[0] as {object_coverage?:unknown}).object_coverage;
    await fs.writeFile(path.join(directory,"views.json"),JSON.stringify(manifest));
    const legacy=spawnSync(process.execPath,[driver,directory,"--audit-input"],{encoding:"utf8"});
    assert.equal(legacy.status,1);
    assert.match(legacy.stderr,/requires explicit pinned object_coverage/);
  } finally { await fs.rm(directory,{recursive:true,force:true}); }
});

test("coverage validates its pin, extent and declared channel without a color heuristic", async () => {
  const directory=await fs.mkdtemp(path.join(os.tmpdir(),"texture-coverage-"));
  try {
    const bytes=await png([0,0,0,255,255,255,255,255],2);
    await fs.writeFile(path.join(directory,"mask.png"),bytes);
    const coverage:ObjectCoverage={file:"mask.png",sha256:hash(bytes),channel:"red"};
    assert.deepEqual(await auditCoverage(directory,coverage,2,1),new Uint8Array([0,1]));
    await assert.rejects(auditCoverage(directory,undefined,2,1),/requires explicit pinned/);
    await assert.rejects(auditCoverage(directory,{...coverage,sha256:"0".repeat(64)},2,1),/hash changed/);
    await assert.rejects(auditCoverage(directory,coverage,3,1),/dimensions/);
    const grayscale=await sharp(Buffer.from([0,255]),{raw:{width:2,height:1,channels:1}}).toColourspace("b-w").png().toBuffer();
    await fs.writeFile(path.join(directory,"gray.png"),grayscale);
    assert.deepEqual(await auditCoverage(directory,{file:"gray.png",sha256:hash(grayscale),channel:"red"},2,1),new Uint8Array([0,1]));
    const colored=await png([255,0,0,255,255,255,255,255],2);
    await fs.writeFile(path.join(directory,"colored.png"),colored);
    await assert.rejects(auditCoverage(directory,{file:"colored.png",sha256:hash(colored),channel:"red"},2,1),/grayscale mask/);
    const opaque=await sharp(bytes).removeAlpha().png().toBuffer();
    await fs.writeFile(path.join(directory,"opaque.png"),opaque);
    await assert.rejects(auditCoverage(directory,{file:"opaque.png",sha256:hash(opaque),channel:"alpha"},2,1),/no alpha/);
  } finally {await fs.rm(directory,{recursive:true,force:true});}
});
