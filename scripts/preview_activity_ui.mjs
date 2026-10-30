// Synthetic activity previews. The host fixture forbids real booking and login.
import {mkdirSync} from 'node:fs';
import assert from 'node:assert/strict';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,'Set XMU_TEST_BASE_URL');
const output=process.env.XMU_UI_ARTIFACTS ?? 'artifacts/activity-redesign';
mkdirSync(output,{recursive:true});
const target={sport:'乒乓球',venue:'翔安校区乒乓球室',date:'2026-10-31',time:'19:30',end:'21:00'};
const events=['authenticating','checking_orders','querying_slots','submitting','verifying_order'].map((phase,i)=>({ts:`2026-10-30T07:00:0${i}+08:00`,task_id:'demo',run_id:'demo-run',action:'http_phase',result:phase,http:{phase,target}}));
events.push({ts:'2026-10-30T07:00:08+08:00',task_id:'demo',run_id:'demo-run',action:'done',result:'http_verified',http:{verified:true,target,message:'预约成功'}});
const browser=await launchBrowser();
try {
  for(const scenario of ['filled','empty','running']) {
    const empty=scenario==='empty';
    const data=shellData();
    if(empty) {data.plans={rules:[],attempts:[],paused:false};data.next=[];}
    else data.events=events;
    if(scenario==='running') {
      data.plans={rules:[],attempts:[],paused:false};data.next=[];
      data.task={task_id:'demo',sport:target.sport,venue:target.venue,target_date:target.date,target_time_start:target.time,target_time_end:target.end,executor:'http',revision:0,allow_time_fallback:false,time_fallback_order:'down_then_up'};
      data.live={task:data.task,running:true,recent_events:events.slice(0,-1),last_event:events.at(-2)};
    }
    const context=await browser.newContext({viewport:{width:1100,height:760}});
    await context.addInitScript(installShellFixture,data);
    const page=await context.newPage();
    await page.clock.install({time:new Date(scenario==='running'?'2026-10-30T07:00:05+08:00':'2026-10-30T12:00:00+08:00')});
    await page.goto(base);
    for(const tab of scenario==='running'?['live']:['live','history','settings']) {
      await page.locator(`.app-header [data-tab="${tab}"]`).click();
      await page.locator(`.${tab==='history'?'history':tab==='live'?'live':'settings'}-page`).waitFor();
      await page.locator('.content').evaluate(el=>el.scrollTop=0);
      if(tab==='history'&&!empty) await page.locator('.run-card').first().waitFor();
      for(const theme of ['light','dark']) {
        await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
        await page.screenshot({path:`${output}/${tab}-${scenario}-${theme}.png`,animations:'disabled'});
      }
      await page.setViewportSize({width:960,height:600});
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,'minimum window has no horizontal overflow');
      await page.screenshot({path:`${output}/${tab}-${scenario}-narrow.png`,animations:'disabled'});
      await page.setViewportSize({width:1100,height:760});
    }
    await context.close();
  }
} finally {await browser.close();}
