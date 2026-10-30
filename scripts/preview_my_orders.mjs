// Full homepage preview with synthetic orders; all IPC stays in the offline fixture.
import assert from 'node:assert/strict';
import {mkdirSync} from 'node:fs';
import {launchBrowser} from './lib/browser.mjs';
import {installShellFixture} from './fixtures/tauri-shell.js';
import {shellData} from './fixtures/shell-data.mjs';
const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,'Set XMU_TEST_BASE_URL to a running local Vite server');
const data={...shellData(),mode:'single'};
data.orders=[
  {order_id:'demo-1',details_id:'demo-detail-1',venue:'翔安校区乒乓球室',sport:'乒乓球',court:'02 号场地',date:'2026-10-30',start:'19:30',end:'21:00',can_cancel:true},
  {order_id:'demo-2',details_id:'demo-detail-2',venue:'思明校区风雨球馆',sport:'羽毛球',court:'05 号场地',date:'2026-10-31',start:'16:00',end:'17:00',can_cancel:true},
  {order_id:'demo-3',details_id:'demo-detail-3',venue:'翔安校区健身房',sport:'健身房',court:'爱秋体育馆健身房',date:'2026-11-01',start:'16:30',end:'18:00',can_cancel:true},
];
const browser=await launchBrowser();
mkdirSync('artifacts/my-orders-preview',{recursive:true});
try {
  const context=await browser.newContext({viewport:{width:1100,height:760}});
  await context.addInitScript(installShellFixture,data);
  const page=await context.newPage();
  await page.clock.install({time:new Date('2026-10-30T12:00:00+08:00')});
  await page.goto(base);
  await page.locator('.setup-page').waitFor();
  await page.waitForFunction(()=>document.querySelector('.orders-count')?.textContent==='3');
  const reads=await page.evaluate(()=>window.__calls.filter(c=>c.command==='get_my_orders').length);
  await page.locator('.my-orders-toggle').hover();
  await page.locator('.my-orders-popup').waitFor({state:'visible'});
  assert.equal(await page.locator('.my-order').count(),3);
  await page.locator('.my-orders-popup h2').hover();
  await page.clock.runFor(300);
  assert.equal(await page.locator('.my-orders-popup').isVisible(),true);
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.shellTheme=theme,theme);
    await page.screenshot({path:`artifacts/my-orders-preview/home-${theme}.png`});
  }
  await page.setViewportSize({width:960,height:600});
  await page.locator('.my-orders-toggle').hover();
  await page.locator('.my-orders-popup h2').hover();
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.screenshot({path:'artifacts/my-orders-preview/home-dark-narrow.png'});
  await page.locator('.setup-page h2').hover();
  await page.clock.runFor(300);
  assert.equal(await page.locator('.my-orders-popup').isVisible(),false);
  assert.equal(await page.evaluate(()=>window.__calls.filter(c=>c.command==='get_my_orders').length),reads,'hover uses the current snapshot');
  await page.locator('.my-orders-toggle').hover();
  await page.evaluate(()=>{document.documentElement.dataset.shellTheme='light';window.__holdOrders=true;});
  await page.clock.runFor(11000);
  await page.locator('[data-orders-refresh]').click();
  await page.locator('.order-icon.is-loading svg').waitFor();
  await page.screenshot({path:'artifacts/my-orders-preview/home-refreshing.png',animations:'disabled'});
  await page.keyboard.press('Escape');
  await page.locator('.orders-count.is-loading svg').waitFor();
  await page.screenshot({path:'artifacts/my-orders-preview/header-refreshing.png',animations:'disabled'});
  await page.evaluate(()=>{window.__holdOrders=false;window.__finishOrders();});
  assert.equal(await page.evaluate(()=>window.__calls.some(c=>['start_http_run','arm_hang','cancel_my_order','reauthenticate_availability'].includes(c.command))),false);
  console.log('Homepage order preview passed: hover, card traversal, leave, three synthetic orders, no live operations');
} finally {await browser.close();}
