// Real frontend under a synthetic Tauri boundary. No credentials or school access.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { mkdirSync } from 'node:fs';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE, '.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright'));
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const output = 'artifacts/midnight-booking'; mkdirSync(output, { recursive: true });
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 800 } });
  await page.clock.install({ time: new Date('2026-10-27T00:30:00+08:00') });
  await page.addInitScript(() => {
    let task = { task_id: 'fixture', sport: '羽毛球', venue: '翔安校区羽毛球场', target_date: '2026-10-28', fire_at: '2026-10-27T07:00:00+08:00', target_time_start: '09:00', target_time_end: '10:00', executor: 'http', revision: 0 };
    window.bookingCalls = [];
    window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }, transformCallback: () => 1, unregisterCallback() {}, invoke: async (command, args) => {
      if (command === 'read_task_raw') return task;
      if (command === 'save_task') return task = { ...args.task, revision: task.revision + 1 };
      if (command === 'get_live_status') return { task, armed: false, running: false, recent_events: [] };
      if (command === 'get_shell_prefs') return { favorite_sport_ids: [], venue_slot_prefs: {} };
      if (command === 'list_recent_events') return [];
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
  assert.equal(await page.locator('#setup_primary').innerText(), '定时预约');
  assert.equal(await page.locator('#setup_primary').isEnabled(), true);
  assert.equal(await page.locator('#setup_now').isVisible(), true);
  assert.equal(await page.locator('#setup_now').isEnabled(), true, 'tomorrow retains immediate action before 07:00');
  await page.locator('[data-date="2026-10-27"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#setup_primary').innerText(), '立即预约');
  assert.equal(await page.locator('#setup_primary').isEnabled(), true, 'today is not blocked by the local hour');
  assert.doesNotMatch(await page.locator('#task_action_reason').innerText(), /07:00 后/);
  await page.locator('#slot_host .shell-select-trigger').click();
  assert.match(await page.locator('#slot_host [data-value="10:00|11:00"]').innerText(), /已满/);
  assert.match(await page.locator('#slot_host [data-value="09:00|10:00"]').innerText(), /暂不可约/);
  await page.keyboard.press('Escape');
  await page.locator('#court_host .shell-select-trigger').click();
  assert.match(await page.locator('#court_host [data-value="01"]').innerText(), /已满/);
  assert.match(await page.locator('#court_host [data-value="07"]').innerText(), /暂不可约/);
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
  await page.setViewportSize({ width: 480, height: 800 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.screenshot({ path: `${output}/extra-day-narrow-dark.png` });
  await page.locator('[data-date="2026-10-28"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#date_hint.is-warning').count(), 0, 'selecting tomorrow clears warning');
  await page.clock.setSystemTime(new Date('2026-10-27T07:00:00+08:00')); await page.clock.runFor(1000);
  await page.locator('[data-date="2026-10-29"]').click(); await page.clock.runFor(1000);
  assert.equal(await page.locator('#date_hint.is-warning').count(), 0, '07:00 clears extra-day warning');
  assert.deepEqual(await page.evaluate(() => window.bookingCalls), []);
  console.log('Midnight booking UI passed: today/tomorrow immediate, scheduled default, full/unavailable counts, extra-day warning, 07:00 boundary and narrow layout');
} finally { await browser.close(); }
