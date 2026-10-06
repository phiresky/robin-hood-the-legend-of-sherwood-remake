import {spawn} from 'node:child_process';
import path from 'node:path';
import {createServer} from '../../app/node_modules/vite/dist/node/index.js';

const root=process.cwd();
const config=path.join(root,'level-editor/work/croisement03-refinement/restart2/trio-tree-integration-v1/stage-v1/browser-preparation-v1/config.json');
const server=await createServer({root:path.join(root,'level-editor/app'),server:{host:'127.0.0.1',port:0,hmr:false,watch:null,fs:{allow:[root]}}});
try {
  await server.listen();
  const address=server.httpServer.address();
  if(!address || typeof address==='string')throw Error('Expected isolated TCP server');
  console.log('Isolated no-watch Editor ready',address.port);
  const child=spawn(process.execPath,[path.join(root,'level-editor/refinement/browser/verify_publication.mjs'),config,`http://127.0.0.1:${address.port}`],{stdio:'inherit',env:{...process.env,TMPDIR:'/home/phire/.cache'}});
  const code=await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',(code,signal)=>signal?reject(Error('Verifier terminated '+signal)):resolve(code));});
  if(code!==0)throw Error('Verifier exited '+code);
} finally {
  await server.close();
}
