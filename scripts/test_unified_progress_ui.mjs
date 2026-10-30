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
 const primary=await page.locator('.live-plan-queue').boundingBox(),secondary=await page.locator('.live-secondary').boundingBox();
 assert.ok(Math.abs(primary.y-secondary.y)<1,'left and right cards start at the same height');
 assert.equal(await page.locator('.live-column-title').count(),0);
 assert.equal(await page.evaluate(()=>window.__live.armed),true,'creating a plan leaves the single timer armed');

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
 const manage=page.locator('.plan-queue-heading [data-tab="plans"]'),edit=page.locator('#live_draft_edit');
 const manageBox=await manage.boundingBox(),editBox=await edit.boundingBox();
 assert.equal(manageBox.width,editBox.width,'manage plan and edit booking have the same width');
 assert.equal(manageBox.height,editBox.height,'manage plan and edit booking have the same height');
 assert.equal(await manage.locator('svg').evaluate(el=>el.outerHTML),await edit.locator('svg').evaluate(el=>el.outerHTML),'both navigation controls use the same diagonal arrow');
 await page.screenshot({path:'artifacts/progress-unified/paused-draft-dark.png'});
 await page.evaluate(()=>{window.__plans.attempts=[{task:{...window.__task,task_id:'completed'},sport:'pingpong',account:'a'.repeat(64),status:'succeeded',message:''}];window.__emit('plans-changed');});
 await page.getByText('已约到',{exact:true}).waitFor();
 await page.screenshot({path:'artifacts/progress-unified/results-dark.png'});
 await page.evaluate(()=>{
  window.__pendingOrders=[{request_ref:'a'.repeat(64),can_recheck:true,target:{sport:'乒乓球',venue:'旧待确认场馆',date:'2026-10-31',time:'19:30',end:'21:00',court:'04'}}];
  window.__emit('run-changed');
 });
 await page.locator('#pending_order_checks .settings-group').waitFor();
 await page.evaluate(()=>window.__pendingNode=document.querySelector('#pending_order_checks .settings-group'));
 await page.evaluate(()=>{
  const event={task_id:'active',ts:'2026-10-30T12:01:00+08:00',action:'http_phase',result:'querying_slots',http:{phase:'querying_slots'}};
  window.__live={...window.__live,task:{...window.__task,task_id:'active'},running:true,armed:false,operation:'booking',recent_events:[event],last_event:event};window.__emit('run-changed');
 });
 await page.locator('.booking-execution.running').waitFor();
 assert.equal(await page.locator('.live-results').count(),0,'current progress replaces historical results while running');
 assert.equal(await page.locator('.live-secondary .booking-execution.running').count(),1,'execution occupies the right result column');
 assert.equal(await page.locator('.live-primary .booking-execution').count(),0,'execution does not create a wide primary card');
 assert.equal(await page.locator('#live_single_context').isVisible(),false);
 await page.screenshot({path:'artifacts/progress-unified/running-dark.png'});
 assert.equal(await page.locator('.live-secondary #pending_order_checks').count(),1);
 assert.equal(await page.locator('[data-order-recheck]').isDisabled(),true,'existing unresolved orders remain visible but cannot be queried during execution');
 await page.evaluate(()=>{
  window.__pendingOrders.push({request_ref:'b'.repeat(64),can_recheck:false,reason:'临时提交记录',target:{venue:'本次执行中的临时记录'}});
  window.__executionNode=document.querySelector('#live_execution .booking-execution');
 });
 const executionBox=await page.locator('#live_execution').boundingBox();
 for(const phase of ['server_precheck','submitting','order_accepted','verifying_order']) {
  const reads=await page.evaluate(()=>window.__calls.filter(c=>c.command==='list_pending_orders').length);
  const message=await page.locator('.booking-current-detail').first().innerText();
  await page.evaluate(phase=>{
   const event={...window.__live.last_event,ts:'2026-10-30T12:01:02+08:00',result:phase,http:{phase}};
   window.__live.last_event=event;window.__live.recent_events.push(event);window.__emit('run-changed');
  },phase);
  await page.waitForFunction(previous=>document.querySelector('.booking-current-detail').textContent!==previous,message);
  assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==='list_pending_orders').length),reads,'running phases do not scan the actively written journal');
  assert.equal(await page.evaluate(()=>window.__pendingNode===document.querySelector('#pending_order_checks .settings-group')),true,'older pending rows retain their DOM nodes while running');
  assert.equal(await page.getByText('本次执行中的临时记录',{exact:true}).count(),0,'temporary current-run records never create another pending widget');
  assert.ok(Math.abs((await page.locator('#live_execution').boundingBox()).y-executionBox.y)<1,'pending reads do not move the execution card');
 }
 await page.screenshot({path:'artifacts/progress-unified/confirming-dark.png'});
 await page.evaluate(()=>{
  const event={...window.__live.last_event,action:'done',result:'http_verified',http:{verified:true,target:{sport:'乒乓球',venue:'测试场馆',date:'2026-10-31',time:'19:30',end:'21:00'}}};
  window.__live={...window.__live,running:false,last_event:event,recent_events:[...window.__live.recent_events,event]};window.__pendingOrders=[];window.__emit('run-changed');
 });
 await page.locator('#live_execution .booking-execution.success').waitFor();
 assert.equal(await page.locator('.live-results').count(),0,'the result stays in the same card without a second results component');
 assert.equal(await page.evaluate(()=>window.__executionNode===document.querySelector('#live_execution .booking-execution')),true,'the same right-hand card changes from progress to result');
 await page.waitForFunction(()=>document.querySelector('#pending_order_checks').children.length===0);
 for(const width of [1100,960]) {
  await page.setViewportSize({width,height:760});
  for(const theme of ['light','dark']) {
   await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
   await page.screenshot({path:`artifacts/progress-unified/result-${theme}-${width}.png`});
  }
 }
 await page.evaluate(()=>{
  const event={task_id:'active',ts:'2026-10-30T12:01:03+08:00',action:'http_phase',result:'submitting',http:{phase:'submitting'}};
  window.__live={...window.__live,running:true,last_event:event,recent_events:[event]};
  window.__pendingOrders=[{request_ref:'b'.repeat(64),can_recheck:false,reason:'临时提交记录',target:{venue:'本次执行中的临时记录'}}];
  window.__emit('run-changed');
 });
 await page.locator('#live_execution .booking-execution.running').waitFor();
 assert.equal(await page.locator('#pending_order_checks .settings-group').count(),0,'a current submission does not spawn a pending card');
 await page.evaluate(()=>{
  const event={...window.__live.last_event,action:'fail',result:'SUBMIT_UNKNOWN',http:{phase:'order_unconfirmed'}};
  window.__live={...window.__live,running:false,last_event:event,recent_events:[...window.__live.recent_events,event]};
  window.__pendingOrders=[{request_ref:'b'.repeat(64),can_recheck:true,target:{sport:'乒乓球',venue:'待核对场馆',date:'2026-10-31',time:'19:30',end:'21:00',court:'04'}}];window.__emit('run-changed');
 });
 await page.locator('#live_execution .booking-execution.unconfirmed').waitFor();
 await page.waitForFunction(()=>document.querySelector('[data-order-recheck]')?.disabled===false);
 await page.screenshot({path:'artifacts/progress-unified/unconfirmed-stopped.png'});
 await page.evaluate(()=>{
  const event={...window.__live.last_event,result:'HTTP_FAILED',http:{phase:'checking_orders'}};
  window.__live={...window.__live,last_event:event,recent_events:[event]};window.__pendingOrders=[];window.__emit('run-changed');
 });
 await page.getByRole('status').filter({hasText:'学校接口请求失败'}).first().waitFor();
 await page.waitForFunction(()=>document.querySelector('#pending_order_checks').children.length===0);
 await page.screenshot({path:'artifacts/progress-unified/pre-submit-timeout.png'});
 await page.evaluate(()=>{
  const event={task_id:'launch-failed',ts:'2026-10-30T12:02:00+08:00',action:'fail',result:'LOGIN_CONFIRM_REQUIRED',http:{reason:'MINIPROGRAM_NOT_VISIBLE',message:'旧的未显示提示'}};
  window.__live={...window.__live,running:false,armed:false,operation:'auth',last_event:event,recent_events:[event]};window.__emit('run-changed');
 });
 await page.waitForFunction(()=>document.querySelector('.booking-execution.failed'));
 await page.locator('.booking-head-copy h2').filter({hasText:'厦大体育小程序打开失败'}).waitFor();
 assert.match(await page.locator('.booking-current-detail').innerText(),/更多.*添加到桌面.*重试/);
 assert.equal(await page.locator('#open_booking').count(),0,'do not send the user into a failing launch retry loop');
 await page.screenshot({path:'artifacts/progress-unified/launch-failed-dark.png'});
 await page.evaluate(()=>{
   const base={task_id:'launch-failed',run_id:'fixture-process-01'};
   const events=[
    {...base,ts:'2026-10-30T12:02:00+08:00',action:'spawn',result:'http_auth'},
    {...base,ts:'2026-10-30T12:02:01+08:00',action:'http_phase',http:{phase:'authenticating'}},
    {...base,ts:'2026-10-30T12:02:02+08:00',action:'auth_check',result:'ready'},
    {...base,ts:'2026-10-30T12:02:03+08:00',action:'login_recovery'},
    {...base,ts:'2026-10-30T12:02:18+08:00',action:'http_phase',http:{phase:'authenticating',message:'未检测到可见的厦大体育小程序窗口，无法继续自动导航。请确认电脑微信已登录并打开厦大体育后重试；未提交预约'}},
    {...window.__live.last_event,...base,ts:'2026-10-30T12:02:19+08:00',http:{reason:'MINIPROGRAM_NOT_VISIBLE',message:'未检测到可见的厦大体育小程序窗口，无法继续自动导航。请确认电脑微信已登录并打开厦大体育后重试；未提交预约'}}
   ];
   window.__live={...window.__live,recent_events:events,last_event:events.at(-1)};window.__emit('run-changed');
 });
 await page.waitForFunction(()=>document.querySelectorAll('.live-timeline li').length===6);
 const summary=page.locator('.booking-details summary');
 await summary.focus();await page.keyboard.press('Enter');
 assert.equal(await page.locator('.booking-details').getAttribute('open'),'','keyboard opens process records');
 await page.clock.runFor(8000);
 for(const width of [1100,960]) {
   await page.setViewportSize({width,height:900});
   for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    await page.locator('.content').evaluate(el=>el.scrollTop=0);
    await page.locator('.live-timeline').evaluate(el=>el.scrollTop=0);
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    const header=await summary.boundingBox(),timeline=await page.locator('.live-timeline').boundingBox();
    assert.ok(timeline.y>=header.y+header.height,'timeline starts below its disclosure header');
    assert.equal(await page.locator('.live-timeline').evaluate(el=>el.scrollWidth>el.clientWidth),false,'long log messages wrap inside the timeline');
    await page.screenshot({path:`artifacts/progress-unified/process-${theme}-${width}.png`});
   }
 }
 await page.setViewportSize({width:1100,height:900});
 await summary.focus();await page.keyboard.press('Enter');
 const headerY=(await summary.boundingBox()).y;
 await page.keyboard.press('Enter');
 assert.ok(Math.abs((await summary.boundingBox()).y-headerY)<1,'expanding records keeps the header in place');
 await page.evaluate(()=>window.__emit('run-changed'));await page.clock.runFor(100);
 assert.equal(await page.locator('.booking-details').getAttribute('open'),'','status refresh preserves expanded records');
 await page.evaluate(()=>{
   window.__live={...window.__live,task:null,running:false,armed:false,last_event:null,recent_events:[]};
   window.__plans.paused=false;window.__plans.attempts=[];
   window.__emit('run-changed');window.__emit('plans-changed');
 });
 const progressTab=page.locator('.nav-primary [data-tab="live"]');
 await page.waitForFunction(()=>document.querySelector('.nav-primary [data-tab="live"]').classList.contains('has-run-dot'));
 await page.evaluate(()=>{window.__emit('run-changed');});
 await page.clock.runFor(100);
 assert.ok((await progressTab.getAttribute('class')).includes('has-run-dot'),'idle status does not erase a plan opening indicator');
 await page.evaluate(()=>{window.__plans.paused=true;window.__emit('plans-changed');});
 await page.waitForFunction(()=>!document.querySelector('.nav-primary [data-tab="live"]').classList.contains('has-run-dot'));
 await page.evaluate(()=>{window.__plans.paused=false;window.__next.forEach(n=>n.fire_at='2026-11-02T07:00:00+08:00');window.__emit('plans-changed');});
 await page.clock.runFor(100);
 assert.ok(!(await progressTab.getAttribute('class')).includes('has-run-dot'),'distant openings do not light the near-term indicator');
 assert.equal(await page.evaluate(()=>window.__calls.some(c=>['arm_hang','start_http_run','cancel_my_order'].includes(c.command))),false);
 console.log('Unified progress passed: single/plan chronological queue, pause, draft distinction, collapsed login, compact navigation and one identity request for create/save');
} finally {await browser.close();await server.close();}
