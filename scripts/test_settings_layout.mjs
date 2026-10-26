// Actual settings UI and preference store, isolated from Tauri and user data.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import ts from 'typescript';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE, '.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright'));
const output = resolve('artifacts/settings-layout-test');
mkdirSync(output, { recursive: true });
const module = source => `data:text/javascript;base64,${Buffer.from(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText).toString('base64')}`;
const source = file => readFileSync(file, 'utf8').replace(/^import[\s\S]*?from ["'][^"']+["'];\r?\n/gm, '');
const icons = module(source('src/ui/icons.ts'));
const select = module(`import {SVG} from ${JSON.stringify(icons)};\n` + source('src/ui/select.ts'));
const catalog = module(`const catalog=${readFileSync('src/catalog/venues.json', 'utf8')};\n` + source('src/lib/catalog.ts'));
const prefs = module('const invoke=(...args)=>window.mockInvoke(...args);\n' + source('src/lib/prefs-store.ts'));
const dependencies = [
  ['closeModeFromPrefs,withCloseMode', module(source('src/lib/close-prefs.ts'))],
  ['isSportEnabled,resolveFavoriteSportIds,sports', catalog],
  ['SVG', icons], ['mountShellSelect', select], ['updatePrefs', prefs],
  ['mountPage', module(source('src/lib/page-lifecycle.ts'))], ['setMsg', module(source('src/lib/msg.ts'))],
].map(([names, url]) => `import {${names}} from ${JSON.stringify(url)};`).join('\n');
const settings = module(dependencies + '\n' + source('src/pages/settings.ts') + `
const invoke=(...args)=>window.mockInvoke(...args),getVersion=async()=>"0.1.29";
const log={debug(){},info(){},warn(){}};const currentThemePref=()=>document.documentElement.dataset.shellTheme;
const applyTheme=value=>document.documentElement.dataset.shellTheme=value;const renderAbout=async()=>{};`);
const css = readFileSync(join('dist/assets', readdirSync('dist/assets').find(name => name.endsWith('.css'))), 'utf8');
const fixture = join(output, 'fixture.html');
writeFileSync(fixture, `<!doctype html><html data-shell-theme="light"><meta charset="utf-8"><style>${css}</style>
<div id="app"><div class="shell"><div class="shell-row"><aside class="nav"><div class="nav-brand"><h1>XMU Sports</h1></div><nav class="nav-tabs"><button>预约</button><button>运行</button><button>历史</button><button class="active">设置</button></nav><div class="nav-foot">v0.1.29</div></aside><div class="shell-main"><div class="titlebar"></div><main class="main"><div class="content" id="panel"></div></main></div></div></div></div>
<script>window.prefs={favorite_sport_ids:['pingpong','badminton','swimming','gym'],close_to_tray:true,close_pref_set:true,venue_slot_prefs:{}};
window.mockInvoke=async(command,args)=>{if(command==='get_shell_prefs')return structuredClone(window.prefs);if(command==='save_shell_prefs'){if(window.failSave)throw Error('模拟保存失败');window.prefs=args.prefs;return;}throw Error(command);};</script>
<script type="module">import {renderSettings} from ${JSON.stringify(settings)};await renderSettings(document.querySelector('#panel'));window.ready=true;</script>`);
const browser = await chromium.launch({ headless: true, channel: 'msedge' });
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 720 } });
  page.setDefaultTimeout(10000);
  page.on('pageerror', error => console.error(error.message));
  await page.goto(pathToFileURL(fixture).href);
  await page.waitForFunction(() => window.ready, null, { timeout: 10000 });
  const card = page.locator('.settings-favorites');
  const toggle = page.locator('.favorite-edit');
  assert.equal(await page.locator('#favorite_sports_body').isVisible(), false);
  assert.ok((await card.boundingBox()).height < 100);
  assert.equal(await page.locator('.preference-card').first().locator('h3').innerText(), '外观与窗口');
  assert.equal(await page.locator('h2').evaluate(el => getComputedStyle(el).userSelect), 'none');
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
  await page.setViewportSize({ width: 480, height: 720 });
  const box = await page.locator('.settings-sport-grid').boundingBox();
  for (const button of await buttons.all()) {
    const rect = await button.boundingBox();
    assert.ok(rect.x >= box.x && rect.x + rect.width <= box.x + box.width + 1);
  }
  await page.screenshot({ path: join(output, 'expanded-narrow.png') });
  console.log('Settings browser checks passed: collapsed height, order, selection policy, editing, save/rollback, minimum one, keyboard and narrow layout');
} finally { await browser.close(); }
