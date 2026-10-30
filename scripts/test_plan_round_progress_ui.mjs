import assert from 'node:assert/strict';
import {mkdirSync,readFileSync} from 'node:fs';
import {createServer} from 'vite';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const server=await createServer({server:{host:'127.0.0.1',port:1448,strictPort:false}});await server.listen();
const browser=await launchBrowser();mkdirSync('artifacts/plan-round-progress',{recursive:true});
try {
 const data=shellData();
 const {sports}=JSON.parse(readFileSync('src/catalog/venues.json','utf8'));
 data.plans.rules.forEach(r=>{r.first_date='2026-11-01';r.weekdays=[1,2,3,4,5,6,7];});
 data.next=data.plans.rules.map(r=>({id:r.id,date:'2026-11-01',fire_at:'2026-10-31T07:00:00+08:00'}));
 const tasks=data.plans.rules.map(r=>{
  const sport=sports.find(s=>s.id===r.sport),venue=sport.venues.find(v=>v.id===r.venue);
  return {task_id:`plan-${r.id}-2026-11-01`,plan_id:r.id,sport:sport.ocr_label,venue:venue.ocr_label,target_date:'2026-11-01',fire_at:'2026-10-31T07:00:00+08:00',target_time_start:r.start,target_time_end:r.end,executor:'http',revision:0,allow_time_fallback:false,time_fallback_order:'down_then_up'};
 });
 const initial={task_id:tasks[0].task_id,run_id:'round-run-a',ts:'2026-10-31T07:00:00+08:00',action:'http_phase',result:'authenticating',http:{phase:'authenticating'}};
 data.task=tasks[0];data.live={task:tasks[0],running:true,operation:'booking',recent_events:[initial],last_event:initial};
 data.plans.attempts=tasks.map((task,i)=>({task,sport:data.plans.rules[i].sport,account:data.plans.rules[i].account,status:i?'queued':'running',message:''}));
 const context=await browser.newContext({viewport:{width:1100,height:760},reducedMotion:'reduce'});
 await context.addInitScript(installShellFixture,data);
 const page=await context.newPage();await page.clock.install({time:new Date('2026-10-31T07:00:01+08:00')});
 await page.goto('http://127.0.0.1:'+server.httpServer.address().port);
 await page.locator('.nav-primary [data-tab="live"]').click();await page.locator('.booking-execution.running').waitFor();
 const active=()=>page.locator('.booking-track [aria-current="step"]').innerText();
 const phase=async (name,expected)=>{
  const previous=await page.locator('.booking-current-detail').first().innerText();
  await page.evaluate(name=>{
   const t=window.__live.task;
   const target={sport:t.sport,venue:t.venue,date:t.target_date,time:t.target_time_start,end:t.target_time_end};
   const event={task_id:t.task_id,run_id:window.__live.last_event.run_id,ts:new Date(Date.now()).toISOString(),action:'http_phase',result:name,http:{phase:name,target}};
   window.__live.last_event=event;window.__live.recent_events.push(event);window.__emit('run-changed');
  },name);
  await page.waitForFunction(text=>document.querySelector('.booking-current-detail').textContent!==text,previous);
  assert.match(await active(),new RegExp(expected));
 };
 assert.equal(await page.locator('.plan-queue-row').count(),1,'only the second frozen task remains in the waiting list');
 assert.match(await page.locator('.plan-queue-row').innerText(),new RegExp(tasks[1].sport));
 for(const name of ['h5_entry_reused','local_eligibility','warming','reading_entry','authenticating','checking_orders']) await phase(name,'准备检查');
 await phase('captcha','验证码');await phase('captcha_recognized','验证码');
 await phase('resolving_venue','实时场次');await phase('querying_slots','实时场次');
 await page.screenshot({path:'artifacts/plan-round-progress/first-running.png'});
 await phase('checking_slot','预约校验');await phase('server_precheck','预约校验');await phase('slot_ready','预约校验');
 await phase('submitting','提交预约');await phase('verifying_order','查单确认');
 await page.evaluate(()=>{
  const event={...window.__live.last_event,action:'done',result:'http_verified',http:{...window.__live.last_event.http,verified:true}};
  window.__plans.attempts[0].status='succeeded';window.__live={...window.__live,running:false,last_event:event,recent_events:[...window.__live.recent_events,event]};window.__emit('plans-changed');window.__emit('run-changed');
 });
 await page.locator('.booking-execution.success').waitFor();
 await page.evaluate(task=>{
  const event={task_id:task.task_id,run_id:'round-run-b',ts:new Date(Date.now()).toISOString(),action:'http_phase',result:'authenticating',http:{phase:'authenticating',target:{sport:task.sport,venue:task.venue,date:task.target_date,time:task.target_time_start,end:task.target_time_end}}};
  window.__plans.attempts[1].status='running';window.__live={...window.__live,task,running:true,last_event:event,recent_events:[...window.__live.recent_events,event]};window.__emit('plans-changed');window.__emit('run-changed');
 },tasks[1]);
 await page.locator('.booking-execution.running').waitFor();
 assert.match(await active(),/准备检查/);
 assert.equal(await page.locator('.booking-track li.done').count(),0,'second run does not inherit completed steps from the first');
 assert.match(await page.locator('.booking-target').innerText(),new RegExp(tasks[1].sport));
 assert.doesNotMatch(await page.locator('.booking-target').innerText(),new RegExp(tasks[0].sport),'target belongs to the second frozen task');
 await phase('warming','准备检查');await phase('captcha','验证码');await phase('querying_slots','实时场次');await phase('server_precheck','预约校验');await phase('submitting','提交预约');
 await phase('captcha_retry','验证码');
 for(const name of ['实时场次','预约校验','提交预约','查单确认']) assert.equal(await page.locator('.booking-track li.done').filter({hasText:name}).count(),0,'captcha retry clears previous slot/submit checkmarks');
 for(const theme of ['light','dark']) {
  await page.setViewportSize({width:960,height:760});await page.evaluate(t=>document.documentElement.dataset.shellTheme=t,theme);
  await page.screenshot({path:`artifacts/plan-round-progress/second-retry-${theme}.png`});
 }
 await phase('captcha','验证码');await phase('querying_slots','实时场次');await phase('slot_ready','预约校验');await phase('submitting','提交预约');await phase('verifying_order','查单确认');
 await page.evaluate(()=>{
  const event={...window.__live.last_event,action:'done',result:'http_verified',http:{...window.__live.last_event.http,verified:true}};
  window.__plans.attempts[1].status='succeeded';window.__next.forEach(n=>{n.date='2026-11-02';n.fire_at='2026-11-01T07:00:00+08:00';});
  window.__live={...window.__live,running:false,last_event:event,recent_events:[...window.__live.recent_events,event]};window.__emit('plans-changed');window.__emit('run-changed');
 });
 await page.locator('.booking-execution.success').waitFor();
 assert.match(await page.locator('.booking-target').innerText(),new RegExp(tasks[1].sport));
 assert.doesNotMatch(await page.locator('.booking-target').innerText(),new RegExp(tasks[0].sport),'final result remains attached to the second frozen task');
 assert.deepEqual(await page.evaluate(()=>window.__plans.attempts.map(a=>a.status)),['succeeded','succeeded']);
 await page.screenshot({path:'artifacts/plan-round-progress/second-result-dark.png'});
 assert.equal(await page.evaluate(()=>window.__calls.some(c=>['arm_hang','start_http_run','cancel_my_order','launch_xmu_miniprogram'].includes(c.command))),false);
 console.log('Plan-round UI passed: two frozen targets in priority order, stable prepare stage, independent run progress, real captcha retry reset, light/dark minimum window; synthetic host only');
} finally {await browser.close();await server.close();}
