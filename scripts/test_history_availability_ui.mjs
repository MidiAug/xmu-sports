// Full frontend with a fake Tauri boundary; no real login, school calls or bookings.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { mkdirSync } from "node:fs";
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE,".cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright"));
const artifacts=process.env.XMU_UI_ARTIFACTS ?? "artifacts/compact-availability-025";
const browser=await chromium.launch({channel:"msedge",headless:true});
try {
  const context=await browser.newContext({viewport:{width:960,height:900}});
  const page=await context.newPage();
  if(process.env.XMU_FORCE_POPOVER_FALLBACK) await page.context().addInitScript(()=>Object.defineProperty(HTMLElement.prototype,"showPopover",{value:undefined,configurable:true}));
  await page.clock.install({time:new Date("2026-10-23T10:00:00+08:00")});
  await page.context().addInitScript(()=>{
    let task={task_id:"fixture",sport:"羽毛球",venue:"翔安校区羽毛球场",target_date:"2026-10-24",fire_at:"2026-10-23T07:00:00+08:00",target_time_start:"13:00",target_time_end:"14:00",executor:"http",revision:0,allow_time_fallback:false,time_fallback_order:"down_then_up"};
    const target={sport:task.sport,venue:task.venue,date:task.target_date,time:"13:00",end:"14:00"};
    const phases=["authenticating","normal_ui_opening","semantic_login_control","native_login_opening","semantic_login_control","native_login_opening","uia_window_message","normal_ui_http_ready","checking_orders","captcha","querying_slots","submitting","verifying_order"];
    const events=[{ts:"2026-10-23T07:00:00+08:00",task_id:"fixture",run_id:"run",action:"spawn"},...phases.map((phase,i)=>({ts:`2026-10-23T07:00:${String(i+1).padStart(2,"0")}+08:00`,task_id:"fixture",run_id:"run",action:"http_phase",result:phase,http:{phase,target}})),{ts:"2026-10-23T07:00:30+08:00",task_id:"fixture",run_id:"run",action:"done",result:"http_verified",http:{verified:true,target,message:"预约成功"}}];
    window.__queries=[]; window.__authCalls=[]; window.__cancelCalls=[]; window.__bookingCalls=[];
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:"main"},currentWebview:{label:"main"}},transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(cmd,args)=>{
      if(cmd==="read_task_raw") return task;
      if(cmd==="save_task") return task={...args.task,revision:task.revision+1};
      if(cmd==="get_live_status") return {task,armed:false,running:false,recent_events:[]};
      if(cmd==="list_recent_events") {
        window.__historyRequests=(window.__historyRequests ?? 0)+1;
        const snapshot=window.__historyCleared ? [] : events;
        if(window.__holdHistory) return new Promise(resolve=>window.__finishHistory=()=>resolve(snapshot));
        return snapshot;
      }
      if(cmd==="clear_run_history") {window.__historyCleared=true;return "已清空";}
      if(cmd==="get_shell_prefs") return {theme:"light",close_mode:"tray",favorite_sport_ids:[],venue_slot_prefs:{}};
      if(cmd==="reauthenticate_availability") {
        window.__authCalls.push(args);
        if(window.__holdLogin) await new Promise(resolve=>window.__finishLogin=resolve);
        else await new Promise(resolve=>setTimeout(resolve,150));
        if(window.__authFailure) return {ok:false,code:"LOGIN_CONFIRM_REQUIRED",message:"请完成微信确认"};
        window.__authExpired=false;
        return {ok:true,authenticated:true,account_checked:true,account_fingerprint:"fixture-account"};
      }
      if(cmd==="cancel_availability_login") {window.__cancelCalls.push(args);return null;}
      if(cmd==="get_venue_availability") {
        window.__queries.push(args);
        if(window.__authExpired) return {ok:false,code:"SESSION_EXPIRED",message:"登录信息失效"};
        if(window.__refreshFails && args.force) throw Error("fixture network failure");
        await new Promise(resolve=>setTimeout(resolve,120));
        const response={ok:true,state:"availability_ready",account_fingerprint:"fixture-account",cached:!args.force,sport:args.sport,venue:args.venue,date:args.date,fetched_at:"2026-10-23T09:00:00+08:00",can_choose:true,slots:[
          {start:"14:00",end:"15:00",cells:[{court:"03",admin_locked:true,status:"locked",remaining:1,capacity:1,reason:""},{court:"07",admin_locked:!!window.__lockSelected,status:"available",remaining:0,capacity:1,reason:""},{court:"08",admin_locked:false,status:"available",remaining:1,capacity:1,reason:""}]},
          {start:"12:00",end:"13:00",cells:[{court:"01",admin_locked:false,status:"available",remaining:0,capacity:1,reason:""}]},
          {start:"13:00",end:"14:00",cells:[{court:"01",admin_locked:!args.force,status:args.force ? "available":"locked",remaining:1,capacity:1,reason:"课程使用"},{court:"02",admin_locked:!args.force,status:args.force ? "available":"locked",remaining:0,capacity:1,reason:"课程使用"}]}
        ]};
        return {...response,snapshots:Object.fromEntries(["2026-10-23","2026-10-24"].map(date=>[date,{...response,date}]))};
      }
      if(cmd.includes("version")) return "0.1.25";
      if(cmd.includes("is_maximized")) return false;
      if(cmd.includes("listen")) return 1;
      if(cmd==="arm_hang" || cmd==="start_http_run") {window.__bookingCalls.push(cmd); throw Error("Booking forbidden");}
      return null;
    }};
  });
  await page.goto(process.env.XMU_TEST_BASE_URL ?? "http://127.0.0.1:1421");
  await page.locator('[data-date="2026-10-24"]').click();
  const panel=page.locator("#availability_panel");
  await page.clock.runFor(1000);
  const slot=page.locator('#slot_host');
  const court=page.locator('#court_host');
  const choose=async(host,value)=>{await host.locator('.shell-select-trigger').click();await host.locator(`[data-value="${value}"]`).click();};
  assert.equal(await panel.locator('details,strong').count(),0);
  assert.equal(await page.locator('#slot_host [data-value="13:00|14:00"]').evaluate(el=>getComputedStyle(el).cursor),"default");
  const stable=await page.locator('.content').evaluate(el=>{
    const before=el.clientWidth;
    const spacer=document.createElement('div');spacer.style.height='3000px';el.append(spacer);
    const during=el.clientWidth;spacer.remove();
    return {before,during,after:el.clientWidth,gutter:getComputedStyle(el).scrollbarGutter};
  });
  assert.equal(stable.gutter,"stable");assert.equal(stable.before,stable.during);assert.equal(stable.before,stable.after);

  assert.equal(await panel.locator('..').getAttribute('class'),"booking-field-heading");
  const slotBox=await slot.locator('.shell-select-trigger').boundingBox();
  const courtBox=await court.locator('.shell-select-trigger').boundingBox();
  const refreshBox=await panel.locator('button').boundingBox();
  assert.ok(Math.abs(slotBox.y-courtBox.y)<1,"Time and court controls stay aligned");
  assert.ok(refreshBox.y+refreshBox.height<=slotBox.y,"Refresh shares the time heading row");
  assert.equal((await panel.innerText()).trim(),"刷新");
  assert.match(await panel.locator('button').getAttribute('title'),/09:00:00.*[\s\S]*查询时快照/);
  assert.match(await slot.locator('.shell-select-value').innerText(),/全部锁定/);
  assert.equal(await slot.locator('[data-value="13:00|14:00"]').isDisabled(),true);
  assert.equal(await slot.locator('[data-value="12:00|13:00"]').isEnabled(),true);
  assert.equal(await slot.locator('[data-value="14:00|15:00"]').isEnabled(),true);
  assert.equal(await court.locator('.shell-select-trigger').isDisabled(),true);
  assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  assert.equal(await page.locator('#setup_check').isEnabled(),true);
  await slot.locator('.shell-select-trigger').focus();
  await page.keyboard.press("ArrowDown");
  assert.equal(await page.evaluate(()=>document.activeElement?.dataset.value),"14:00|15:00");
  await page.keyboard.press("End");
  assert.equal(await page.evaluate(()=>document.activeElement?.dataset.value),"12:00|13:00");
  await page.keyboard.press("Escape");
  await slot.locator('[data-value="13:00|14:00"]').evaluate(button=>button.click());
  assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  const beforeSwitch=await page.evaluate(()=>window.__queries.length);
  for(let i=0;i<4;i++) {
    for(const date of ["2026-10-23","2026-10-24"]) {
      await page.locator(`[data-date="${date}"]`).click();
      assert.equal((await panel.locator('button').innerText()).trim(),"刷新");
      assert.equal(await page.locator('#setup_primary').isDisabled(),true);
      await page.clock.runFor(500);
    }
  }
  assert.equal(await page.evaluate(()=>window.__queries.length),beforeSwitch,"Date switching must not invoke the host");
  await page.evaluate(()=>{
    window.__tabRefreshing=0;
    window.__availabilityObserver=new MutationObserver(()=>{
      if(document.querySelector('#availability_panel')?.textContent.includes('刷新中')) window.__tabRefreshing++;
    });
    window.__availabilityObserver.observe(document.querySelector('#panel'),{childList:true,subtree:true,characterData:true});
  });
  for(const tab of ["live","history","settings","history","live"]) {
    await page.locator(`.nav-tabs [data-tab="${tab}"]`).click();await page.clock.runFor(100);
    await page.locator('.nav-tabs [data-tab="setup"]').click();await page.clock.runFor(700);
    assert.equal((await panel.locator('[data-refresh-availability]').innerText()).trim(),"刷新");
    assert.equal(await page.evaluate(()=>window.__queries.length),beforeSwitch,"Returning from another tab must not query availability");
    assert.match(await slot.locator('.shell-select-value').innerText(),/全部锁定/);
    assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  }
  assert.equal(await page.evaluate(()=>window.__tabRefreshing),0,"Tab remount must never flash Refreshing");
  await page.evaluate(()=>window.__availabilityObserver.disconnect());
  await page.context().addInitScript(()=>{
    window.__loadRefreshing=0;
    new MutationObserver(()=>{
      if(document.querySelector('#availability_panel')?.textContent.includes('刷新中')) window.__loadRefreshing++;
    }).observe(document,{childList:true,subtree:true,characterData:true});
  });
  await page.reload();await page.clock.runFor(1000);
  // Cold start intentionally selects the next release day; choose a cached day.
  await page.locator('[data-date="2026-10-24"]').click();await page.clock.runFor(500);
  assert.equal(await page.evaluate(()=>window.__queries.length),0,"A real document reload restores local snapshots without RPC");
  assert.equal(await page.evaluate(()=>window.__loadRefreshing),0,"Reload must not flash Refreshing");
  assert.equal((await panel.locator('[data-refresh-availability]').innerText()).trim(),"刷新");
  assert.match(await slot.locator('.shell-select-value').innerText(),/全部锁定/);
  const reopened=await page.context().newPage();
  await reopened.clock.install({time:new Date("2026-10-23T10:00:00+08:00")});
  await reopened.goto(process.env.XMU_TEST_BASE_URL ?? "http://127.0.0.1:1421");await reopened.clock.runFor(1000);
  await reopened.locator('[data-date="2026-10-24"]').click();await reopened.clock.runFor(500);
  assert.equal(await reopened.evaluate(()=>window.__queries.length),0,"A new JS environment restores persisted snapshots without querying");
  assert.equal(await reopened.evaluate(()=>window.__loadRefreshing),0);
  assert.match(await reopened.locator('#slot_host .shell-select-value').innerText(),/全部锁定/);
  await reopened.close();
  console.log("Persistence browser checks passed: document reload and new document, zero availability RPCs and zero Refreshing flashes");
  const noPoll=await page.evaluate(()=>window.__queries.length);
  await page.clock.runFor(60_000);
  assert.equal(await page.evaluate(()=>window.__queries.length),noPoll);
  await page.evaluate(()=>window.__refreshFails=true);
  await panel.getByRole("button",{name:"刷新场次状态"}).click();
  await page.clock.runFor(300);
  assert.match(await panel.innerText(),/仍使用上次缓存/);
  assert.equal(await court.locator('.shell-select-trigger').isDisabled(),true);
  await page.evaluate(()=>window.__refreshFails=false);
  await page.clock.runFor(5100);
  await panel.getByRole("button",{name:"重试",exact:true}).click();
  await page.clock.runFor(300);
  assert.equal(await slot.locator('[data-value="13:00|14:00"]').isEnabled(),true);
  assert.equal(await court.locator('.shell-select-trigger').isEnabled(),true);
  assert.equal(await page.locator('#setup_primary').isEnabled(),true);
  assert.equal(await page.evaluate(()=>window.__queries.at(-1).force),true);
  const refreshed=await page.evaluate(()=>window.__queries.length);
  await panel.getByRole("button",{name:"刷新场次状态"}).click();await page.clock.runFor(100);
  assert.equal(await page.evaluate(()=>window.__queries.length),refreshed,"Rapid manual refresh stays within the cooldown");
  for(const date of ["2026-10-23","2026-10-24"]) {
    await page.locator(`[data-date="${date}"]`).click();await page.clock.runFor(300);
    assert.equal(await slot.locator('[data-value="13:00|14:00"]').isEnabled(),true,"Refresh updates both dates");
  }
  assert.equal(await page.evaluate(()=>window.__queries.length),refreshed);
  await choose(slot,"14:00|15:00");
  assert.equal((await slot.locator('.shell-select-badge').first().textContent()),"部分锁定");
  assert.equal(await slot.locator('.shell-select-option-hint').count(),0);
  assert.equal(await court.locator('.shell-select-option-hint').count(),0);
  assert.deepEqual([...new Set(await court.locator('.shell-select-badge').allTextContents())].sort(),["已满","已锁"].sort());
  assert.equal(await court.locator('[data-value="03"]').isDisabled(),true);
  assert.match(await court.locator('[data-value="03"]').innerText(),/已锁/);
  assert.equal(await court.locator('[data-value="07"]').isEnabled(),true);
  assert.match(await court.locator('[data-value="07"]').innerText(),/已满/);
  await choose(court,"07");
  assert.equal(await page.locator('#setup_primary').isEnabled(),true);
  await court.locator('[data-value="03"]').evaluate(button=>button.click());
  assert.match(await court.locator('.shell-select-value').innerText(),/07/);
  mkdirSync(artifacts,{recursive:true});
  await page.screenshot({path:resolve(artifacts,"partial.png"),fullPage:true});
  await court.locator('.shell-select-trigger').click();
  await page.screenshot({path:resolve(artifacts,"courts.png"),fullPage:true});
  await page.keyboard.press('Escape');
  await page.evaluate(()=>window.__lockSelected=true);
  await page.clock.runFor(5100);
  await panel.getByRole("button",{name:"刷新场次状态"}).click();
  await page.clock.runFor(300);
  assert.match(await court.locator('.shell-select-value').innerText(),/已锁/);
  assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  assert.equal(await court.locator('.shell-select-trigger').isEnabled(),true);
  await choose(court,"08");
  assert.equal(await page.locator('#setup_primary').isEnabled(),true);
  await choose(slot,"12:00|13:00");
  assert.equal(await page.locator('#setup_primary').isEnabled(),true);
  assert.match(await slot.locator('.shell-select-value').innerText(),/已满/);
  assert.match(await court.locator('[data-value="01"]').innerText(),/已满/);
  assert.equal(await court.locator('[data-value="01"]').isEnabled(),true);
  await page.setViewportSize({width:550,height:900});
  await slot.locator('.shell-select-trigger').click();
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
  const box=await slot.locator('.shell-select-menu').boundingBox();
  assert.ok(box.x>=0 && box.x+box.width<=550);
  await page.screenshot({path:resolve(artifacts,"narrow.png"),fullPage:true});
  await page.keyboard.press('Escape');
  await page.setViewportSize({width:960,height:900});
  assert.doesNotMatch(await page.locator('.setup-page').innerText(),/[·•]/);
  // Authentication recovery has its own IPC and never starts a booking operation.
  // A forced refresh now automatically attempts recovery. Keep that first
  // attempt unsuccessful so navigation and explicit retry remain testable.
  await page.evaluate(()=>{window.__authExpired=true;window.__authFailure=true;});
  await page.clock.runFor(5100);
  await panel.locator('[data-refresh-availability]').click();
  await page.clock.runFor(300);
  await panel.getByRole('button',{name:'重新登录',exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>window.__authCalls.length),1);
  assert.match(await slot.locator('.shell-select-value').innerText(),/已满/);
  const expiredRequests=await page.evaluate(()=>window.__queries.length);
  await page.locator('.nav-tabs [data-tab="history"]').click();await page.clock.runFor(31000);
  await page.locator('.nav-tabs [data-tab="setup"]').click();await page.clock.runFor(700);
  assert.equal(await panel.getByRole('button',{name:'重新登录',exact:true}).count(),1);
  assert.equal(await page.evaluate(()=>window.__queries.length),expiredRequests,"Known expired credentials do not trigger another automatic query after navigation");
  const beforeLogin=await page.evaluate(()=>window.__queries.length);
  await page.evaluate(()=>window.__authFailure=false);
  await panel.getByRole('button',{name:'重新登录',exact:true}).evaluate(button=>{button.click();button.click();});
  await page.clock.runFor(50);
  assert.equal(await panel.getByRole('button',{name:'登录中',exact:true}).isDisabled(),true);
  assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  await page.clock.runFor(500);
  await panel.getByRole('button',{name:'刷新场次状态'}).waitFor();
  assert.equal(await page.evaluate(()=>window.__authCalls.length),2);
  assert.equal(await page.evaluate(()=>window.__queries.length),beforeLogin+1);
  assert.equal(await page.evaluate(()=>window.__queries.at(-1).force),true);
  await page.evaluate(()=>{window.__authExpired=true;window.__authFailure=true;});
  await page.clock.runFor(5100);
  await panel.locator('[data-refresh-availability]').click();await page.clock.runFor(300);
  await panel.getByRole('button',{name:'重新登录',exact:true}).click();await page.clock.runFor(300);
  assert.match(await panel.innerText(),/重新登录/);
  assert.match(await slot.locator('.shell-select-value').innerText(),/已满/);
  await page.evaluate(()=>{window.__authFailure=false;window.__holdLogin=true;});
  const beforeCancel=await page.evaluate(()=>window.__queries.length);
  await panel.getByRole('button',{name:'重新登录',exact:true}).click();await page.clock.runFor(20);
  await panel.getByRole('button',{name:'取消',exact:true}).click();
  await page.evaluate(()=>window.__finishLogin());await page.clock.runFor(300);
  assert.equal(await page.evaluate(()=>window.__cancelCalls.length),1);
  assert.equal(await page.evaluate(()=>window.__queries.length),beforeCancel);
  await panel.getByRole('button',{name:'重新登录',exact:true}).click();await page.clock.runFor(20);
  await page.locator('.nav-tabs [data-tab="history"]').click();
  await page.evaluate(()=>window.__finishLogin());await page.clock.runFor(300);
  assert.equal(await page.evaluate(()=>window.__cancelCalls.length),2);
  assert.equal(await page.evaluate(()=>window.__queries.length),beforeCancel);
  await page.evaluate(()=>{window.__holdLogin=false;window.__authExpired=false;});
  await page.locator('.nav-tabs [data-tab="setup"]').click();
  await page.locator('[data-date="2026-10-24"]').click();await page.clock.runFor(700);
  assert.equal(await page.evaluate(()=>window.__bookingCalls.length),0);
  const count=await page.evaluate(()=>window.__queries.length);
  await page.locator('[data-date="2026-10-25"]').click();
  await page.clock.runFor(500);
  assert.equal(await panel.isHidden(),true);
  assert.equal(await slot.locator('[data-value="13:00|14:00"]').isEnabled(),true);
  assert.equal(await court.locator('.shell-select-trigger').isEnabled(),true);
  assert.equal(await page.evaluate(()=>window.__queries.length),count);
  await page.locator('[data-date="2026-10-23"]').click();
  await page.clock.runFor(260);
  await page.locator('[data-date="2026-10-25"]').click();
  await page.clock.runFor(500);
  assert.equal(await panel.isHidden(),true);
  await page.locator('.nav-tabs [data-tab="history"]').click();
  assert.equal(await page.locator('.run-card-body .run-stages').count(),0,"Collapsed history does not render step DOM");
  await page.locator('[data-toggle-run="0"]').click();
  const labels=await page.locator(".run-flow-label").allTextContents();
  for (const text of ["登录","已有预约","验证码","实时场次","发送预约","匹配订单"]) assert.ok(labels.some(label=>label.includes(text)));
  assert.ok(labels.every(label=>label!=="步骤"));
  const columns=await page.locator('.run-stage-row').evaluateAll(rows=>rows.map(row=>({time:row.querySelector('.run-stage-start').getBoundingClientRect().right,duration:row.querySelector('.run-stage-elapsed').getBoundingClientRect().right})));
  assert.ok(columns.every(col=>Math.abs(col.time-columns[0].time)<1 && Math.abs(col.duration-columns[0].duration)<1));
  assert.equal(labels.filter(label=>label.includes("验证登录")).length,1);
  assert.ok(!labels.some(label=>label.includes("识别当前登录")));
  const authDetails=page.locator('.run-stages > li').filter({has:page.locator('.run-flow-label',{hasText:'验证登录'})}).locator('details');
  assert.equal(await authDetails.getAttribute('open'),null);
  await authDetails.locator('summary').click();
  assert.match(await authDetails.innerText(),/识别当前登录页面/);
  await authDetails.locator('summary').click();
  assert.match(await page.locator(".run-card-body").innerText(),/翔安校区羽毛球/);
  await page.screenshot({path:resolve(artifacts,"history.png"),fullPage:true});
  await page.setViewportSize({width:550,height:900});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
  await page.evaluate(()=>window.__holdHistory=true);
  await page.locator('.nav-tabs [data-tab="live"]').click();
  await page.locator('.nav-tabs [data-tab="history"]').click();
  await page.locator('[data-toggle-run="0"]').waitFor();
  assert.equal(await page.locator('.run-card-body .run-stages').count(),0);
  assert.doesNotMatch(await page.locator('#panel').innerText(),/正在读取运行记录/);
  const requests=await page.evaluate(()=>window.__historyRequests);
  await page.locator('.nav-tabs [data-tab="live"]').click();
  await page.locator('.nav-tabs [data-tab="history"]').click();
  await page.locator('[data-toggle-run="0"]').waitFor();
  assert.equal(await page.evaluate(()=>window.__historyRequests),requests,"Reentry shares an in-flight refresh");
  await page.locator('[data-toggle-run="0"]').click();
  assert.ok(await page.locator('.run-stage-row').count()>0,"Cached details open before the refresh resolves");
  await page.locator('#history_filter_clear').click();
  await page.locator('[data-confirm-ok]').click();
  assert.equal(await page.locator('.run-card').count(),0);
  await page.evaluate(()=>window.__finishHistory());
  await page.clock.runFor(100);
  assert.equal(await page.locator('.run-card').count(),0,"Old refresh cannot resurrect cleared history");
  console.log("History/availability browser checks passed: admin vs full, cache stamp, refresh, date range, stale reply, compact selectors, court locks, quota, keyboard and history stages");
} finally { await browser.close(); }
