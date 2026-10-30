// Real frontend in an isolated browser, with an in-memory Tauri stub. No booking access.
import assert from "node:assert/strict";
import {launchBrowser} from "./lib/browser.mjs";
import { resolve } from "node:path";
import { mkdirSync } from "node:fs";
const browser = await launchBrowser();
try {
  const page = await browser.newPage({viewport:{width:960,height:820}});
  await page.clock.install({time:new Date("2026-10-23T06:58:00+08:00")});
  await page.addInitScript(() => {
    const initial = {task_id:"browser-only",revision:0,sport:"健身房",venue:"翔安校区健身房",target_date:"2026-10-24",fire_at:"2026-10-23T07:00:00+08:00",target_time_start:"16:30",target_time_end:"18:00",allow_time_fallback:false,time_fallback_order:"down_then_up",executor:"http",court_offset:0};
    window.__TAURI_INTERNALS__ = {
      metadata:{currentWindow:{label:"main"},currentWebview:{label:"main"}},
      transformCallback:()=>1, unregisterCallback:()=>{},
      invoke:async (cmd,args) => {
        const task = JSON.parse(localStorage.getItem("fixture-task") ?? JSON.stringify(initial));
        if(cmd === "read_task_raw") return task;
        if(cmd === "save_task") { const next={...args.task,revision:(task.revision??0)+1}; localStorage.setItem("fixture-task",JSON.stringify(next)); return next; }
        if(cmd === "get_live_status") return {task,armed:localStorage.getItem("fixture-locked")==="true",running:false,recent_events:[],fire_at:task.fire_at};
        if(cmd === "get_shell_prefs") return {theme:"light",close_mode:"tray",favorite_sport_ids:[],venue_slot_prefs:{}};
        if(cmd === "get_history") return [];
        if(cmd.includes("version")) return "0.1.22";
        if(cmd.includes("is_maximized")) return false;
        if(cmd.includes("listen")) return 1;
        if(cmd === "arm_hang" || cmd === "start_http_run") throw new Error("Booking is forbidden in this fixture");
        return null;
      }
    };
  });
  await page.goto(process.env.XMU_TEST_BASE_URL ?? "http://127.0.0.1:1421");
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.locator(".task-advanced summary").click();
  await page.clock.runFor(600);
  await page.waitForFunction(()=>{const el=document.querySelector('.task-advanced');return el.getBoundingClientRect().bottom<=el.closest('.content').getBoundingClientRect().bottom+1;});
  const advancedBounds=await page.locator('.task-advanced').evaluate(el=>({box:el.getBoundingClientRect().toJSON(),viewport:el.closest('.content').getBoundingClientRect().toJSON(),scroll:el.closest('.content').scrollTop,open:el.open}));
  assert.ok(advancedBounds.box.bottom<=advancedBounds.viewport.bottom+1,`expanded options scroll into view: ${JSON.stringify(advancedBounds)}`);
  const delay=page.locator("#fire_delay");
  await delay.fill("10");
  await page.waitForFunction(()=>JSON.parse(localStorage.getItem("fixture-task")??"null")?.fire_delay_seconds===10);
  assert.match(await page.locator("#task_execution").innerText(), /07:00:10/);
  await page.reload();
  await page.locator(".task-advanced summary").click();
  assert.equal(await delay.inputValue(),"10");
  assert.equal(await page.locator("#fire_time").innerText(),"07:00");
  assert.match(await page.locator("#task_execution").innerText(), /07:00:10/);
  await delay.fill("180");
  await page.waitForFunction(()=>JSON.parse(localStorage.getItem("fixture-task"))?.fire_at==="2026-10-23T07:03:00+08:00");
  await page.reload();
  await page.locator(".task-advanced summary").click();
  assert.equal(await page.locator("#fire_time").innerText(),"07:00");
  assert.equal(await delay.inputValue(),"180");
  await delay.fill("301");
  assert.equal(await page.locator("#setup_primary").isDisabled(),true);
  assert.equal(await page.evaluate(()=>JSON.parse(localStorage.getItem("fixture-task")).fire_delay_seconds),180);
  await delay.fill("10");
  await page.waitForFunction(()=>JSON.parse(localStorage.getItem("fixture-task"))?.fire_delay_seconds===10);
  mkdirSync("artifacts/delayed-schedule-022",{recursive:true});
  await page.screenshot({path:resolve("artifacts/delayed-schedule-022/settings.png"),fullPage:true});
  await page.evaluate(()=>localStorage.setItem("fixture-locked","true"));
  await page.reload();
  await page.locator(".task-advanced summary").click();
  assert.equal(await delay.isDisabled(),true);
  console.log("Schedule delay browser checks passed: save, reopen, minute carry, invalid input, locked task; no booking available");
} finally { await browser.close(); }
