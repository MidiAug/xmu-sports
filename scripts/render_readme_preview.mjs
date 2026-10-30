import assert from "node:assert/strict";
import {mkdirSync, readFileSync, copyFileSync} from "node:fs";
import {resolve} from "node:path";
import {preview} from "vite";
import {launchBrowser} from "./lib/browser.mjs";
import {installShellFixture} from "./fixtures/tauri-shell.js";
import {shellData} from "./fixtures/shell-data.mjs";

// Render the production frontend with synthetic IPC data, never the desktop host.
// Run npm run build first. Captures share a fixed clock and window size.
const output=resolve(".github/assets"), staging=resolve("artifacts/readme-demo");
mkdirSync(staging,{recursive:true});
const {version}=JSON.parse(readFileSync("package.json","utf8"));
const {sports}=JSON.parse(readFileSync("src/catalog/venues.json","utf8"));
const sport=sports.find(s=>s.label==="羽毛球"),venue=sport.venues[0];
const task={task_id:"readme-demo",sport:sport.ocr_label,venue:venue.ocr_label,target_date:"2026-10-31",fire_at:"2026-10-30T07:00:00+08:00",target_time_start:"19:00",target_time_end:"20:00",executor:"http",revision:0,allow_time_fallback:false,time_fallback_order:"down_then_up"};
const target={sport:task.sport,venue:task.venue,date:task.target_date,time:task.target_time_start,end:task.target_time_end};
const phases=["authenticating","checking_orders","captcha","querying_slots","server_precheck","submitting","verifying_order"];
const events=[{ts:"2026-10-30T07:00:00+08:00",task_id:task.task_id,run_id:"readme-run",action:"spawn"},...phases.map((phase,i)=>({ts:`2026-10-30T07:00:${String(i+1).padStart(2,"0")}+08:00`,task_id:task.task_id,run_id:"readme-run",action:"http_phase",result:phase,http:{phase,target}})),{ts:"2026-10-30T07:00:09+08:00",task_id:task.task_id,run_id:"readme-run",action:"done",result:"http_verified",http:{verified:true,target,court:"03",message:"预约成功"}}];
const data={...shellData(),mode:"single",version,task,events,live:{task,running:false,armed:false,recent_events:events,last_event:events.at(-1)}};
data.orders=[{order_id:"demo-order",details_id:"demo-detail",sport:task.sport,venue:task.venue,court:"03",date:task.target_date,start:task.target_time_start,end:task.target_time_end,can_cancel:true}];
data.availabilitySlots=venue.time_slots.map((slot,i)=>({...slot,cells:["01","02","03","04"].map((court,j)=>({court,admin_locked:i===0,status:i===0?"locked":"available",remaining:i===0||j===0?0:1,capacity:1,reason:i===0?"课程使用":""}))}));
const server=await preview({preview:{host:"127.0.0.1",port:1451,strictPort:false}});
const browser=await launchBrowser();
try {
  const context=await browser.newContext({viewport:{width:1100,height:760},deviceScaleFactor:1,reducedMotion:"reduce"});
  await context.addInitScript(installShellFixture,data);
  const page=await context.newPage(),errors=[];
  page.on("pageerror",e=>errors.push(e.message));
  await page.clock.install({time:new Date("2026-10-30T12:00:00+08:00")});
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}`);
  const capture=async(name,selector)=>{
    await page.locator(selector).first().waitFor();
    await page.setViewportSize({width:1100,height:760});
    await page.clock.runFor(500);
    const height=await page.locator(".content").evaluate(el=>Math.max(760,el.scrollHeight+56));
    await page.setViewportSize({width:1100,height});
    await page.locator(".content").evaluate(el=>el.scrollTop=0);
    await page.clock.runFor(1500);
    await page.mouse.move(1095,755);
    await page.screenshot({path:resolve(staging,`${name}-demo.png`)});
  };
  await capture("booking","#slot_host .choice-option");
  await page.locator('.nav-primary [data-tab="plans"]').click();
  await capture("plans",".plan-card");
  await page.locator('.nav-primary [data-tab="live"]').click();
  await capture("progress",".booking-execution.success");
  await page.locator('.nav-primary [data-tab="history"]').click();
  await page.locator(".run-card-head").first().click();
  await capture("history",".run-card-body:not([hidden])");
  assert.deepEqual(errors,[]);
  assert.equal(await page.evaluate(()=>window.__calls.some(c=>["start_http_run","arm_hang","launch_xmu_miniprogram","reauthenticate_availability","cancel_my_order"].includes(c.command))),false);
  mkdirSync(output,{recursive:true});
  for(const name of ["booking","plans","progress","history"]) copyFileSync(resolve(staging,`${name}-demo.png`),resolve(output,`${name}-demo.png`));
  console.log(`README previews rendered from the ${version} production frontend and synthetic data`);
} finally {await browser.close();await new Promise(resolve=>server.httpServer.close(resolve));}
