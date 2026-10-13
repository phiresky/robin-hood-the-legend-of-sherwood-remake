import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { execFileSync } from "node:child_process";

// Run before browser admission: successful transforms do not execute module startup.
const app = resolve(process.argv[2] ?? resolve(import.meta.dirname, ".."));
const appRequire = createRequire(resolve(app, "package.json"));
const solidPackage = appRequire.resolve("solid-js/package.json");
const solidRequire = createRequire(solidPackage);
const signalsPackage = solidRequire.resolve("@solidjs/signals/package.json");
const solid = JSON.parse(readFileSync(solidPackage, "utf8"));
const signals = JSON.parse(readFileSync(signalsPackage, "utf8"));
const entry = pathToFileURL(resolve(dirname(solidPackage), "dist/solid.dev.js")).href;
const script = `const m = await import(${JSON.stringify(entry)}); if (typeof m.createSignal !== 'function' || typeof m.DEV?.setConsoleFooter !== 'function') throw Error('Solid development startup API is incomplete'); console.log('PASS_SOLID_BROWSER_DEVELOPMENT_STARTUP');`;
try {
  const result = execFileSync(process.execPath, [
    "--conditions=browser", "--conditions=development", "--input-type=module", "-e", script,
  ], { cwd: app, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  console.log(JSON.stringify({ status: result.trim(), solid: solid.version, signals: signals.version,
    solidPackage, signalsPackage, scope: "Browser/development module evaluation; DOM mounting still requires an actual browser smoke." }));
} catch (error) {
  console.error(JSON.stringify({ status: "FAIL_SOLID_BROWSER_DEVELOPMENT_STARTUP", solid: solid.version,
    signals: signals.version, solidPackage, signalsPackage, error: String(error.stderr ?? error) }));
  process.exitCode = 1;
}
