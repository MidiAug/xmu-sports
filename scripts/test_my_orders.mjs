import assert from "node:assert/strict";
import {readFileSync,mkdirSync} from "node:fs";
import {createRequire} from "node:module";
import ts from "typescript";
const require=createRequire(import.meta.url);
const {chromium}=require("C:/Users/22346/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const timeSource=ts.transpileModule(readFileSync("src/lib/time.ts","utf8"),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const timeUrl=`data:text/javascript;base64,${Buffer.from(timeSource).toString("base64")}`;
const source=readFileSync("src/ui/my-orders.ts","utf8")
 .replace('import { invoke } from "@tauri-apps/api/core";','const invoke=(command,args)=>window.__invoke(command,args);')
 .replace('import { liveStatusCache, subscribeLiveStatus } from "../state";','const liveStatusCache=null; const subscribeLiveStatus=fn=>{window.__status=fn;};')
 .replace('"../lib/time"',JSON.stringify(timeUrl))
 .replace('import { askConfirm } from "./confirm-ask";','const askConfirm=opts=>{window.__confirmOptions=opts;return new Promise(resolve=>window.__confirm=resolve);};')
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
  window.__intervals=[];window.setInterval=fn=>{window.__intervals.push(fn);return 1;};
  window.__rows=[
   {order_id:"42",details_id:"71",venue:"翔安校区乒乓球室",sport:"乒乓球",court:"02 号场地",date:"2026-10-27",start:"19:30",end:"21:00",can_cancel:true},
   {order_id:"43",details_id:"72",venue:"思明校区风雨球馆",sport:"羽毛球",court:"05 号场地",date:"2026-10-27",start:"07:00",end:"08:00",can_cancel:true},
  ];
  window.__invoke=async (command,args)=>{
   window.__calls.push({command,args});
   if(command==="get_live_status")return {running:false,armed:false};
   if(command==="get_my_orders") {
    if(window.__failure)return {ok:false,code:"HTTP_FAILED",message:"模拟网络故障"};
    if(window.__hold)return new Promise(resolve=>{window.__finish=()=>resolve({ok:true,account_fingerprint:"a".repeat(64),orders:structuredClone(window.__rows)});});
    return {ok:true,account_fingerprint:"a".repeat(64),orders:structuredClone(window.__rows)};
   }
   if(command==="cancel_my_order")return new Promise(resolve=>{window.__cancelFinish=()=>{window.__rows=window.__rows.filter(row=>row.details_id!==args.detailsId);resolve({ok:true,state:"cancelled"});};});
   throw new Error(command);
  };
  (await import(url)).mountMyOrders(document.querySelector("#orders"));
 },moduleUrl);
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===2);
 assert.match(await page.locator(".my-order").first().innerText(),/明天[\s\S]*07:00[\s\S]*思明/);
 mkdirSync("artifacts/sidebar-orders",{recursive:true});
 await page.evaluate(()=>document.documentElement.dataset.shellTheme="light");
 await page.screenshot({path:"artifacts/sidebar-orders/light.png"});
 await page.evaluate(()=>document.documentElement.dataset.shellTheme="dark");
 await page.screenshot({path:"artifacts/sidebar-orders/dark.png"});
 for(const size of [{width:970,height:480},{width:480,height:720}]) {
  await page.setViewportSize(size);
  assert.ok(await page.evaluate(()=>[...document.querySelectorAll(".order-cancel")].every(el=>{const b=el.getBoundingClientRect();return b.left>=0 && b.right<=innerWidth && b.bottom<=innerHeight;})));
 }
 await page.screenshot({path:"artifacts/sidebar-orders/narrow.png"});
 await page.setViewportSize({width:970,height:815});
 const queries=()=>page.evaluate(()=>window.__calls.filter(c=>c.command==="get_my_orders").length);
 assert.equal(await page.locator(".my-orders-status").count(),0,"no explanatory text when loaded");
 assert.equal(await page.evaluate(async url=>{
  const {orderDay}=await import(url);
  return ["2026-10-26","2026-10-27","2026-10-28","2026-10-29"].map(day=>orderDay(day,new Date("2026-10-26T23:59:00+08:00"))).join(",");
 },moduleUrl),"今天,明天,后天,10/29");
 const beforeHidden=await queries();
 await page.evaluate(()=>{
  window.__hidden=true;
  Object.defineProperty(document,"hidden",{configurable:true,get:()=>window.__hidden});
  window.__now+=300001;
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
 assert.ok(await page.getByRole("button",{name:"刷新我的预约",exact:true}).isDisabled());
 await page.evaluate(()=>document.querySelector("[data-orders-refresh]").click());
 assert.equal(await queries(),initial+1,"single flight");
 await page.evaluate(()=>{window.__hold=false;window.__finish();});
 await page.waitForFunction(()=>!document.querySelector("[data-orders-refresh]").disabled);
 await page.locator(".order-cancel").first().click();
 assert.match(await page.evaluate(()=>window.__confirmOptions.lead),/思明校区风雨球馆.*2026-10-27.*07:00/);
 await page.evaluate(()=>window.__confirm(false));
 assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==="cancel_my_order").length),0);
 await page.locator(".order-cancel").first().click();
 await page.evaluate(()=>window.__confirm(true));
 await page.waitForFunction(()=>!!window.__cancelFinish);
 await page.evaluate(()=>document.querySelector(".order-cancel").click());
 assert.deepEqual(await page.evaluate(()=>window.__calls.filter(c=>c.command==="cancel_my_order")),[
  {command:"cancel_my_order",args:{accountFingerprint:"a".repeat(64),orderId:"43",detailsId:"72"}}
 ]);
 await page.evaluate(()=>window.__cancelFinish());
 await page.waitForFunction(()=>document.querySelectorAll(".my-order").length===1);
 await page.evaluate(()=>{window.__status({running:true,armed:false});window.__now+=300001;window.__intervals.forEach(fn=>fn());});
 const before=await queries();
 assert.ok(await page.locator(".order-cancel").isDisabled());
 await page.evaluate(()=>window.__status({running:false,armed:false}));
 await page.waitForFunction(()=>!document.querySelector(".order-cancel").disabled);
 assert.equal(await queries(),before+1,"refresh when booking finishes");
 await page.evaluate(()=>{window.__now+=300001;window.__failure=true;window.__intervals.forEach(fn=>fn());});
 await page.waitForFunction(()=>document.querySelector(".my-orders-status").textContent.includes("更新失败"));
 assert.equal(await page.locator(".my-order").count(),1,"failed refresh retains snapshot");
 await page.evaluate(()=>{window.__failure=false;window.__rows=[{...window.__rows[0],venue:'<img src=x onerror=alert(1)>'}];window.dispatchEvent(new Event("orders-authenticated"));});
 await page.waitForFunction(()=>document.querySelector(".my-order-venue").textContent.includes("<img"));
 assert.equal(await page.locator("#orders img").count(),0,"escape server text");
 await page.evaluate(()=>{window.__rows=[];window.dispatchEvent(new Event("orders-authenticated"));});
 await page.waitForFunction(()=>document.querySelector(".my-orders-status").textContent.includes("暂无"));
 console.log("Sidebar orders UI passed: sorting, real IPC arguments, confirmation, single flight/cooldown, busy resume, stale/empty/escaped states, light/dark/narrow screenshots");
} finally {await browser.close();}
