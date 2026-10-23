import assert from "node:assert/strict";
import {readFileSync,mkdirSync,readdirSync} from "node:fs";
import {createRequire} from "node:module";
import ts from "typescript";
const require=createRequire(import.meta.url);
const {chromium}=require("C:/Users/22346/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const source=readFileSync("src/ui/order-recheck.ts","utf8")
  .replace('import { invoke } from "@tauri-apps/api/core";','const invoke = (command, args) => window.__invoke(command,args);')
  .replace('import { toastError, toastInfo } from "./toast";','const toastError = (...args) => window.__toasts.push(args); const toastInfo = toastError;');
const compiled=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const moduleUrl=`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`;
const browser=await chromium.launch({headless:true,channel:"msedge"});
try {
  const page=await browser.newPage({viewport:{width:960,height:720}});
  const cssFile=readdirSync("dist/assets").find(name=>name.endsWith(".css"));
  const css=readFileSync(`dist/assets/${cssFile}`,"utf8");
  await page.setContent(`<html data-shell-theme="light"><style>${css}\nbody{padding:24px}main{max-width:700px;margin:auto}</style><main><div id="pending"></div></main></html>`);
  await page.evaluate(async url=>{
    window.__calls=[];window.__toasts=[];window.__current=true;
    window.__rows=[{request_ref:"a".repeat(64),can_recheck:true,target:{sport:"乒乓球",venue:"原场馆 <img src=x>",date:"2026-10-22",time:"19:30",end:"21:00",court:"04"}},
      {request_ref:"b".repeat(64),can_recheck:false,reason:"旧记录缺少实际场次",target:{venue:"旧场馆"}}];
    window.__invoke=async (command,args)=>{
      window.__calls.push({command,args});
      if(command==="list_pending_orders")return structuredClone(window.__rows);
      if(command==="recheck_order")return new Promise(resolve=>{window.__finish=()=>{window.__rows=[];resolve({ok:true,verified:true,booking_submitted:false});};});
      throw new Error("forbidden command: "+command);
    };
    window.__mount=(await import(url)).mountOrderRecheck;
    await window.__mount(document.querySelector("#pending"),false,()=>window.__current);
  },moduleUrl);
  const buttons=page.getByRole("button",{name:"再次查单",exact:true});
  assert.equal(await buttons.count(),2);
  assert.equal(await buttons.nth(0).isEnabled(),true);
  assert.equal(await buttons.nth(1).isDisabled(),true);
  assert.equal(await page.locator("img").count(),0,"journal text must be escaped");
  assert.match(await page.locator("main").innerText(),/原场馆.*04 号场地/);
  mkdirSync("artifacts/worker-recheck-018",{recursive:true});
  await page.screenshot({path:"artifacts/worker-recheck-018/pending-orders.png"});
  await page.setViewportSize({width:480,height:720});
  const bounds=await buttons.nth(0).boundingBox();
  assert.ok(bounds.x>=0 && bounds.x+bounds.width<=480,"recheck action must stay in narrow viewport");
  await page.setViewportSize({width:960,height:720});
  await buttons.nth(0).click();
  assert.equal(await page.getByRole("button",{name:"正在查单…"}).isDisabled(),true);
  await page.evaluate(()=>document.querySelector("[data-order-recheck='0']").click());
  const requests=await page.evaluate(()=>window.__calls.filter(call=>call.command==="recheck_order"));
  assert.deepEqual(requests,[{command:"recheck_order",args:{requestRef:"a".repeat(64)}}],"send only the durable ref, never editable target/account");
  await page.evaluate(()=>window.__finish());
  await page.waitForFunction(()=>document.querySelector("#pending").innerHTML==="");
  await page.evaluate(async()=>{
    window.__rows=[{request_ref:"a".repeat(64),can_recheck:true,target:{venue:"原场馆",court:"04"}}];
    await window.__mount(document.querySelector("#pending"),true,()=>window.__current);
  });
  assert.equal(await page.getByRole("button",{name:"再次查单",exact:true}).isDisabled(),true,"scheduled/booking work blocks parallel recheck");
  await page.evaluate(async()=>{
    await window.__mount(document.querySelector("#pending"),false,()=>window.__current);
    document.querySelector("button").click();
    window.__current=false;
    document.querySelector("main").innerHTML='<p id="new-page">其他页面</p>';
    window.__finish();
  });
  await page.waitForFunction(()=>document.querySelector("#new-page")?.textContent==="其他页面");
  assert.equal(await page.locator("button").count(),0,"an old response must not repaint a new page");
  assert.equal(await page.evaluate(()=>window.__calls.some(call=>call.command==="start_http_run" || call.command==="arm_hang")),false);
  console.log("Order recheck UI passed: original snapshot/ref only, incomplete legacy blocked, duplicate click blocked, busy gate, escaping and page ownership; no booking command");
} finally {await browser.close();}
