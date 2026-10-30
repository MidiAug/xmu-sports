import assert from 'node:assert/strict';
import {mkdirSync,readFileSync} from 'node:fs';
import {createServer} from 'vite';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const server=await createServer({server:{host:'127.0.0.1',port:1442,strictPort:false}});await server.listen();
const browser=await launchBrowser();mkdirSync('artifacts/progress-unified',{recursive:true});
try {
 const data=shellData();
 const task={task_id:'single',sport:'乒乓球',venue:'翔安校区乒乓球室',target_date:'2026-10-31',target_time_start:'09:00',target_time_end:'10:30',fire_at:'2026-10-30T21:00:00+08:00',executor:'http',revision:0};
 const events=[{ts:'2026-10-30T11:00:00+08:00',task_id:'auth',action:'spawn',result:'http_auth'},{ts:'2026-10-30T11:00:25+08:00',task_id:'auth',action:'done',result:'http_auth_ready'}];
 const context=await browser.newContext({viewport:{width:1100,height:760},reducedMotion:'reduce'});
 await context.addInitScript(installShellFixture,{...data,task,live:{task,armed:true,fire_at:task.fire_at,recent_events:events,last_event:events[1],operation:'auth'}});
 const page=await context.newPage();await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});await page.goto(`http://127.0.0.1:${server.httpServer.address().port}`);
 assert.equal(await page.locator('.nav-primary [data-tab="plans"]').innerText(),'计划');
 // New-plan identity verification is shared by opening and saving this editor.
 await page.locator('#plan_add').click();
 const {sports}=JSON.parse(readFileSync('src/catalog/venues.json','utf8'));
 const free=sports.find(s=>s.enabled!==false&&!data.plans.rules.some(r=>r.sport===s.id));
 await page.locator('#plan_sport .shell-select-trigger').click();await page.locator(`#plan_sport [data-value="${free.id}"]`).click();
 await page.locator('#plan_editor [type="submit"]').click();await page.locator('#plan_editor').waitFor({state:'hidden'});
 assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==='booking_plan_account').length),1,'one verified identity request per new editor/save');
 await page.locator('.nav-primary [data-tab="live"]').click();await page.locator('.plan-queue-row').first().waitFor();
 assert.equal(await page.locator('.plan-queue-row').count(),2,'single and recurring share one queue');
 assert.match(await page.locator('.plan-queue-row').first().innerText(),/乒乓球/);
 assert.equal(await page.locator('.live-auxiliary .booking-execution').isVisible(),false);
 assert.equal(await page.locator('#live_single_context').isVisible(),false,'armed task is not a draft');
 assert.equal(await page.locator('#live_remain').count(),0,'no second countdown card');
 for(const size of [{width:1100,height:760},{width:960,height:600}]) {
  await page.setViewportSize(size);
  for(const theme of ['light','dark']) {
   await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
   const left=await page.locator('.live-primary').boundingBox(),right=await page.locator('.live-secondary').boundingBox();
   assert.ok(right.x>=left.x+left.width,'upcoming and result areas share a desktop row');
   await page.screenshot({path:`artifacts/progress-unified/combined-${theme}-${size.width}.png`});
  }
 }
 await page.evaluate(()=>{window.__plans.paused=true;window.__emit('plans-changed');});
 await page.waitForFunction(()=>document.querySelectorAll('.plan-queue-row').length===1);
 assert.match(await page.locator('.plan-queue-row').innerText(),/单次/);
 await page.evaluate(()=>{window.__live.armed=false;window.__emit('run-changed');});
 await page.locator('#live_single_context').waitFor({state:'visible'});
 assert.match(await page.locator('#live_single_context').innerText(),/未启动/);
 assert.equal(await page.locator('.live-primary #live_single_context').count(),1,'draft belongs to the shared arrangement column');
 assert.equal(await page.locator('.plan-queue-items').count(),0,'paused empty list is removed');
 assert.equal(await page.locator('.plan-queue-actions').evaluate(el=>getComputedStyle(el).display),'none');
 await page.screenshot({path:'artifacts/progress-unified/paused-draft-dark.png'});
 await page.evaluate(()=>{window.__plans.attempts=[{task:{...window.__task,task_id:'completed'},sport:'pingpong',account:'a'.repeat(64),status:'succeeded',message:''}];window.__emit('plans-changed');});
 await page.getByText('已约到',{exact:true}).waitFor();
 await page.screenshot({path:'artifacts/progress-unified/results-dark.png'});
 await page.evaluate(()=>{
  const event={task_id:'active',ts:'2026-10-30T12:01:00+08:00',action:'http_phase',result:'querying_slots',http:{phase:'querying_slots'}};
  window.__live={...window.__live,task:{...window.__task,task_id:'active'},running:true,armed:false,operation:'booking',recent_events:[event],last_event:event};window.__emit('run-changed');
 });
 await page.locator('.booking-execution.running').waitFor();
 await page.locator('.live-results').waitFor();
 assert.equal(await page.locator('#live_single_context').isVisible(),false);
 await page.screenshot({path:'artifacts/progress-unified/running-dark.png'});
 await page.evaluate(()=>{
  const event={task_id:'launch-failed',ts:'2026-10-30T12:02:00+08:00',action:'fail',result:'LOGIN_CONFIRM_REQUIRED',http:{reason:'MINIPROGRAM_NOT_VISIBLE',message:'旧的未显示提示'}};
  window.__live={...window.__live,running:false,armed:false,operation:'auth',last_event:event,recent_events:[event]};window.__emit('run-changed');
 });
 await page.waitForFunction(()=>document.querySelector('.booking-execution.failed'));
 await page.locator('.booking-head-copy h2').filter({hasText:'厦大体育小程序打开失败'}).waitFor();
 assert.match(await page.locator('.booking-current-detail').innerText(),/更多.*添加到桌面.*重试/);
 assert.equal(await page.locator('#open_booking').count(),0,'do not send the user into a failing launch retry loop');
 await page.screenshot({path:'artifacts/progress-unified/launch-failed-dark.png'});
 assert.equal(await page.evaluate(()=>window.__calls.some(c=>['arm_hang','start_http_run','cancel_my_order'].includes(c.command))),false);
 console.log('Unified progress passed: single/plan chronological queue, pause, draft distinction, collapsed login, compact navigation and one identity request for create/save');
} finally {await browser.close();await server.close();}
