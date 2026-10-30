// Full settings page with an offline Tauri boundary; no copied component implementation.
import assert from 'node:assert/strict';
import {mkdirSync} from 'node:fs';
import {join} from 'node:path';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,'Set XMU_TEST_BASE_URL to an isolated running Vite fixture server');
const output='artifacts/settings-layout-test';mkdirSync(output,{recursive:true});
const browser=await launchBrowser();
try {
  const context=await browser.newContext({viewport:{width:960,height:720}});
  await context.addInitScript(installShellFixture,shellData());
  const page=await context.newPage(),errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});
  await page.goto(base);
  await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.locator('.settings-page').waitFor();
  const card = page.locator('.settings-favorites');
  const toggle = page.locator('.favorite-edit');
  assert.equal(await page.locator('#favorite_sports_body').isVisible(), false);
  assert.ok((await card.boundingBox()).height < 100);
  assert.equal(await page.locator('.preference-card').first().locator('h3').innerText(), '外观与窗口');
  assert.equal(await page.locator('details.settings-diagnostics').count(),0,'diagnostic actions are visible without expanding the page');
  const diagnosticBox=await page.locator('.settings-diagnostics').boundingBox();
  const aboutBox=await page.locator('.settings-about-entry').boundingBox();
  assert.ok(Math.abs(diagnosticBox.height-aboutBox.height)<1,'bottom settings cards share a height');
  assert.ok(Math.abs(diagnosticBox.height-(await card.boundingBox()).height)<1,'diagnostics matches the compact favorites row');
  const actions=await page.locator('.settings-diagnostic-actions').boundingBox();
  const copy=await page.locator('.settings-diagnostic-copy').boundingBox();
  assert.ok(actions.x>=copy.x+copy.width,'diagnostic actions stay to the right of the heading');
  assert.equal(await page.locator('#startup_settings .settings-cell').last().evaluate(el=>getComputedStyle(el).borderBottomWidth),'0px');
  const arrow=await page.locator('.settings-about-entry > span svg').boundingBox();
  const arrowTile=await page.locator('.settings-about-entry > span').boundingBox();
  assert.ok(Math.abs(arrow.y+arrow.height/2-arrowTile.y-arrowTile.height/2)<1,'about arrow is centered in its tile');
  for(const id of ['diag_export_btn','diag_open_logs_btn']) {
    const bounds=await page.locator(`#${id}`).boundingBox();
    assert.ok(bounds.width>=96 && bounds.height>=38,'diagnostic actions have a comfortable click target');
  }
  await page.locator('#diag_export_btn').click();
  await page.getByText('已导出到下载：diagnostic-fixture.zip',{exact:true}).waitFor();
  await page.locator('#diag_open_logs_btn').click();
  await page.getByText('已打开日志文件夹',{exact:true}).waitFor();
  await page.locator('.content').evaluate(el=>el.scrollTop=0);
  assert.equal(await page.locator('.settings-page h2').evaluate(el => getComputedStyle(el).userSelect), 'none');
  assert.notEqual(await page.locator('#settings_msg').evaluate(el => getComputedStyle(el).userSelect), 'none');
  await page.screenshot({ path: join(output, 'collapsed-light.png') });
  await toggle.click();
  assert.equal(await toggle.getAttribute('aria-expanded'), 'true');
  assert.equal(await page.locator('[data-fav-sport]').count(), 8);
  assert.ok((await page.locator('[data-fav-sport]').first().boundingBox()).height <= 34);
  await page.screenshot({ path: join(output, 'expanded-light.png') });
  const buttons = page.locator('[data-fav-sport]');
  const selected = page.locator('[data-fav-sport][aria-pressed="true"]');
  // Save one change; the summary must immediately match the selected labels.
  await buttons.first().click();
  await page.waitForFunction(() => !document.querySelector('[data-fav-sport]').disabled);
  assert.ok(!(await page.locator('#favorite_sports_summary').innerText()).includes('乒乓球'));
  await page.evaluate(() => window.failSave = true);
  await buttons.first().click();
  await page.locator('#favorite_sports_message').filter({ hasText: '模拟保存失败' }).waitFor();
  assert.equal(await buttons.first().getAttribute('aria-pressed'), 'false');
  await page.evaluate(() => window.failSave = false);
  while (await selected.count() > 1) {
    await selected.first().click();
    await page.waitForFunction(() => !document.querySelector('[data-fav-sport]').disabled);
  }
  await selected.first().click();
  await page.getByText('至少保留一项常用运动', { exact: true }).waitFor();
  assert.equal(await selected.count(), 1);
  await toggle.click();
  await toggle.focus(); await page.keyboard.press('Enter');
  assert.equal(await toggle.getAttribute('aria-expanded'), 'true');
  await page.evaluate(() => document.documentElement.dataset.shellTheme = 'dark');
  await page.screenshot({ path: join(output, 'expanded-dark.png') });
  await page.setViewportSize({ width: 960, height: 600 });
  const box = await page.locator('.settings-sport-grid').boundingBox();
  for (const button of await buttons.all()) {
    const rect = await button.boundingBox();
    assert.ok(rect.x >= box.x && rect.x + rect.width <= box.x + box.width + 1);
  }
  await page.screenshot({ path: join(output, 'expanded-narrow.png') });
  assert.deepEqual(errors,[]);
  console.log('Settings browser checks passed: collapsed height, order, selection policy, editing, save/rollback, minimum one, keyboard and narrow layout');
} finally { await browser.close(); }
