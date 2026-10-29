// Full frontend, deterministic clock and an offline host boundary. No school access.
import assert from "node:assert/strict";
import {mkdirSync} from "node:fs";
import {createRequire} from "node:module";
import {join} from "node:path";
import {installShellFixture} from "./fixtures/tauri-shell.js";
import {shellData} from "./fixtures/shell-data.mjs";
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE,".cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright"));
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,"Set XMU_TEST_BASE_URL to an isolated running Vite fixture server");
const output="artifacts/ui-refactor";mkdirSync(output,{recursive:true});
const browser=await chromium.launch({headless:true,channel:"msedge"});
try {
  const context=await browser.newContext({viewport:{width:960,height:720}});
  const data=shellData();
  await context.addInitScript(installShellFixture,data);
  const page=await context.newPage(),errors=[];
  page.on("pageerror",error=>errors.push(error.message));
  await page.clock.install({time:new Date("2026-10-30T12:00:00+08:00")});
  await page.goto(base);
  await page.locator('.plan-card').first().waitFor();
  assert.equal(await page.locator('.nav-tabs [aria-current="page"]').getAttribute('data-tab'),'plans');
  assert.equal(await page.locator('.booking-modes').count(),0);
  assert.match(await page.locator('#plans_next').innerText(),/明天 07:00/);
  assert.match(await page.locator('#plans_exercise').innerText(),/11月1日/);
  assert.equal(await page.locator('.plan-card').count(),2);
  await page.screenshot({path:`${output}/plans-light.png`});
  await page.getByRole('button',{name:'运行',exact:true}).click();
  await page.locator('.plan-queue-row').waitFor();
  assert.equal(await page.getByText('尚未配置任务',{exact:true}).isVisible(),false);
  assert.equal(await page.locator('#live_configuration').getAttribute('data-tab'),'plans');
  await page.screenshot({path:`${output}/live-plans-light.png`});
  await page.locator('#live_configuration').click();
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
  await page.locator('#live_configuration').click();
  await page.locator('.plans-page').waitFor();
  // Reset only the synthetic host; no actual booking or system operation was invoked.
  await page.evaluate(plans=>{
    window.__plans=plans;window.__live={task:null,armed:false,running:false,recent_events:[],last_event:null,fire_at:null};window.__emit('run-changed');window.__emit('plans-changed');
  },data.plans);
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    for(const size of [{width:960,height:720},{width:480,height:520}]) {
      await page.setViewportSize(size);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
      const card=await page.locator('.plan-card').first().boundingBox();
      await page.screenshot({path:`${output}/plans-${theme}-${size.width}.png`});
      assert.ok(card.y<400,`rules should start before the lower portion of a narrow screen: ${JSON.stringify({size,card})}`);
      if(size.width===480) {
        assert.equal(await page.locator('.my-orders-body').isVisible(),false);
        await page.locator('.my-orders-toggle').click();
        assert.equal(await page.locator('.my-orders-body').isVisible(),true);
        assert.equal(await page.locator('.my-order').count(),1);
        await page.locator('.my-orders-toggle').click();
      }
    }
  }
  await page.setViewportSize({width:960,height:720});
  await page.locator('.plan-more').first().locator('summary').focus();
  await page.keyboard.press('Enter');await page.keyboard.press('Escape');
  assert.equal(await page.locator('.plan-more').first().getAttribute('open'),null);
  assert.equal(await page.locator('.plan-more').first().locator('summary').evaluate(el=>el===document.activeElement),true);
  // Read failures retain the page and expose a working retry.
  await page.evaluate(()=>{window.__plansError='模拟计划读取失败';window.__emit('plans-changed');});
  await page.getByRole('button',{name:'重新读取计划',exact:true}).waitFor();
  await page.evaluate(()=>window.__plansError='');
  await page.getByRole('button',{name:'重新读取计划',exact:true}).click();
  await page.locator('#plans_retry').waitFor({state:'hidden'});
  await page.evaluate(()=>{window.__plans={rules:[],attempts:[],paused:false};window.__next=[];window.__emit('plans-changed');});
  await page.getByRole('button',{name:'添加第一条计划',exact:true}).waitFor();
  await page.getByRole('button',{name:'运行',exact:true}).click();
  await page.getByText('尚未配置任务',{exact:true}).waitFor();
  assert.equal(await page.locator('#live_configuration').getAttribute('data-tab'),'setup');
  await page.locator('#live_configuration').click();
  await page.locator('.setup-page').waitFor();
  assert.equal(await page.locator('.nav-tabs [aria-current="page"]').getAttribute('data-tab'),'setup');
  assert.deepEqual(errors,[]);
  assert.equal(await page.evaluate(()=>window.__calls.some(c=>['start_http_run','arm_hang','launch_xmu_miniprogram','set_startup','cancel_my_order'].includes(c.command))),false);
  console.log('Shell UI passed: direct navigation, pending outcomes, frozen targets, conflicts, empty/error, order expansion, keyboard and light/dark/narrow; no live operations');
} finally {await browser.close();}
