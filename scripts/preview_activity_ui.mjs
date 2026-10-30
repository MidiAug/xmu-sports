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
  for(const scenario of ['filled','empty','running','auth']) {
    const empty=scenario==='empty';
    const data=shellData();
    if(empty) {data.plans={rules:[],attempts:[],paused:false};data.next=[];}
    else data.events=events;
    if(scenario==='running') {
      data.plans={rules:[],attempts:[],paused:false};data.next=[];
      data.task={task_id:'demo',sport:target.sport,venue:target.venue,target_date:target.date,target_time_start:target.time,target_time_end:target.end,executor:'http',revision:0,allow_time_fallback:false,time_fallback_order:'down_then_up'};
      data.live={task:data.task,running:true,recent_events:events.slice(0,-1),last_event:events.at(-2)};
    }
    if(scenario==='auth') {
      data.plans={rules:[],attempts:[],paused:false};data.next=[];
      data.task={task_id:'draft',sport:'健身房',venue:'翔安校区健身房',target_date:'2026-11-01',target_time_start:'16:30',target_time_end:'18:00',executor:'http',revision:0,allow_time_fallback:false,time_fallback_order:'down_then_up'};
      const authEvents=[{ts:'2026-10-30T11:59:30+08:00',task_id:'auth-demo',run_id:'auth-demo',action:'auth_check',result:'auth_check_started'},
        {ts:'2026-10-30T11:59:55+08:00',task_id:'auth-demo',run_id:'auth-demo',action:'done',result:'http_auth_ready',http:{message:'体育登录已就绪，未提交预约'}}];
      data.events=authEvents;
      data.live={task:data.task,operation:'auth',recent_events:authEvents,last_event:authEvents.at(-1)};
    }
    const context=await browser.newContext({viewport:{width:1100,height:760}});
    await context.addInitScript(installShellFixture,data);
    const page=await context.newPage();
    await page.clock.install({time:new Date(scenario==='running'?'2026-10-30T07:00:05+08:00':'2026-10-30T12:00:00+08:00')});
    await page.goto(base);
    for(const tab of scenario==='running'?['live']:scenario==='auth'?['live','history']:['live','history','settings']) {
      await page.locator(`.app-header [data-tab="${tab}"]`).click();
      await page.locator(`.${tab==='history'?'history':tab==='live'?'live':'settings'}-page`).waitFor();
      await page.locator('.content').evaluate(el=>el.scrollTop=0);
      if(tab==='history'&&!empty) await page.locator('.run-card').first().waitFor();
      if(scenario==='auth' && tab==='live') {
        await page.locator('.booking-auth').waitFor();
        assert.equal(await page.locator('.booking-target').count(),0,'login-only check does not invent a booking target');
        assert.equal(await page.locator('.booking-track').count(),0,'login result needs no booking stepper');
        assert.equal(await page.locator('#live_single_context .live-draft-place').innerText(),'翔安校区健身房');
      }
      if(scenario==='auth' && tab==='history') {
        await page.locator('#history_kind_host .shell-select-trigger').click();
        await page.getByRole('option',{name:'成功',exact:true}).click();
        assert.equal(await page.locator('.run-card').count(),0,'login probe is not a successful booking');
        await page.locator('#history_kind_host .shell-select-trigger').click();
        await page.getByRole('option',{name:'其他',exact:true}).click();
        assert.equal(await page.locator('.run-card').count(),1,'login probe is in Other');
      }
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
