// Real booking controls with synthetic availability; no upstream or booking calls.
import assert from 'node:assert/strict';
import {readFileSync,mkdirSync} from 'node:fs';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const data={...shellData(),mode:'single',plans:{rules:[],attempts:[],paused:false},next:[]};
const {sports}=JSON.parse(readFileSync('src/catalog/venues.json','utf8'));
const ping=sports.find(s=>s.label==='乒乓球');
data.availabilitySlots=ping.venues[0].time_slots.map(slot=>({...slot,cells:Array.from({length:12},(_,i)=>({court:String(i+1).padStart(2,'0'),admin_locked:i%3===0,status:i%3===0?'locked':'available',remaining:i%3===1?0:1,capacity:1,reason:''}))}));
const browser=await launchBrowser();
mkdirSync('artifacts/court-refinement',{recursive:true});
try {
  const context=await browser.newContext({viewport:{width:1100,height:760}});
  await context.addInitScript(installShellFixture,data);
  const page=await context.newPage();
  await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});
  await page.goto(process.env.XMU_TEST_BASE_URL);
  await page.locator('#court_host').waitFor();
  await page.locator('[data-date="2026-10-30"]').click();
  await page.clock.runFor(1500);
  await page.locator('#court_host .shell-select-badge').first().waitFor({state:'attached'});
  for(const size of [{width:1100,height:760},{width:960,height:600}]) {
    await page.setViewportSize(size);
    await page.keyboard.press('Escape');
    await page.locator('#court_host .shell-select-trigger').scrollIntoViewIfNeeded();
    await page.clock.runFor(1000);
    await page.locator('#court_host .shell-select-trigger').click();
    await page.locator('#court_host .shell-select-menu').waitFor({state:'visible'});
    const widths=await page.locator('#court_host .shell-select-menu').boundingBox();
    assert.ok(widths.width>=380);
    assert.equal(await page.locator('#court_host .shell-select-option-main').evaluateAll(els=>els.every(el=>getComputedStyle(el).flexWrap==='nowrap' && el.scrollWidth<=el.clientWidth+1)),true,'court name and status fit on one line');
    for(const theme of ['light','dark']) {
      await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
      await page.screenshot({path:`artifacts/court-refinement/courts-${theme}-${size.width}.png`,animations:'disabled'});
    }
  }
  console.log('Court preview passed: rounded trigger, 380px menu, locked/full one-line labels, light/dark/minimum window');
} finally {await browser.close();}
