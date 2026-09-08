import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { readDocument } from "./inputs";
import { bake } from "./bake";

test("only a missing implicit document permits an unedited bake", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "bake-input-test-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const file = path.join(dir, "document.json");
  assert.equal(await readDocument(file, false), undefined);
  await assert.rejects(readDocument(file, true), /cannot read document/);
  await assert.rejects(
    bake({ map: "york", document: file }),
    /cannot read document/,
  );
  await fs.writeFile(file, "{ malformed");
  await assert.rejects(readDocument(file, false), /invalid JSON/);
  await assert.rejects(bake({ map: "york", document: file }), /invalid JSON/);
  await fs.writeFile(file, JSON.stringify({ version: 999 }));
  await assert.rejects(
    bake({ map: "york", document: file }),
    /unsupported version/,
  );
  await assert.rejects(readDocument(dir, false), /cannot read document/);
  await assert.rejects(bake({ map: "../york" }), /invalid map/);
});
