// Run the unchanged publication assertions with additional CDP failure diagnostics.
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname} from 'node:path';
import {pathToFileURL} from 'node:url';
const verifier=resolve('level-editor/refinement/browser/verify_publication.mjs');
let source=await readFile(verifier,'utf8');
source=source.replace("'../../app/tests/cdp.mjs'",JSON.stringify(pathToFileURL(resolve('level-editor/app/tests/cdp.mjs')).href));
source=source.replace("new URL('./publication-check.js',import.meta.url)",JSON.stringify(resolve('level-editor/refinement/browser/publication-check.js')));
source=source.replace(" const request=(method,params)",` const diagnostics=[];ws.addEventListener('message',event=>{const message=JSON.parse(event.data);if(['Runtime.exceptionThrown','Runtime.consoleAPICalled','Page.frameNavigated','Log.entryAdded','Inspector.targetCrashed'].includes(message.method))diagnostics.push({at:new Date().toISOString(),...message});});
 const request=(method,params)`);
source=source.replace(" await request('Page.enable',{});"," await request('Runtime.enable',{});await request('Log.enable',{});await request('Page.enable',{});");
source=source.replace("}finally{ws?.close();", "}finally{await writeFile(join(here,'cdp-diagnostics.json'),JSON.stringify(diagnostics,null,2));ws?.close();");
// Diagnostics live across the outer try/finally scope.
source=source.replace('let ws,id=0;','let ws,id=0;const diagnostics=[];').replace(' const diagnostics=[];ws.addEventListener',' ws.addEventListener');
await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
