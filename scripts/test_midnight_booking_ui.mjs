// Real frontend under a synthetic Tauri boundary. No credentials or school access.
import assert from 'node:assert/strict';
import {launchBrowser} from "./lib/browser.mjs";
import { mkdirSync } from 'node:fs';
const browser = await launchBrowser();
const output = 'artifacts/midnight-booking'; mkdirSync(output, { recursive: true });
try {
  const context = await browser.newContext({ viewport: { width: 960, height: 800 } });
  const page = await context.newPage();
  await page.clock.install({ time: new Date('2026-10-27T00:30:00+08:00') });
  await page.context().addInitScript(() => {
    let task = { task_id: 'fixture', sport: '羽毛球', venue: '翔安校区羽毛球场', target_date: '2026-10-28', fire_at: '2026-10-27T07:00:00+08:00', target_time_start: '09:00', target_time_end: '10:00', executor: 'http', revision: 0 };
    window.bookingCalls = [];
    window.readFixtureTask = () => task;
    window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }, transformCallback: () => 1, unregisterCallback() {}, invoke: async (command, args) => {
      if (command === 'read_task_raw') return task;
      if (command === 'save_task') return task = { ...args.task, revision: task.revision + 1 };
      if (command === 'get_live_status') return { task, armed: false, running: false, recent_events: [], ...window.liveStatusOverride };
      if (command === 'get_shell_prefs') return { favorite_sport_ids: [], venue_slot_prefs: {} };
      if (command === 'list_recent_events') return [];
      if (command === 'list_pending_orders') return [];
      if (command.includes('version')) return '0.1.30';
      if (command === 'get_venue_availability') {
        const cells = Array.from({ length: 10 }, (_, i) => ({ court: String(i + 1).padStart(2, '0'), admin_locked: false, status: 'unavailable', reason: '', remaining: i === 6 ? 1 : 0, capacity: 1 }));
        return { ok: true, state: 'availability_ready', date: args.date, sport: args.sport, venue: args.venue, cached: false, can_choose: true, fetched_at: '2026-10-27T00:30:00+08:00', slots: [{ start: '09:00', end: '10:00', cells }, { start: '10:00', end: '11:00', cells: cells.map(cell => ({ ...cell, remaining: 0 })) }] };
      }
      if (command === 'arm_hang' || command === 'start_http_run') { window.bookingCalls.push(command); throw Error('Booking forbidden in fixture'); }
      return null;
    } };
  });
  await page.goto(process.env.XMU_TEST_BASE_URL ?? 'http://127.0.0.1:1425');
  await page.locator('[data-date="2026-10-28"][aria-pressed="true"]').waitFor();
  await page.clock.runFor(1000);
  await page.locator('.task-advanced > summary').click();
  assert.equal(await page.locator('#delay_prev').isDisabled(),true);
  await page.locator('#delay_next').click();
  assert.equal(await page.locator('#fire_delay').inputValue(),'1');
  await page.locator('#fire_delay').fill('300');
  assert.equal(await page.locator('#delay_next').isDisabled(),true);
  await page.locator('#delay_prev').click();
  assert.equal(await page.locator('#fire_delay').inputValue(),'299');
  await page.locator('#fire_delay').fill('10');
  await page.locator('#fire_delay').focus();
  assert.deepEqual(await page.locator('#fire_delay').evaluate(el=>({outline:getComputedStyle(el).outlineStyle,shadow:getComputedStyle(el).boxShadow})),{outline:'none',shadow:'none'});
  await page.clock.runFor(500);
  assert.equal((await page.evaluate(()=>window.readFixtureTask())).fire_delay_seconds,10);
  await page.locator('.fire-delay-stepper').screenshot({path:`${output}/delay-stepper-focused.png`});
  await page.locator('#fire_delay').fill('0');
  await page.locator('.task-advanced > summary').click();
  assert.equal(await page.locator('#setup_primary').innerText(), '定时预约');
  assert.equal(await page.locator('#setup_primary').isEnabled(), true);
  assert.equal(await page.locator('#setup_now').isVisible(), false, 'no immediate action before 07:00');
  await page.locator('[data-date="2026-10-27"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#setup_primary').innerText(), '定时预约');
  assert.equal(await page.locator('#setup_primary').isEnabled(), true, 'today can be scheduled for today 07:00');
  assert.match(await page.locator('#task_execution').innerText(), /今天.*07:00/);
  assert.equal(await page.locator('#setup_now').isVisible(), false);
  assert.match(await page.locator('#slot_host [data-value="10:00|11:00"]').innerText(), /已满/);
  assert.doesNotMatch(await page.locator('#slot_host').innerText(), /暂不可约/);
  await page.keyboard.press('Escape');
  await page.locator('#court_host .shell-select-trigger').click();
  assert.match(await page.locator('#court_host [data-value="01"]').innerText(), /已满/);
  assert.doesNotMatch(await page.locator('#court_host').innerText(), /暂不可约/);
  assert.equal(await page.locator('#court_host [data-value="01"]').isEnabled(), true, 'quota snapshot is informational');
  await page.screenshot({ path: `${output}/counts-light.png` });
  await page.keyboard.press('Escape');
  await page.locator('[data-date="2026-10-29"]').click(); await page.clock.runFor(1000);
  assert.match(await page.locator('#date_hint.is-warning').innerText(), /明天 07:00.*今天 07:00.*多等一天/);
  assert.equal(await page.locator('#setup_primary').isEnabled(), true, 'extra day is a warning rather than a ban');
  assert.equal(await page.locator('#setup_now').isVisible(), false, 'day after tomorrow is not an immediate booking');
  assert.match(await page.locator('#task_execution').innerText(), /明天.*07:00/);
  await page.screenshot({ path: `${output}/extra-day-light.png` });
  await page.evaluate(() => document.documentElement.dataset.shellTheme = 'dark');
  await page.setViewportSize({ width: 960, height: 600 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.screenshot({ path: `${output}/extra-day-narrow-dark.png` });
  await page.locator('[data-date="2026-10-28"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#date_hint.is-warning').count(), 0, 'selecting tomorrow clears warning');
  await page.clock.setSystemTime(new Date('2026-10-27T07:00:00+08:00')); await page.clock.runFor(1000);
  assert.equal(await page.locator('[data-date="2026-10-28"]').getAttribute('aria-pressed'), 'true', '07:00 preserves the selected actual date');
  assert.equal(await page.locator('#setup_primary').innerText(), '立即预约');
  for (const target of ['2026-10-27','2026-10-28']) {
    await page.locator(`[data-date="${target}"]`).click(); await page.clock.runFor(500);
    assert.equal(await page.locator('#setup_primary').innerText(), '立即预约');
    assert.equal(await page.locator('#setup_primary').isEnabled(), true);
  }
  await page.locator('[data-date="2026-10-29"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#date_hint.is-warning').count(), 0, '07:00 clears extra-day warning');
  assert.equal(await page.locator('#setup_primary').innerText(), '定时预约');
  assert.deepEqual(await page.evaluate(() => window.bookingCalls), []);
  for (const [target,relative] of [['2026-10-28','今天'],['2026-10-29','明天']]) {
    const rollover = await page.context().newPage();
    await rollover.clock.install({ time: new Date('2026-10-27T23:59:50+08:00') });
    await rollover.clock.pauseAt(new Date('2026-10-27T23:59:58+08:00'));
    await rollover.goto(process.env.XMU_TEST_BASE_URL ?? 'http://127.0.0.1:1425');
    await rollover.locator(`[data-date="${target}"]`).click(); await rollover.clock.runFor(300);
    await rollover.clock.runFor(2200);
    await rollover.locator(`[data-date="${target}"][aria-pressed="true"]`).waitFor();
    assert.equal(await rollover.locator(`[data-date="${target}"] > span`).innerText(), relative);
    assert.equal(await rollover.locator('[data-date="2026-10-27"]').count(), 0, 'old today leaves the range');
    assert.equal(await rollover.locator('[data-date="2026-10-30"]').count(), 1, 'new day-after-tomorrow appears');
    assert.equal(await rollover.locator('#setup_primary').innerText(), '定时预约');
    assert.equal(await rollover.locator('#setup_now').isVisible(), false);
    assert.match(await rollover.locator('#task_execution').innerText(), /今天.*07:00/);
    assert.equal((await rollover.evaluate(() => window.readFixtureTask())).target_date, target);
    await rollover.screenshot({ path: `${output}/rollover-${relative}.png` });
    await rollover.close();
  }
  const showLive = async status => {
    await page.evaluate(value => window.liveStatusOverride=value, status);
    await page.locator('.nav-tabs [data-tab="history"]').click();
    await page.locator('.nav-tabs [data-tab="live"]').click();
    await page.locator('.live-page').waitFor();
  };
  await page.setViewportSize({width:960,height:800});
  await page.evaluate(()=>document.documentElement.dataset.shellTheme='light');
  await showLive({});
  assert.equal(await page.locator('#arm_hang,#trial_run,#http_prepare').count(),0);
  await page.getByRole('button',{name:'修改单次预约',exact:true}).waitFor();
  await page.screenshot({path:`${output}/live-idle.png`});
  await showLive({armed:true,fire_at:'2026-10-28T07:00:00+08:00'});
  await page.locator('#live_remain').waitFor();
  assert.equal(await page.locator('#disarm_hang').innerText(),'取消挂机');
  const phase={task_id:'fixture',run_id:'fixture-run',ts:'2026-10-28T07:00:00+08:00',action:'http_phase',result:'querying_slots',http:{phase:'querying_slots'}};
  await showLive({running:true,last_event:phase,recent_events:[phase]});
  await page.locator('.booking-execution').waitFor();
  assert.equal(await page.locator('#disarm_hang').innerText(),'停止');
  assert.equal(await page.locator('#arm_hang,#trial_run,#http_prepare').count(),0);
  assert.deepEqual(await page.evaluate(()=>window.bookingCalls),[]);
  console.log('Midnight booking UI passed: all overnight dates scheduled, daytime actions, counts, warning, 07:00, midnight actual-date preservation and narrow layout');
} finally { await browser.close(); }
