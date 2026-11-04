// Synthetic equivalents of a verified gym booking and an exhausted badminton fallback chain.
import assert from 'node:assert/strict';
import {mkdirSync,readFileSync} from 'node:fs';
import {createServer} from 'vite';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const server=await createServer({server:{host:'127.0.0.1',port:1451,strictPort:false}});await server.listen();
const browser=await launchBrowser();mkdirSync('artifacts/booking-round-results',{recursive:true});
try {
 const data=shellData(),{sports}=JSON.parse(readFileSync('src/catalog/venues.json','utf8'));
 const badminton=sports.find(s=>s.label==='羽毛球');
 data.plans.rules[1]={...data.plans.rules[1],sport:badminton.id,venue:badminton.venues[0].id,start:'09:00',end:'10:00'};
 const tasks=data.plans.rules.map(r=>{
  const s=sports.find(s=>s.id===r.sport),v=s.venues.find(v=>v.id===r.venue);
  return {task_id:`plan-${r.id}-2026-11-05`,plan_id:r.id,sport:s.ocr_label,venue:v.ocr_label,target_date:'2026-11-05',fire_at:'2026-11-04T07:00:00+08:00',target_time_start:r.start,target_time_end:r.end,executor:'http',allow_time_fallback:true};
 });
 const target=t=>({sport:t.sport,venue:t.venue,date:t.target_date,time:t.target_time_start,end:t.target_time_end});
 const events=(t,i,failed)=>[
  {task_id:t.task_id,run_id:`round-${i}`,ts:`2026-11-04T07:0${i}:00+08:00`,action:'spawn',result:'http_started',http:{target:target(t)}},
  {task_id:t.task_id,run_id:`round-${i}`,ts:`2026-11-04T07:0${i}:10+08:00`,action:'http_phase',result:'checking_slot',http:{phase:'checking_slot',target:target(t)}},
  {task_id:t.task_id,run_id:`round-${i}`,ts:`2026-11-04T07:0${i}:35+08:00`,action:failed?'fail':'done',result:failed?'FALLBACKS_EXHAUSTED':'http_verified',http:failed?{target:target(t)}:{verified:true,target:target(t),date:t.target_date,time:`${t.target_time_start}-${t.target_time_end}`}},
 ];
 data.next=[{...data.next[0],date:'2026-11-06',fire_at:'2026-11-05T07:00:00+08:00'}];
 data.events=[...events(tasks[0],0,false),...events(tasks[1],1,true)];
 data.task={...tasks[1],task_id:'new-draft',plan_id:undefined,target_date:'2026-11-06'};
 data.live={running:false,operation:'booking',recent_events:events(tasks[1],1,true),last_event:data.events.at(-1)};
 data.plans.attempts=tasks.map((task,i)=>({task,sport:data.plans.rules[i].sport,account:'a'.repeat(64),status:i?'failed':'succeeded',message:i?'FALLBACKS_EXHAUSTED':'http_verified'}));
 // Other dates, accounts and opening times must not count toward this round.
 data.plans.attempts.push(...[
  {...data.plans.attempts[0],task:{...tasks[0],task_id:'old',target_date:'2026-11-02'}},
  {...data.plans.attempts[0],task:{...tasks[0],task_id:'other-account'},account:'b'.repeat(64)},
  {...data.plans.attempts[0],task:{...tasks[0],task_id:'other-time',fire_at:'2026-11-04T07:01:00+08:00'}},
 ]);
 const context=await browser.newContext({viewport:{width:1100,height:760},reducedMotion:'reduce'});
 await context.addInitScript(installShellFixture,data);
 const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.clock.install({time:new Date('2026-11-04T10:00:00+08:00')});
 await page.goto(`http://127.0.0.1:${server.httpServer.address().port}`);
 await page.locator('.nav-primary [data-tab="live"]').click();
 await page.locator('.booking-round h3').filter({hasText:'部分约到'}).waitFor();
 assert.equal(await page.locator('.round-item').count(),2);
 assert.match(await page.locator('.round-item').first().innerText(),/健身房.*16:30–18:00.*已约到/s);
 assert.match(await page.locator('.round-item').nth(1).innerText(),/羽毛球.*09:00–10:00.*未约到/s);
 assert.equal(await page.locator('#live_execution').innerText(),'');
 assert.equal(await page.locator('.round-item[open]').count(),0);
 const reads=await page.evaluate(()=>window.__calls.filter(c=>c.command==='list_recent_events').length);
 for(const [theme,width,height] of [['light',1100,760],['dark',960,600]]) {
  await page.setViewportSize({width,height});await page.evaluate(t=>document.documentElement.dataset.shellTheme=t,theme);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.screenshot({path:`artifacts/booking-round-results/mixed-${theme}-${width}.png`});
  if(theme==='light') await page.locator('.booking-round').screenshot({path:'artifacts/booking-round-results/result-card-light.png'});
 }
 await page.evaluate(()=>{
  const original=window.__TAURI_INTERNALS__.invoke;
  window.__failRoundHistory=true;
  window.__TAURI_INTERNALS__.invoke=async(command,args)=>{
   if(command==='list_recent_events' && window.__failRoundHistory) throw Error('fixture local read failed');
   return original(command,args);
  };
 });
 await page.locator('.round-item > summary').first().click();
 await page.getByRole('status').filter({hasText:'执行记录读取失败'}).waitFor();
 assert.match(await page.locator('.booking-round h3').innerText(),/部分约到/);
 assert.equal(await page.locator('.round-item-result[data-result="succeeded"]').count(),1,'trace failure never downgrades a saved success');
 await page.locator('.round-item > summary').first().click();
 await page.evaluate(()=>window.__failRoundHistory=false);
 await page.locator('.round-item > summary').first().focus();await page.keyboard.press('Enter');
 await page.locator('.round-item .booking-execution.success').waitFor();
 assert.match(await page.locator('.booking-target').innerText(),/健身房.*2026-11-05/s);
 assert.doesNotMatch(await page.locator('.booking-target').innerText(),/2026-11-06/,'draft never fills an old result');
 assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==='list_recent_events').length),reads+1);
 await page.locator('.round-item > summary').nth(1).click();
 await page.locator('.round-item .booking-execution.failed').waitFor();
 assert.equal(await page.locator('.round-item[open]').count(),1);
 await page.locator('.booking-failure-details summary').click();
 await page.locator('.booking-details summary').click();
 await page.evaluate(()=>window.__emit('plans-changed'));
 await page.clock.runFor(250);
 assert.equal(await page.locator('.booking-failure-details').getAttribute('open'),'');
 assert.equal(await page.locator('.booking-details').getAttribute('open'),'');
 await page.screenshot({path:'artifacts/booking-round-results/failure-expanded-dark.png'});
 // Active runs still use their own full progress; completed siblings cannot donate steps.
 await page.evaluate(()=>{
  const e={...window.__live.last_event,action:'http_phase',result:'authenticating',http:{phase:'authenticating'}};
  window.__live={...window.__live,task:window.__plans.attempts[1].task,running:true,last_event:e,recent_events:[e]};window.__emit('run-changed');
 });
 await page.locator('#live_execution .booking-execution.running').waitFor();
 assert.equal(await page.locator('.booking-round').count(),0);
 assert.equal(await page.locator('.booking-track .done').count(),0);
 assert.equal(await page.evaluate(()=>window.__calls.some(c=>['arm_hang','start_http_run','cancel_my_order','launch_xmu_miniprogram'].includes(c.command))),false);
 for (const [statuses,title] of [
  [['succeeded','succeeded'],'全部约到'],
  [['failed','failed'],'本轮未约到'],
  [['succeeded','unconfirmed'],'有预约待确认'],
  [['succeeded','queued'],'本轮尚未结束'],
 ]) {
  await page.evaluate(statuses=>{
   window.__plans.attempts.slice(0,2).forEach((a,i)=>a.status=statuses[i]);
   const e={...window.__live.last_event,action:'fail',result:'RESULT_UNCONFIRMED'};
   window.__live={...window.__live,running:false,last_event:e,recent_events:[e]};
   window.__emit('plans-changed');window.__emit('run-changed');
  },statuses);
  await page.locator('.booking-round h3').filter({hasText:title}).waitFor();
 }
 if(!await page.locator('.round-item').nth(1).evaluate(el=>el.open)) await page.locator('.round-item > summary').nth(1).click();
 await page.locator('.round-item[open] .booking-execution').waitFor();
 await page.evaluate(()=>{
  window.__plans.attempts[1].status='succeeded';
  const t=window.__plans.attempts[1].task;
  const e={...window.__live.last_event,action:'done',result:'http_verified',ts:'2026-11-04T07:03:00+08:00',http:{verified:true,target:{sport:t.sport,venue:t.venue,date:t.target_date,time:t.target_time_start,end:t.target_time_end}}};
  window.__live={...window.__live,last_event:e,recent_events:[e]};
  window.__emit('plans-changed');window.__emit('run-changed');
 });
 await page.locator('.booking-round h3').filter({hasText:'全部约到'}).waitFor();
 await page.locator('.round-item[open] .booking-execution.success').waitFor();
 assert.equal(await page.locator('.round-item[open] .booking-execution.unconfirmed').count(),0,'open details refresh when a saved outcome changes');
 assert.deepEqual(errors,[]);
 await context.close();
 console.log('Round results passed: mixed success/failure, exact round/account/date isolation, lazy local traces, keyboard, disclosure state, active-run isolation and light/dark minimum window');
} finally {await browser.close();await server.close();}
