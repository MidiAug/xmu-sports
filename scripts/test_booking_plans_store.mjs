import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import ts from "typescript";
const timeSource=readFileSync(new URL("../src/lib/time.ts",import.meta.url),"utf8");
const timeJs=ts.transpileModule(timeSource,{compilerOptions:{module:ts.ModuleKind.ESNext}}).outputText;
const timeUrl=`data:text/javascript;base64,${Buffer.from(timeJs).toString("base64")}`;
const source=readFileSync(new URL("../src/lib/booking-plans-store.ts",import.meta.url),"utf8")
  .replace('"./time"',JSON.stringify(timeUrl))
  .replace('import { invoke } from "@tauri-apps/api/core";','const invoke=()=>Promise.reject(new Error("unused host boundary"));')
  .replace('import { listen } from "@tauri-apps/api/event";','const listen=async()=>()=>{};');
const js=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext}}).outputText;
const {createBookingPlansStore}=await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
let calls=0,finish,now=Date.parse('2026-10-30T12:00:00+08:00');
const store=createBookingPlansStore(()=>{calls++;return new Promise(resolve=>finish=resolve);},()=>now);
const seen=[];const stop=store.subscribe(state=>seen.push(state));
const first=store.read(),same=store.read(true);
assert.equal(first,same);assert.equal(calls,1);
store.invalidate();store.invalidate();
assert.equal(calls,1,"events during a pending read coalesce");
const before={revision:1};finish(before);await first;
assert.equal(calls,2,"an event during the read gets one follow-up snapshot");
const second=store.read();const after={revision:2};finish(after);await second;
assert.equal(await store.read(),after);assert.equal(calls,2,"page re-entry and live paints reuse the snapshot");
assert.deepEqual(seen.map(s=>s.view.revision),[1,2]);
stop();store.invalidate();assert.equal(calls,2,"no view means no background read");
const reenter=store.read();finish({revision:3});await reenter;assert.equal(calls,3);
now+=86400000;
const nextDay=store.read();assert.equal(calls,4,'calendar previews expire on a new Beijing day without a TTL');
finish({revision:4});await nextDay;
let failures=0,broken=true;
const retry=createBookingPlansStore(async()=>{failures++;if(broken)throw new Error('offline');return after;});
const unsubscribe=retry.subscribe(()=>{});
await assert.rejects(retry.read(),/offline/);
assert.equal(failures,1,"a read failure must not create a retry loop");
assert.match(String(retry.snapshot().error),/offline/);
broken=false;assert.equal(await retry.read(true),after);assert.equal(retry.snapshot().error,undefined);
broken=true;await assert.rejects(retry.read(true),/offline/);
assert.equal(retry.snapshot().view,after,"read errors retain the last known snapshot");
unsubscribe();
console.log('Plan store passed: shared reads, event coalescing, page lifetime, failure/retry and retained snapshot');

let refreshCalls=0,finishRefresh;
const shared=createBookingPlansStore(()=>{refreshCalls++;return new Promise(resolve=>finishRefresh=resolve);});
shared.subscribe(()=>{});
const inflight=shared.read();assert.equal(shared.read(true),inflight);
finishRefresh({plans:{rules:[],attempts:[],paused:false}});await inflight;
assert.equal(refreshCalls,1,'force during a pending read reuses the snapshot without an extra fetch');
