import assert from "node:assert/strict";
import ts from "typescript";
import { readFileSync } from "node:fs";
let complete, completeDownload, progress, calls=0, downloads=0;
globalThis.__updateInvoke=async command=>{
  if(command==='check_app_update') {calls++;return new Promise(resolve=>complete=resolve);}
  if(command==='download_app_update') {downloads++;return new Promise((resolve,reject)=>completeDownload={resolve,reject});}
  throw Error(command);
};
let stopped=0;
globalThis.__updateListen=async(event,callback)=>{assert.equal(event,'app-update-progress');progress=callback;return ()=>stopped++;};
const source=readFileSync('src/lib/app-update.ts','utf8').replace(/^import .* from .*;\r?\n/gm,'');
const js=ts.transpileModule('const invoke=globalThis.__updateInvoke,listen=globalThis.__updateListen;\n'+source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText;
const module=await import('data:text/javascript;base64,'+Buffer.from(js).toString('base64'));
let first,last;
const unsubscribe=module.watchAppUpdate(state=>first=state);
assert.equal(first.phase,'idle');assert.equal(calls,0);
await module.downloadAppUpdate();assert.equal(downloads,0);
const check=module.checkAppUpdate();await module.checkAppUpdate();assert.equal(calls,1);
unsubscribe();const subscribe=module.watchAppUpdate(state=>last=state);
assert.equal(last.phase,'checking');
complete({status:'available',version:'0.2.0',size:100,notes:''});await check;assert.equal(last.phase,'ready');
const download=module.downloadAppUpdate();await Promise.resolve();
await module.downloadAppUpdate();await module.checkAppUpdate();assert.equal(calls,1);assert.equal(downloads,1);
progress({payload:{received:40,total:100}});assert.equal(last.progress,40);
completeDownload.reject('校验失败');await download;assert.equal(last.phase,'error');assert.equal(stopped,1);
const retry=module.downloadAppUpdate();await Promise.resolve();completeDownload.resolve('verified.exe');await retry;
assert.equal(last.phase,'downloaded');assert.equal(last.path,'verified.exe');assert.equal(stopped,2);
subscribe();const old=last;const again=module.checkAppUpdate();complete({status:'unpublished',version:'',size:0,notes:''});await again;
assert.equal(last,old,'Disposed page receives no updates');
console.log('App update controller passed: manual-only, single-flight, navigation retention, progress, failed download retry and disposal');
