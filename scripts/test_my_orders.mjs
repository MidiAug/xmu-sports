import assert from "node:assert/strict";
import {readFileSync,mkdirSync} from "node:fs";
import {createRequire} from "node:module";
import ts from "typescript";
const require=createRequire(import.meta.url);
const {chromium}=require("C:/Users/22346/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const timeSource=ts.transpileModule(readFileSync("src/lib/time.ts","utf8"),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const timeUrl=`data:text/javascript;base64,${Buffer.from(timeSource).toString("base64")}`;
const availabilitySource=ts.transpileModule(readFileSync("src/lib/availability.ts","utf8"),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const availabilityUrl=`data:text/javascript;base64,${Buffer.from(availabilitySource).toString("base64")}`;
const progressUrl=`data:text/javascript;base64,${Buffer.from(ts.transpileModule(readFileSync("src/lib/booking-progress.ts","utf8"),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText).toString("base64")}`;
const confirmSource=ts.transpileModule(readFileSync("src/ui/confirm-ask.ts","utf8"),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const confirmUrl=`data:text/javascript;base64,${Buffer.from(confirmSource).toString("base64")}`;
const source=readFileSync("src/ui/my-orders.ts","utf8")
 .replace('import { invoke } from "@tauri-apps/api/core";','const invoke=(command,args)=>window.__invoke(command,args);')
 .replace('import { liveStatusCache, subscribeLiveStatus } from "../state";','const liveStatusCache=null; const subscribeLiveStatus=fn=>{window.__status=fn;};')
 .replace('"../lib/time"',JSON.stringify(timeUrl))
 .replace('"../lib/availability"',JSON.stringify(availabilityUrl))
 .replace('"../lib/booking-progress"',JSON.stringify(progressUrl))
 .replace('import { resetAvailabilitySessionCache } from "../lib/availability-store";','const resetAvailabilitySessionCache=()=>{window.__availabilityResets=(window.__availabilityResets??0)+1;};')
 .replace('import { askConfirm } from "./confirm-ask";',`import { askConfirm as realAskConfirm } from ${JSON.stringify(confirmUrl)}; const askConfirm=opts=>{window.__confirmOptions=opts;window.__confirm=ok=>document.querySelector(ok?"[data-confirm-ok]":"[data-confirm-cancel]").click();return realAskConfirm(opts);};`)
 .replace('import { toastError, toastSuccess } from "./toast";','const toastError=(...args)=>window.__toasts.push(args);const toastSuccess=toastError;');
const compiled=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const moduleUrl=`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`;
const css=["tokens","shell","forms","pages","overlays","responsive"].map(name=>readFileSync(`src/styles/${name}.css`,"utf8").replace(/^\uFEFF/, "")).join("\n");
const browser=await chromium.launch({headless:true,channel:"msedge"});
try {
 const page=await browser.newPage({viewport:{width:970,height:815}});
 await page.setContent(`<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"><style>${css}</style><div id="app"><div class="shell"><div class="shell-row"><aside class="nav"><div class="nav-brand"><h1>XMU Sports</h1></div><nav class="nav-tabs"><button>预约</button><button class="active">运行</button><button>历史</button><button>设置</button></nav><section class="my-orders" id="orders" aria-label="未核销预约"></section><div class="nav-foot">v0.1.29</div></aside><div class="shell-main"><header class="titlebar"></header><main class="content"><h2>运行状态</h2><p>查看执行进度与预约结果</p><div class="page-card"><strong>当前任务</strong><p>翔安校区乒乓球室　19:30–21:00</p></div></main></div></div></div></div>`);
 await page.evaluate(async url=>{
  window.__calls=[];window.__toasts=[];window.__now=Date.parse("2026-10-26T12:00:00+08:00");
  Date.now=()=>window.__now;
  Object.defineProperty(crypto,"randomUUID",{configurable:true,value:()=>"123e4567-e89b-42d3-a456-426614174000"});
  window.__intervals=[];window.setInterval=fn=>{window.__intervals.push(fn);return 1;};
  window.__authFailure=false;window.__holdRecovery=false;
  window.__rows=[
   {order_id:"42",details_id:"71",venue:"翔安校区乒乓球室",sport:"乒乓球",court:"02 号场地",date:"2026-10-27",start:"19:30",end:"21:00",can_cancel:true},
   {order_id:"43",details_id:"72",venue:"思明校区风雨球馆",sport:"羽毛球",court:"05 号场地",date:"2026-10-27",start:"07:00",end:"08:00",can_cancel:true},
  ];
  window.__invoke=async (command,args)=>{
   window.__calls.push({command,args});
   if(command==="get_live_status")return {running:false,armed:false};
   if(command==="get_my_orders") {
    if(window.__authFailure)return {ok:false,code:"SESSION_EXPIRED",message:"登录状态已失效"};
    if(window.__failure)return {ok:false,code:"HTTP_FAILED",message:"模拟网络故障"};
    if(window.__hold)return new Promise(resolve=>{window.__finish=()=>resolve({ok:true,account_fingerprint:"a".repeat(64),orders:structuredClone(window.__rows)});});
    return {ok:true,account_fingerprint:"a".repeat(64),orders:structuredClone(window.__rows),fetched_at:window.__fetchedAt ?? new Date(Date.now()).toISOString()};
   }
   if(command==="reauthenticate_availability") {
    if(window.__holdRecovery)return new Promise(resolve=>{window.__finishRecovery=()=>{window.__authFailure=false;resolve({ok:true,authenticated:true,account_checked:true});};});
    window.__authFailure=false;return {ok:true,authenticated:true,account_checked:true};
   }
   if(command==="cancel_my_order")return new Promise(resolve=>{window.__cancelFinish=()=>{window.__rows=window.__rows.filter(row=>row.details_id!==args.detailsId);resolve({ok:true,state:"cancelled"});};});
   throw new Error(command);
  };
  (await import(url)).mountMyOrders(document.querySelector("#orders"));
 },moduleUrl);
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===2);
 assert.equal(await page.locator("#orders [title]").count(),0,"reservation controls do not add tooltips");
 assert.equal(await page.evaluate(()=>window.__calls.find(call=>call.command==="get_my_orders").args.force),false,"startup can reuse the persistent snapshot");
 assert.match(await page.locator(".my-order").first().innerText(),/思明[\s\S]*明天[\s\S]*07:00/);
 assert.deepEqual(await page.evaluate(async url=>{
  const {orderPlace}=await import(url);
  return [
   {venue:"翔安校区健身房",sport:"健身房",court:"爱秋体育馆健身房"},
   {venue:"翔安校区乒乓球室",sport:"乒乓球",court:"02 号场地"},
   {venue:"翔安校区乒乓球室",sport:"乒乓球",court:"04"},
   {venue:"翔安校区健身房",sport:"健身房",court:"健身房"},
  ].map(orderPlace);
 },moduleUrl),[
  {name:"爱秋体育馆健身房",court:""},
  {name:"翔安校区乒乓球室",court:"02 号场地"},
  {name:"翔安校区乒乓球室",court:"04 号场地"},
  {name:"翔安校区健身房",court:""},
 ]);
 await page.evaluate(()=>{
  window.__rows.push({order_id:"44",details_id:"73",venue:"翔安校区健身房",sport:"健身房",court:"爱秋体育馆健身房",date:"2026-10-27",start:"19:30",end:"21:00",can_cancel:true});
  window.dispatchEvent(new Event("orders-authenticated"));
 });
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===3);
 const gym=page.locator(".my-order").filter({hasText:"爱秋体育馆健身房"});
 assert.match(await gym.innerText(),/^爱秋体育馆健身房/);
 assert.doesNotMatch(await gym.innerText(),/翔安校区健身房/);
 mkdirSync("artifacts/sidebar-orders",{recursive:true});
 await page.evaluate(()=>document.documentElement.dataset.shellTheme="light");
 await page.screenshot({path:"artifacts/sidebar-orders/light.png"});
 await page.evaluate(()=>document.documentElement.dataset.shellTheme="dark");
 await page.screenshot({path:"artifacts/sidebar-orders/dark.png"});
 for(const size of [{width:970,height:480},{width:480,height:720}]) {
  await page.setViewportSize(size);
  for(const button of await page.locator(".order-cancel").all()) {
   await button.scrollIntoViewIfNeeded();
   const bounds=await button.boundingBox();
   assert.ok(bounds.x>=0 && bounds.x+bounds.width<=size.width && bounds.y+bounds.height<=size.height);
  }
 }
 await page.screenshot({path:"artifacts/sidebar-orders/narrow.png"});
 await page.setViewportSize({width:970,height:815});
 await page.evaluate(()=>{window.__rows=window.__rows.filter(row=>row.order_id!=="44");window.dispatchEvent(new Event("orders-authenticated"));});
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===2);
 const queries=()=>page.evaluate(()=>window.__calls.filter(c=>c.command==="get_my_orders").length);
 const fresh=await queries();
 await page.evaluate(()=>{window.__now+=61000;window.dispatchEvent(new Event('focus'));document.dispatchEvent(new Event('visibilitychange'));window.__status({running:true,armed:false,operation:'auth'});window.__status({running:false,armed:false,last_event:{ts:new Date(Date.now()).toISOString(),action:'done',result:'http_auth_ready'}});});
 assert.equal(await queries(),fresh,'focus after one minute and login-only completion reuse the fresh snapshot');
 await page.evaluate(()=>{
  window.__hidden=true;Object.defineProperty(document,'hidden',{configurable:true,get:()=>window.__hidden});
  const event={task_id:'booking',run_id:'hidden-booking',ts:new Date(Date.now()+1).toISOString(),action:'done',result:'http_verified'};
  window.__status({running:false,armed:false,last_event:event});
 });
 assert.equal(await queries(),fresh,'hidden booking completion queues refresh without requesting');
 await page.evaluate(()=>{window.__hidden=false;document.dispatchEvent(new Event('visibilitychange'));});
 await page.waitForFunction(n=>window.__calls.filter(c=>c.command==='get_my_orders').length>n,fresh);
 assert.equal(await queries(),fresh+1,'queued mutation bypasses a fresh snapshot when the window becomes visible');
 await page.evaluate(()=>{window.__fetchedAt=new Date(Date.now()-1790000).toISOString();window.dispatchEvent(new Event('orders-authenticated'));});
 await page.waitForFunction(()=>!document.querySelector('[data-orders-refresh]').disabled);
 const oldSnapshotReads=await queries();
 await page.evaluate(()=>{window.__now+=70000;window.__fetchedAt=undefined;window.dispatchEvent(new Event('focus'));});
 await page.waitForFunction(n=>window.__calls.filter(c=>c.command==='get_my_orders').length>n,oldSnapshotReads);
 assert.equal(await queries(),oldSnapshotReads+1,'cached snapshot age uses fetched_at and does not slide forward on read');
 assert.equal(await page.locator(".my-orders-status").count(),0,"no explanatory text when loaded");
 const stableButton=await page.locator(".order-cancel").first().elementHandle();
 await page.evaluate(()=>window.__status({running:false,armed:false}));
 assert.ok(await stableButton.evaluate(el=>el.isConnected),"unchanged live status must not rebuild a pressed button");
 assert.deepEqual(await page.evaluate(async url=>{
  const {orderEnded,orderCanCancel}=await import(url);
  const row={date:"2026-10-27",start:"19:30",end:"21:00",can_cancel:true};
  return [orderCanCancel(row,Date.parse("2026-10-27T18:29:59+08:00")),orderCanCancel(row,Date.parse("2026-10-27T18:30:00+08:00")),orderEnded(row,Date.parse("2026-10-27T21:00:00+08:00"))];
 },moduleUrl),[true,false,true]);
 assert.equal(await page.evaluate(async url=>{
  const {orderDay}=await import(url);
  return ["2026-10-26","2026-10-27","2026-10-28","2026-10-29"].map(day=>orderDay(day,new Date("2026-10-26T23:59:00+08:00"))).join(",");
 },moduleUrl),"今天,明天,后天,10/29");
 assert.deepEqual(await page.evaluate(async url=>{
  const {orderSnapshotFresh}=await import(url);
  const at=s=>Date.parse(`2026-10-26T${s}+08:00`);
  return [
   orderSnapshotFresh(at('12:00:00'),at('12:29:59')),
   orderSnapshotFresh(at('12:00:00'),at('12:30:00')),
   orderSnapshotFresh(at('06:59:00'),at('07:03:59')),
   orderSnapshotFresh(at('06:59:00'),at('07:04:00')),
   orderSnapshotFresh(at('07:10:00'),at('07:15:00')),
   orderSnapshotFresh(at('12:01:00'),at('12:00:00')),
  ];
 },moduleUrl),[true,false,true,false,true,false],'Beijing morning freshness is five minutes; normal freshness is thirty minutes');
 const quietQueries=await queries();
 await page.evaluate(()=>{window.__now+=600000;window.__intervals.forEach(fn=>fn());window.dispatchEvent(new Event('focus'));});
 assert.equal(await queries(),quietQueries,'normal visible window reuses ten-minute-old orders');
 const beforeHidden=await queries();
 await page.evaluate(()=>{
  window.__hidden=true;
  Object.defineProperty(document,"hidden",{configurable:true,get:()=>window.__hidden});
  window.__now+=1800001;
  window.__intervals.forEach(fn=>fn());
 });
 assert.equal(await queries(),beforeHidden,"hidden window must not poll");
 await page.evaluate(()=>{window.__hidden=false;document.dispatchEvent(new Event("visibilitychange"));});
 await page.waitForFunction(()=>!document.querySelector("[data-orders-refresh]").disabled);
 assert.equal(await queries(),beforeHidden+1,"visible stale window refreshes once");
 const initial=await queries();
 await page.getByRole("button",{name:"刷新我的预约",exact:true}).click();
 assert.equal(await queries(),initial,"10s manual cooldown");
 await page.evaluate(()=>{window.__now+=11000;window.__hold=true;});
 await page.getByRole("button",{name:"刷新我的预约",exact:true}).click();
 assert.ok(await page.locator("[data-orders-refresh]").isDisabled());
 assert.ok(await page.locator("[data-orders-refresh]").evaluate(el=>el.classList.contains("is-loading")));
 assert.equal(await page.locator(".my-orders-status").count(),0,"refresh progress is shown on the button, not another row");
 assert.equal(await page.locator("[data-orders-refresh]").getAttribute("aria-label"),"正在更新预约");
 assert.deepEqual(await page.locator("[data-orders-refresh]").evaluate(el=>{
  const style=getComputedStyle(el);return {cursor:style.cursor,background:style.backgroundColor,color:style.color};
 }),{cursor:"default",background:"rgba(0, 0, 0, 0)",color:"rgb(173, 178, 184)"});
 await page.screenshot({path:"artifacts/sidebar-orders/refreshing.png"});
 await page.evaluate(()=>document.querySelector("[data-orders-refresh]").click());
 assert.equal(await queries(),initial+1,"single flight");
 await page.evaluate(()=>{window.__hold=false;window.__finish();});
 await page.waitForFunction(()=>!document.querySelector("[data-orders-refresh]").disabled);
 const cancelBounds=await page.locator(".order-cancel").first().boundingBox();
 await page.mouse.move(cancelBounds.x+cancelBounds.width/2,cancelBounds.y+cancelBounds.height/2);
 await page.mouse.down();
 await page.evaluate(()=>window.__status({running:false,armed:false}));
 await page.mouse.up();

 assert.equal(await page.getByRole("dialog",{name:"取消这场预约？"}).isVisible(),true);
 await page.getByRole("dialog").screenshot({path:"artifacts/sidebar-orders/cancel-confirm.png"});
 assert.match(await page.evaluate(()=>window.__confirmOptions.lead),/思明校区风雨球馆.*2026-10-27.*07:00/);
 await page.evaluate(()=>window.__confirm(false));
 assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==="cancel_my_order").length),0);
 await page.locator(".order-cancel").first().click();
 const beforeConfirmedCancel=await queries();
 await page.evaluate(()=>window.__confirm(true));
 await page.waitForFunction(()=>!!window.__cancelFinish);
 await page.evaluate(()=>document.querySelector(".order-cancel").click());
 assert.deepEqual(await page.evaluate(()=>window.__calls.filter(c=>c.command==="cancel_my_order")),[
  {command:"cancel_my_order",args:{accountFingerprint:"a".repeat(64),orderId:"43",detailsId:"72"}}
 ]);
 await page.evaluate(()=>window.__cancelFinish());
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===1);
 assert.equal(await queries(),beforeConfirmedCancel,'verified cancellation removes one card without refetching the full list');
 const beforeMutation=await queries();
 await page.evaluate(()=>{
  const event={task_id:'booking',run_id:'new-booking',ts:new Date(Date.now()+1).toISOString(),action:'http_phase',result:'submitting'};
  window.__status({running:true,armed:false,last_event:event,recent_events:[event]});
  window.__status({running:false,armed:false,last_event:{...event,action:'done',result:'http_verified'}});
 });
 await page.waitForFunction(n=>window.__calls.filter(c=>c.command==='get_my_orders').length>n,beforeMutation);
 assert.equal(await queries(),beforeMutation+1,'a submitted booking forces exactly one refresh even within the TTL');
 await page.evaluate(()=>{window.__status({running:true,armed:false});window.__now+=1800001;window.__intervals.forEach(fn=>fn());});
 const before=await queries();
 assert.ok(await page.locator(".order-cancel").isDisabled());
 await page.evaluate(()=>window.__status({running:false,armed:false}));
 await page.waitForFunction(()=>!document.querySelector(".order-cancel").disabled);
 assert.equal(await queries(),before+1,"refresh when booking finishes");
 await page.evaluate(()=>{window.__now+=1800001;window.__failure=true;window.__intervals.forEach(fn=>fn());});
 await page.waitForFunction(()=>document.querySelector(".my-orders-status").textContent.includes("更新失败"));
 const failedCount=await queries();
 await page.evaluate(()=>{window.__now+=30000;window.dispatchEvent(new Event('focus'));window.__intervals.forEach(fn=>fn());});
 assert.equal(await queries(),failedCount,'automatic retries respect failure backoff');
 assert.equal(await page.locator(".my-order").count(),1,"failed refresh retains snapshot");
 await page.evaluate(()=>{window.__failure=false;window.__rows=[{...window.__rows[0],venue:'<img src=x onerror=alert(1)>'}];window.dispatchEvent(new Event("orders-authenticated"));});
 await page.waitForFunction(()=>document.querySelector(".my-order-name").textContent.includes("<img"));
 assert.equal(await page.locator("#orders img").count(),0,"escape server text");
 await page.evaluate(()=>{window.__rows=[];window.dispatchEvent(new Event("orders-authenticated"));});
 await page.waitForFunction(()=>document.querySelector(".my-orders-status").textContent.includes("暂无"));
 await page.evaluate(()=>{window.__rows=[{order_id:"52",details_id:"81",venue:"思明校区风雨球馆",sport:"羽毛球",court:"03 号场地",date:"2026-10-28",start:"08:00",end:"09:00",can_cancel:true}];window.__authFailure=true;window.__now+=1800001;window.__intervals.forEach(fn=>fn());});
 await page.waitForFunction(()=>document.querySelector(".my-orders-status")?.textContent.includes("登录状态已失效"));
 const authBefore=await page.evaluate(()=>window.__calls.filter(c=>c.command==="reauthenticate_availability").length);
 assert.equal(authBefore,0,"background auth failure must not open WeChat automatically");
 const beforeLogin=await queries();
 await page.evaluate(()=>window.__holdRecovery=true);
 await page.getByRole("button",{name:"重新登录并刷新预约"}).click();
 await page.waitForFunction(()=>window.__calls.some(c=>c.command==="reauthenticate_availability"));
 assert.ok(await page.locator("[data-orders-refresh]").evaluate(el=>el.classList.contains("is-loading")));
 assert.equal(await page.locator(".my-orders-status").count(),0,"login progress uses the refresh button and native mini-program overlay");
 assert.equal(await page.locator("[data-orders-refresh]").getAttribute("aria-label"),"正在重新登录");
 const recoveryArgs=await page.evaluate(()=>window.__calls.find(c=>c.command==="reauthenticate_availability").args);
 assert.match(recoveryArgs.operationId,/^[0-9a-f-]{36}$/i);
 assert.equal(recoveryArgs.expectedAccount,"a".repeat(64));
 await page.evaluate(()=>{window.__holdRecovery=false;window.__finishRecovery();});
 await page.waitForFunction(n=>window.__calls.filter(c=>c.command==="get_my_orders").length>n,beforeLogin);
 await page.waitForFunction(()=>document.querySelector(".my-order-name")?.textContent.includes("思明"));
 await page.evaluate(()=>{window.__authFailure=true;window.__now+=11000;});
 const recoveriesBeforeManual=await page.evaluate(()=>window.__calls.filter(c=>c.command==="reauthenticate_availability").length);
 await page.getByRole("button",{name:"刷新我的预约",exact:true}).click();
 await page.waitForFunction(n=>window.__calls.filter(c=>c.command==="reauthenticate_availability").length===n+1,recoveriesBeforeManual);
 await page.waitForFunction(()=>!document.querySelector("[data-orders-refresh]").disabled);
 assert.equal(await page.locator("#orders [title]").count(),0);
 assert.ok(await page.evaluate(()=>window.__availabilityResets>=2),"successful recovery clears the shared venue login/cache state");
 const requestsBeforeExpiry=await queries();
 await page.evaluate(()=>{window.__now=Date.parse("2026-10-28T09:00:00+08:00");window.__intervals[1]();});
 assert.equal(await page.locator(".my-order").count(),0,"ended reservation disappears locally");
 assert.equal(await queries(),requestsBeforeExpiry,"local expiry does not query the API");
 await page.evaluate(()=>{
  window.__now=Date.parse("2026-10-29T08:59:59+08:00");
  window.__rows=[{order_id:"53",details_id:"82",venue:"示例体育馆",sport:"羽毛球",court:"02",date:"2026-10-29",start:"10:00",end:"11:00",can_cancel:true}];
  window.dispatchEvent(new Event("orders-authenticated"));
 });
 await page.waitForFunction(()=>document.querySelector(".my-order-name")?.textContent==="示例体育馆");
 const cancelCountBeforeBoundary=await page.evaluate(()=>window.__calls.filter(call=>call.command==="cancel_my_order").length);
 await page.locator(".order-cancel").click();
 await page.getByRole("dialog",{name:"取消这场预约？"}).waitFor();
 await page.evaluate(()=>window.__now=Date.parse("2026-10-29T09:00:00+08:00"));
 await page.getByRole("button",{name:"确认取消预约",exact:true}).click();
 assert.equal(await page.evaluate(()=>window.__calls.filter(call=>call.command==="cancel_my_order").length),cancelCountBeforeBoundary,"confirmation cannot bypass the one-hour cutoff");
 assert.equal(await page.locator(".order-cancel").isDisabled(),true);
 assert.match(await page.locator(".my-order-cancel-reason").innerText(),/1小时/);
 await page.locator(".my-order").screenshot({path:"artifacts/sidebar-orders/cancel-cutoff.png"});
 console.log("Sidebar orders UI passed: tooltips removed, inline spinner, explicit login recovery, sorting, cancellation, busy resume, errors, escaping, light/dark/narrow layout");
} finally {await browser.close();}
