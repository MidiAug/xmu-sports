// Real controllers with an offline host; timing preferences must not mutate armed snapshots.
import assert from 'node:assert/strict';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const browser=await launchBrowser();
try {
  for (const locked of [false,true]) {
    const context=await browser.newContext({viewport:{width:960,height:600}});
    const task={task_id:'timing-fixture',revision:0,sport:'羽毛球',venue:'翔安校区羽毛球场',target_date:'2026-10-24',fire_at:'2026-10-23T07:00:00+08:00',fire_delay_seconds:0,target_time_start:'09:00',target_time_end:'10:00',executor:'http'};
    await context.addInitScript(installShellFixture,{...shellData(),mode:'setup',task,live:{armed:locked,fire_at:task.fire_at},prefs:{booking_fire_time:'07:00',booking_delay_seconds:0}});
    const page=await context.newPage();
    await page.clock.install({time:new Date('2026-10-23T06:58:00+08:00')});
    await page.goto(process.env.XMU_TEST_BASE_URL);
    await page.locator('#setup_primary').waitFor();
    await page.getByRole('button',{name:'设置',exact:true}).click();
    await page.locator('#fire_delay').fill('180');
    await page.locator('#fire_delay').press('Tab');
    await page.waitForFunction(()=>window.__prefs.booking_delay_seconds===180);
    await page.locator('#fire_next').click();
    await page.waitForFunction(()=>window.__prefs.booking_fire_time==='07:01');
    await page.locator('[data-tab="plans"]').first().click();
    assert.match(await page.locator('.plans-page .page-heading p').innerText(),/07:04/);
    await page.locator('[data-tab="setup"]').first().click();
    await page.locator('#setup_primary').waitFor();
    assert.match(await page.locator('#task_execution').innerText(),locked ? /07:00/ : /07:04/);
    assert.equal(await page.locator('#setup_primary').isDisabled(),locked);
    assert.equal(await page.locator('.task-advanced,#fire_delay').count(),0);
    if (!locked) {
      await page.locator('#setup_primary').click();
      await page.getByRole('dialog',{name:'安排定时预约？'}).waitFor();
      assert.match(await page.locator('.confirm-ask-details').innerText(),/07:04/);
      await page.keyboard.press('Escape');
      // Confirming was cancelled, so this remains a draft.
    }
    assert.equal((await page.evaluate(()=>window.__calls)).some(c=>['start_http_run','arm_hang'].includes(c.command)),false);
    await context.close();
  }
  console.log('Shared timing passed: minute carry, plan display, fresh draft and armed snapshot preservation; no booking calls');
} finally {await browser.close();}
