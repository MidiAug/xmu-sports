import assert from "node:assert/strict";
import ts from "typescript";
import { readFileSync } from "node:fs";
let complete, completeDownload, completeInstall, progress, calls=0, downloads=0, installs=0;
let downloadArgs;
const stored=new Map();
globalThis.localStorage={getItem:key=>stored.get(key) ?? null,setItem:(key,value)=>stored.set(key,value)};
globalThis.__updateInvoke=async (command,args)=>{
  if(command==='check_app_update') {calls++;return new Promise(resolve=>complete=resolve);}
  if(command==='download_app_update') {downloadArgs=args;downloads++;return new Promise((resolve,reject)=>completeDownload={resolve,reject});}
  if(command==='install_app_update') {installs++;return new Promise((resolve,reject)=>completeInstall={resolve,reject});}
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
assert.equal(module.updateDownloadRoute(),'official');
assert.equal(module.setUpdateDownloadRoute('https://evil.test/'),false);
assert.equal(module.setUpdateDownloadRoute('gh-proxy'),true);
assert.equal(module.updateDownloadRoute(),'gh-proxy');
const download=module.downloadAppUpdate();await Promise.resolve();
assert.deepEqual(downloadArgs,{mirror:'gh-proxy',automatic:false});
assert.equal(module.setUpdateDownloadRoute('official'),false,'active download route remains fixed');
await module.downloadAppUpdate();await module.checkAppUpdate();assert.equal(calls,1);assert.equal(downloads,1);
progress({payload:{received:40,total:100}});assert.equal(last.progress,40);
completeDownload.reject('校验失败');await download;assert.equal(last.phase,'error');assert.equal(stopped,1);
assert.equal(module.setUpdateDownloadRoute('ghproxy'),true);
const retry=module.downloadAppUpdate();await Promise.resolve();completeDownload.resolve('verified.exe');await retry;
assert.deepEqual(downloadArgs,{mirror:'ghproxy',automatic:false},'retry uses newly selected route without repeating metadata check');
assert.equal(last.phase,'downloaded');assert.equal(last.path,'verified.exe');assert.equal(stopped,2);
const install=module.performAppUpdate();assert.equal(last.phase,'installing');
await module.performAppUpdate();assert.equal(installs,1);
completeInstall.reject('请先结束预约');await install;assert.equal(last.failedAt,'install');
const installRetry=module.performAppUpdate();assert.equal(installs,2);
completeInstall.reject('安装包已改变，请重新下载');await installRetry;assert.equal(last.failedAt,'download');
const redownload=module.performAppUpdate();await Promise.resolve();assert.equal(downloads,3);completeDownload.resolve('verified-again.exe');await redownload;
subscribe();const old=last;const again=module.checkAppUpdate();complete({status:'unpublished',version:'',size:0,notes:''});await again;
assert.equal(last,old,'Disposed page receives no updates');
console.log('App update controller passed: no implicit requests, single-flight, navigation retention, progress, failed download retry and disposal');
