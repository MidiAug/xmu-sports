// Full frontend, deterministic clock and an offline host boundary. No school access.
import assert from "node:assert/strict";
import {mkdirSync} from "node:fs";
import {launchBrowser} from "./lib/browser.mjs";
import {installShellFixture} from "./fixtures/tauri-shell.js";
import {shellData} from "./fixtures/shell-data.mjs";
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,"Set XMU_TEST_BASE_URL to an isolated running Vite fixture server");
const output="artifacts/ui-refactor";mkdirSync(output,{recursive:true});
const browser=await launchBrowser();
try {
  const context=await browser.newContext({viewport:{width:960,height:720}});
  const data=shellData();
  await context.addInitScript(installShellFixture,data);
  const page=await context.newPage(),errors=[];
  page.on("pageerror",error=>errors.push(error.message));
  await page.clock.install({time:new Date("2026-10-30T12:00:00+08:00")});
  await page.goto(base);
  await page.locator('.plan-card').first().waitFor();
  const windowCalls=command=>page.evaluate(command=>window.__calls.filter(call=>call.command===command).length,command);
  const dragCommand='plugin:window|start_dragging';
  for(const selector of ['.app-header','.nav-brand','.brand-mark','.nav-brand h1','.nav-primary','.header-drag-space','.header-actions']) {
    const before=await windowCalls(dragCommand);
    await page.locator(selector).dispatchEvent('mousedown',{button:0,detail:1});
    assert.equal(await windowCalls(dragCommand),before+1,`non-interactive header region must drag: ${selector}`);
  }
  const noDrag=await windowCalls(dragCommand);
  for(const selector of ['.nav-primary [data-tab="plans"] span','.my-orders-toggle span:first-of-type','.titlebar-right svg','.header-settings svg']) {
    await page.locator(selector).first().dispatchEvent('mousedown',{button:0,detail:1});
  }
  await page.locator('.header-drag-space').dispatchEvent('mousedown',{button:2,detail:1});
  assert.equal(await windowCalls(dragCommand),noDrag,'controls and secondary mouse button must not start dragging');
  const maximize=await windowCalls('plugin:window|toggle_maximize');
  await page.locator('.header-drag-space').dblclick();
  assert.equal(await windowCalls('plugin:window|toggle_maximize'),maximize+1,'double clicking header space toggles maximize once');
  await page.locator('.my-orders-toggle').click();
  const popupDrag=await windowCalls(dragCommand);
  await page.locator('.my-orders-popup h2').dispatchEvent('mousedown',{button:0,detail:1});
  assert.equal(await windowCalls(dragCommand),popupDrag,'reading order popup content must not drag the window');
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.app-header [data-tab="settings"]').count(),1);
  assert.equal(await page.locator('.app-footer,.nav-foot,.nav-version').count(),0,'shell has no footer or version display');
  assert.equal(await page.locator('.nav-tabs [aria-current="page"]').getAttribute('data-tab'),'plans');
  assert.equal(await page.locator('.booking-modes').count(),0);
  assert.match(await page.locator('#plans_next').innerText(),/明天 07:00/);
  assert.match(await page.locator('#plans_exercise').innerText(),/11月1日/);
  assert.equal(await page.locator('.plan-card').count(),2);
  const planReads=()=>page.evaluate(()=>window.__calls.filter(c=>c.command==='get_booking_plans').length);
  const firstReads=await planReads();
  await page.screenshot({path:`${output}/plans-light.png`});
  await page.locator('.content').evaluate(el=>el.scrollTop=el.scrollHeight);
  await page.getByRole('button',{name:'运行',exact:true}).click();
  await page.locator('.plan-queue-row').waitFor();
  assert.equal(await page.locator('.content').evaluate(el=>el.scrollTop),0,'changing pages starts at the new page heading');
  assert.equal(await page.getByText('尚未配置任务',{exact:true}).isVisible(),false);
  assert.equal(await page.locator('#live_configuration').isVisible(),false,'no duplicate management action beside the queue');
  assert.equal(await planReads(),firstReads,'navigation reuses the application plan snapshot');
  await page.screenshot({path:`${output}/live-plans-light.png`});
  await page.locator('.plan-queue-heading [data-tab="plans"]').click();
  await page.locator('.plans-page').waitFor();
  // A pending outcome is actionable on the first screen, and cannot be resumed past.
  await page.evaluate(()=>{
    const rule=window.__plans.rules[0];window.__plans.paused=true;
    window.__plans.attempts=[{sport:rule.sport,account:rule.account,status:'unconfirmed',message:'提交响应尚未确认，请核对已有预约',task:{task_id:'pending-fixture',plan_id:rule.id,sport:'健身房',venue:'翔安校区健身房',target_date:'2026-10-29',fire_at:'2026-10-28T07:00:00+08:00',target_time_start:rule.start,target_time_end:rule.end}}];
    window.__emit('plans-changed');
  });
  await page.locator('#plan_attention').waitFor({state:'visible'});
  assert.equal(await page.locator('#plans_pause').isDisabled(),true);
  const bounds=await page.locator('[data-resolve]').boundingBox();
  assert.ok(bounds.y>=0 && bounds.y+bounds.height<=720,'pending action must be on the first screen');
  await page.screenshot({path:`${output}/attention-light.png`});
  await page.evaluate(()=>{window.__plans.attempts[0].message='提交响应尚未确认，请先核对已有预约。'.repeat(12);window.__emit('plans-changed');});
  await page.waitForFunction(()=>document.querySelector('.plan-attention-details p').textContent.length>120);
  await page.setViewportSize({width:960,height:600});
  assert.equal(await page.locator('.plan-attention-details').getAttribute('open'),null);
  const narrowPending=await page.locator('[data-resolve]').boundingBox();
  assert.ok(narrowPending.y+narrowPending.height<=600,'long reasons must not push the pending action off screen');
  await page.screenshot({path:`${output}/attention-narrow.png`});
  await page.locator('.plan-attention-details summary').focus();await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.keyboard.press('Enter');
  await page.setViewportSize({width:960,height:720});
  await page.locator('[data-resolve]').click();
  await page.getByRole('button',{name:'已核对，跳过本项',exact:true}).click();
  await page.locator('#plan_attention').waitFor({state:'hidden'});
  assert.equal(await page.evaluate(()=>window.__plans.paused),true,'resolving does not automatically resume');
  await page.locator('#plans_pause').click();
  await page.getByText('自动计划已开启',{exact:true}).waitFor();
  // Single booking conflicts navigate to the exact matching rule, independent of remembered mode.
  await page.evaluate(()=>{
    window.__plans.rules[1].weekdays=[1,2,3,4,5,6,7];window.__plans.rules[1].first_date='2026-10-30';window.__emit('plans-changed');
  });
  await page.getByRole('button',{name:'单次预约',exact:true}).click();
  await page.locator('#task_plan_link').waitFor({state:'visible'});
  const dateBounds=await page.locator('.booking-date-option').evaluateAll(buttons=>buttons.map(button=>button.getBoundingClientRect().toJSON()));
  assert.equal(dateBounds.length,3);
  assert.equal(await page.locator('.booking-date-option.on').count(),0,'all planned dates have no selected option');
  assert.ok(dateBounds.every(box=>Math.abs(box.y-dateBounds[0].y)<1),'the three desktop dates must remain in one horizontal row');
  assert.equal(await page.locator('#task_plan_link').getAttribute('data-plan-id'),data.plans.rules[1].id);
  assert.equal(await page.locator('#setup_primary').isDisabled(),true);
  assert.equal(await page.locator('#setup_check').isDisabled(),false);
  await page.locator('#task_plan_link').click();
  await page.locator('.plans-page').waitFor();
  assert.equal(await page.locator(`[data-edit="${data.plans.rules[1].id}"]`).evaluate(el=>el===document.activeElement),true);
  // A frozen task still links to plans, even after its rule target is changed.
  await page.evaluate(()=>{
    const rule=window.__plans.rules[0];
    const task={task_id:'frozen-fixture',plan_id:rule.id,sport:'健身房',venue:'原场馆',target_date:'2026-11-01',fire_at:'2026-10-31T07:00:00+08:00',target_time_start:'09:00',target_time_end:'10:30',executor:'http'};
    window.__plans.attempts=[{task,sport:rule.sport,account:rule.account,status:'queued',message:''}];
    window.__live={...window.__live,task,armed:true,fire_at:task.fire_at};window.__emit('plans-changed');
  });
  await page.getByRole('button',{name:'运行',exact:true}).click();
  await page.locator('.plan-queue-row').waitFor();
  assert.match(await page.locator('.plan-queue-row').innerText(),/原场馆[\s\S]*09:00/);
  assert.equal(await page.locator('#live_configuration').getAttribute('data-tab'),'plans');
  assert.equal(await page.locator('#live_configuration').getAttribute('data-plan-id'),data.plans.rules[0].id);
  await page.evaluate(()=>{window.__live.running=true;window.__live.armed=false;window.__emit('run-changed');});
  await page.locator('[data-stop-round]').waitFor();
  await page.locator('[data-stop-round]').focus();
  await page.evaluate(()=>window.__queueStop=document.querySelector('[data-stop-round]'));
  const beforePoll=await planReads();
  await page.clock.runFor(11000);
  assert.equal(await planReads(),beforePoll,'live polling does not refetch plans or probe WeChat');
  assert.equal(await page.evaluate(()=>window.__queueStop===document.querySelector('[data-stop-round]')),true);
  assert.equal(await page.locator('[data-stop-round]').evaluate(el=>el===document.activeElement),true);
  await page.locator('#live_configuration').click();
  await page.locator('.plans-page').waitFor();
  // Reset only the synthetic host; no actual booking or system operation was invoked.
  await page.evaluate(plans=>{
    window.__plans=plans;window.__live={task:null,armed:false,running:false,recent_events:[],last_event:null,fire_at:null};window.__emit('run-changed');window.__emit('plans-changed');
  },data.plans);
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    for(const size of [{width:960,height:720},{width:960,height:600}]) {
      await page.setViewportSize(size);
      const header=await page.locator('.app-header').boundingBox();
      assert.equal(header.height,56,'header keeps a compact height at both desktop sizes');
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
      const card=await page.locator('.plan-card').first().boundingBox();
      await page.screenshot({path:`${output}/plans-${theme}-${size.width}.png`});
      assert.ok(card.y<400,`rules should start before the lower portion of a narrow screen: ${JSON.stringify({size,card})}`);
      {
        assert.equal(await page.locator('.my-orders-body').isVisible(),false);
        await page.locator('.my-orders-toggle').click();
        assert.equal(await page.locator('.my-orders-body').isVisible(),true);
        assert.equal(await page.locator('.my-order').count(),1);
        await page.keyboard.press('Escape');
        assert.equal(await page.locator('.my-orders-popup').isVisible(),false);
        assert.equal(await page.locator('.my-orders-toggle').evaluate(el=>el===document.activeElement),true);
      }
    }
  }
  await page.setViewportSize({width:960,height:720});
  await page.locator('.plan-more').first().locator('summary').focus();
  await page.keyboard.press('Enter');await page.keyboard.press('Escape');
  assert.equal(await page.locator('.plan-more').first().getAttribute('open'),null);
  assert.equal(await page.locator('.plan-more').first().locator('summary').evaluate(el=>el===document.activeElement),true);
  await page.evaluate(()=>{window.__plans.paused=true;window.__emit('plans-changed');});
  await page.getByText('自动计划已暂停',{exact:true}).waitFor();
  const midnightReads=await planReads();
  await page.clock.fastForward(12*3600*1000);
  await page.waitForFunction(()=>document.querySelector('#plans_next').textContent.includes('今天 07:00'));
  assert.ok(await planReads()>midnightReads,'paused plans refresh date previews at midnight without health events');
  // Read failures retain the page and expose a working retry.
  await page.evaluate(()=>{window.__plansError='模拟计划读取失败';window.__emit('plans-changed');});
  await page.getByRole('button',{name:'重新读取计划',exact:true}).waitFor();
  await page.evaluate(()=>window.__plansError='');
  await page.getByRole('button',{name:'重新读取计划',exact:true}).click();
  await page.locator('#plans_retry').waitFor({state:'hidden'});
  await page.evaluate(()=>{window.__plans={rules:[],attempts:[],paused:false};window.__next=[];window.__emit('plans-changed');});
  await page.getByRole('button',{name:'添加第一条计划',exact:true}).waitFor();
  assert.equal(await page.locator('.plans-summary').isVisible(),false);
  assert.equal(await page.locator('#plan_add').isVisible(),false);
  await page.clock.runFor(5000);
  for(const theme of ['light','dark']) {
    await page.setViewportSize({width:1100,height:760});
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    await page.screenshot({path:`artifacts/ui-refactor/plans-empty-${theme}.png`});
  }
  await page.setViewportSize({width:960,height:600});
  const emptyAction=await page.locator('#plan_example').boundingBox();
  assert.ok(emptyAction.y+emptyAction.height<=600,'first plan action fits the minimum window');
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.locator('#plan_example').focus();await page.keyboard.press('Enter');
  await page.locator('#plan_editor').waitFor({state:'visible'});
  await page.locator('#plan_cancel').click();
  await page.locator('#plan_example').waitFor({state:'visible'});
  await page.screenshot({path:'artifacts/ui-refactor/plans-empty-dark-narrow.png'});
  await page.getByRole('button',{name:'运行',exact:true}).click();
  await page.getByText('尚未配置任务',{exact:true}).waitFor();
  assert.equal(await page.locator('#live_configuration').getAttribute('data-tab'),'setup');
  assert.equal(await page.locator('#live_configuration').isVisible(),false);
  await page.getByRole('button',{name:'去配置任务',exact:true}).click();
  await page.locator('.setup-page').waitFor();
  assert.equal(await page.locator('.nav-tabs [aria-current="page"]').getAttribute('data-tab'),'setup');
  assert.deepEqual(errors,[]);
  assert.equal(await page.evaluate(()=>window.__calls.some(c=>['start_http_run','arm_hang','launch_xmu_miniprogram','set_startup','cancel_my_order'].includes(c.command))),false);
  console.log('Shell UI passed: direct navigation, pending outcomes, frozen targets, conflicts, empty/error, order expansion, keyboard and light/dark/narrow; no live operations');
} finally {await browser.close();}
