// Shared page origins and the real new-plan form, using offline host data only.
import assert from 'node:assert/strict';
import {mkdirSync} from 'node:fs';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,'Set XMU_TEST_BASE_URL to a running local Vite server');
const browser=await launchBrowser();
mkdirSync('artifacts/page-layout',{recursive:true});
try {
  const context=await browser.newContext();
  await context.addInitScript(installShellFixture,{...shellData(),plans:{rules:[],attempts:[],paused:false},next:[]});
  const page=await context.newPage();
  await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});
  await page.goto(base);
  for(const size of [{width:1100,height:760},{width:960,height:600},{width:1440,height:900}]) {
    await page.setViewportSize(size);
    let origin;
    for(const tab of ['setup','plans','live','history','settings']) {
      await page.locator(`.app-header [data-tab="${tab}"]`).click();
      const heading=page.locator('.page-heading h2');
      await heading.waitFor();
      const box=await heading.boundingBox();
      if(!origin) origin=box;
      assert.ok(Math.abs(box.x-origin.x)<1 && Math.abs(box.y-origin.y)<1,`${tab} title shares the page origin at ${size.width}`);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    }
  }
  await page.setViewportSize({width:1100,height:760});
  await page.locator('.app-header [data-tab="plans"]').click();
  await page.locator('#plan_example').click();
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    await page.screenshot({path:`artifacts/page-layout/new-plan-${theme}.png`});
  }
  await page.setViewportSize({width:960,height:600});
  await page.locator('.content').evaluate(el=>el.scrollTop=0);
  const save=await page.locator('#plan_editor [type="submit"]').boundingBox();
  assert.ok(save.y+save.height<=600,'save stays visible in the minimum window');
  await page.screenshot({path:'artifacts/page-layout/new-plan-dark-narrow.png'});
  console.log('Page layout passed: five aligned page titles at three sizes; new-plan light/dark/minimum-window preview; no live operations');
} finally {await browser.close();}
