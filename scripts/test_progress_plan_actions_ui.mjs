import assert from 'node:assert/strict';
import {mkdirSync} from 'node:fs';
import {createServer} from 'vite';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';

const server=await createServer({server:{host:'127.0.0.1',port:1444,strictPort:false}});
await server.listen();
const browser=await launchBrowser();
mkdirSync('artifacts/progress-plan-actions',{recursive:true});
try {
 const data=shellData();
 data.next=data.next.map(n=>({...n,date:'2026-11-01',fire_at:'2026-10-31T07:00:00+08:00'}));
 const context=await browser.newContext({viewport:{width:1100,height:760},reducedMotion:'reduce'});
 await context.addInitScript(installShellFixture,data);
 const page=await context.newPage();
 await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});
 await page.goto(`http://127.0.0.1:${server.httpServer.address().port}`);
 await page.locator('.nav-primary [data-tab="live"]').click();
 const skips=page.locator('[data-skip-plan]');
 const badge=()=>page.locator('.nav-primary [data-tab="live"]').evaluate(el=>el.classList.contains('has-run-dot'));
 const badgeCalls=()=>page.evaluate(()=>window.__calls.filter(c=>c.command==='set_run_badge').length);
 await page.waitForFunction(()=>document.querySelectorAll('[data-skip-plan]').length===2);
 assert.equal(await badge(),true,'tomorrow opening lights the progress indicator with plans only');
 assert.ok(await badgeCalls()>0,'plan activity also requests a native badge refresh');
 assert.equal(await page.evaluate(()=>window.__live.task),null);
 assert.equal((await page.locator('.live-plan-queue').innerText()).includes('·'),false,'queue uses layout and source badges instead of dot separators');
 const calls=await badgeCalls();
 await page.evaluate(()=>window.__emit('run-changed'));await page.clock.runFor(100);
 assert.equal(await badge(),true,'idle single task updates do not erase plan activity');
 assert.equal(await badgeCalls(),calls,'unchanged union does not refresh the native icon');
 for(const width of [1100,960]) {
  await page.setViewportSize({width,height:760});
  for(const theme of ['light','dark']) {
   await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
   const size=await page.locator('.live-plan-queue').evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth}));
   assert.ok(size.scroll<=size.width,'queue fits the minimum desktop window');
   await page.screenshot({path:`artifacts/progress-plan-actions/queue-${theme}-${width}.png`});
  }
 }
 await page.evaluate(()=>window.__skipError='模拟跳过失败');
 await skips.first().click();
 await page.getByText('跳过失败',{exact:true}).waitFor();
 assert.equal(await skips.first().isDisabled(),false,'failed skip can be retried');
 assert.equal(await skips.count(),2,'failure retains both occurrences');
 await page.evaluate(()=>window.__skipError=null);
 const skippedId=await skips.first().getAttribute('data-skip-plan');
 await skips.first().click();
 await page.waitForFunction(()=>document.querySelectorAll('[data-skip-plan]').length===1);
 assert.equal(await page.evaluate(id=>window.__plans.attempts.find(a=>a.task.plan_id===id)?.task.target_date,skippedId),'2026-11-01','skip applies to the displayed sport date');
 assert.equal(await page.evaluate(()=>window.__plans.rules.every(r=>r.enabled)),true,'skip leaves recurring rules enabled');
 assert.equal(await badge(),true,'another upcoming plan keeps the badge lit');
 await skips.first().click();
 await page.waitForFunction(()=>!document.querySelector('.nav-primary [data-tab="live"]').classList.contains('has-run-dot'));
 assert.ok(await badgeCalls()>calls,'skipping the last nearby opening refreshes the native icon');
 await page.evaluate(()=>{
  window.__next.forEach(n=>{n.date='2026-11-02';n.fire_at='2026-11-01T07:00:00+08:00';});window.__emit('plans-changed');
 });
 await page.clock.runFor(100);
 assert.equal(await badge(),false,'opening beyond tomorrow stays quiet');
 await page.clock.fastForward(12*60*60*1000);
 await page.waitForFunction(()=>document.querySelector('.nav-primary [data-tab="live"]').classList.contains('has-run-dot'));
 await page.evaluate(()=>{window.__plans.paused=true;window.__emit('plans-changed');});
 await page.waitForFunction(()=>!document.querySelector('.nav-primary [data-tab="live"]').classList.contains('has-run-dot'));
 assert.equal(await page.evaluate(()=>window.__calls.some(c=>['arm_hang','start_http_run','cancel_my_order','launch_xmu_miniprogram'].includes(c.command))),false);
 console.log('Progress plan actions passed: plan-only native badge refresh, skip/failure/retry, independent occurrences, midnight, pause and separator-free layout; offline fixture only');
} finally {await browser.close();await server.close();}
