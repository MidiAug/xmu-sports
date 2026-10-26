// Isolated browser fixture: real update controller, version icon and About page;
// synthetic release/download only, with no account or installation access.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import ts from 'typescript';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE, '.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright'));
const output = resolve('artifacts/version-update-test');
mkdirSync(output, { recursive: true });
const asModule = source => `data:text/javascript;base64,${Buffer.from(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText).toString('base64')}`;
const controller = asModule(readFileSync('src/lib/app-update.ts', 'utf8').replace(/^import .*;\r?\n/gm, '') + '\nconst invoke=window.mockInvoke,listen=async()=>()=>{};');
const lifecycle = asModule(readFileSync('src/lib/page-lifecycle.ts', 'utf8'));
const icon = asModule(readFileSync('src/ui/version-update.ts', 'utf8').replace('"../lib/app-update"', JSON.stringify(controller)));
const about = asModule(readFileSync('src/pages/about.ts', 'utf8')
  .replaceAll('import.meta.env.DEV', 'false')
  .replace(/^import .*@tauri.*;\r?\n/gm, '')
  .replace('"../lib/page-lifecycle"', JSON.stringify(lifecycle))
  .replace('"../lib/app-update"', JSON.stringify(controller))
  .replace('new URL("../../assets/app-icon/source.svg", import.meta.url).href', JSON.stringify(`data:image/svg+xml;base64,${Buffer.from(readFileSync('assets/app-icon/source.svg')).toString('base64')}`))
  + '\nconst getVersion=async()=>"0.1.29",open=async()=>{};');
const css = readFileSync(join('dist/assets', readdirSync('dist/assets').find(name => name.endsWith('.css'))), 'utf8');
const fixture = join(output, 'fixture.html');
writeFileSync(fixture, `<!doctype html><html data-shell-theme="light"><meta charset="utf-8"><style>${css}</style>
<div id="app"><div class="shell"><div class="shell-row"><aside class="nav"><div class="nav-brand"><h1>XMU Sports</h1></div><nav class="nav-tabs"><button>预约</button><button>运行</button><button>历史</button><button>设置</button></nav><div class="nav-foot"><span class="nav-version">v0.1.29</span></div></aside><div class="shell-main"><div class="titlebar"></div><main class="main"><div class="content" id="panel">预约</div></main></div></div></div></div>
<script>window.calls=[];window.mockInvoke=async command=>{calls.push(command);if(command==='check_app_update')return {status:window.releaseStatus||'available',version:'0.2.0',size:10485760,notes:'体验改进与问题修复'};if(command==='download_app_update')return 'synthetic.exe';throw Error('Installation is unavailable in this fixture');};</script>
<script type="module">import {mountVersionUpdate} from ${JSON.stringify(icon)};import {renderAbout} from ${JSON.stringify(about)};import {checkAppUpdate} from ${JSON.stringify(controller)};import {disposePage} from ${JSON.stringify(lifecycle)};
const panel=document.querySelector('#panel');window.leave=()=>{disposePage(panel);panel.textContent='预约';};window.openAbout=()=>renderAbout(panel,leave);mountVersionUpdate(document.querySelector('.nav-foot'),openAbout);window.check=checkAppUpdate;window.ready=true;</script>`);
const browser = await chromium.launch({ headless: true, channel: 'msedge' });
try {
  const page = await browser.newPage({ viewport: { width: 960, height: 720 } });
  await page.goto(pathToFileURL(fixture).href);
  await page.waitForFunction(() => window.ready);
  assert.equal(await page.locator('.version-update').isVisible(), false);
  await page.evaluate(() => window.openAbout());
  assert.match(await page.locator('#update_status').innerText(), /自动检查.*24小时/);
  assert.deepEqual(await page.evaluate(() => window.calls), [], 'opening About never triggers a check');
  await page.screenshot({ path: join(output, 'automatic-policy.png') });
  await page.evaluate(() => window.leave());
  await page.evaluate(() => window.check());
  await page.getByRole('button', { name: '有新版本 0.2.0，查看应用更新' }).click();
  await page.getByRole('button', { name: '下载更新', exact: true }).waitFor();
  assert.match(await page.locator('#update_status').innerText(), /发现新版本 0.2.0/);
  assert.deepEqual(await page.evaluate(() => window.calls), ['check_app_update']);
  await page.screenshot({ path: join(output, 'available-light.png') });
  await page.getByRole('button', { name: '下载更新', exact: true }).click();
  await page.getByRole('button', { name: '退出并安装', exact: true }).waitFor();
  await page.getByRole('button', { name: '返回', exact: true }).click();
  await page.locator('.version-update').focus();
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: '退出并安装', exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.calls), ['check_app_update', 'download_app_update']);
  await page.evaluate(() => document.documentElement.dataset.shellTheme = 'dark');
  assert.equal(await page.locator('.version-update').evaluate(button => getComputedStyle(button).outlineStyle), 'none');
  await page.screenshot({ path: join(output, 'keyboard-focus-dark.png') });
  await page.locator('.version-update').evaluate(button => button.blur());
  await page.screenshot({ path: join(output, 'downloaded-dark.png') });
  await page.evaluate(async () => { window.releaseStatus = 'current'; await window.check(); });
  assert.equal(await page.locator('.version-update').isVisible(), false);
  console.log('Version update browser checks passed: visibility, keyboard navigation, retained release/download, no duplicate check, current version hides icon');
} finally { await browser.close(); }
