// Browser-only release fixtures. Never downloads or executes a real installer.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { join } from "node:path";
import { mkdirSync } from "node:fs";
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE ?? join(process.env.USERPROFILE,".cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright"));
const browser=await chromium.launch({channel:"msedge",headless:true});
try {
  const context=await browser.newContext({viewport:{width:960,height:800}});
  await context.addInitScript(()=>{
    window.__checks=0;window.__downloads=0;window.__callbacks={};window.__updateResult={status:"available",version:"0.2.0",size:24000000,notes:"改善场次查询\n<script>window.pwned=true</script>"};
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:"main"},currentWebview:{label:"main"}},transformCallback:fn=>{const id=Math.random();window.__callbacks[id]=fn;return id;},unregisterCallback:id=>delete window.__callbacks[id],invoke:async(cmd,args)=>{
      if(cmd==="get_shell_prefs") return {favorite_sport_ids:[],venue_slot_prefs:{}};
      if(cmd==="get_live_status") return {task:null,armed:false,running:false,recent_events:[]};
      if(cmd==="list_recent_events") return [];
      if(cmd.includes("version")) return "0.1.25";
      if(cmd==="check_app_update") {window.__checks++;if(window.__holdCheck) await new Promise(r=>window.__finishCheck=r);if(window.__updateError) throw window.__updateError;return window.__updateResult;}
      if(cmd==="download_app_update") {window.__downloads++;await new Promise(r=>window.__finishDownload=r);if(window.__downloadError) throw window.__downloadError;return "C:\\Downloads\\fixture-setup.exe";}
      if(cmd.includes("listen")) {if(args?.event==="app-update-progress") window.__progressCallback=args.handler;return 1;}
      return null;
    }};
  });
  const page=await context.newPage();
  await page.goto(process.env.XMU_TEST_BASE_URL ?? "http://127.0.0.1:1425");
  const settings=async()=>{await page.locator('[data-tab="settings"]').click();await page.locator('.settings-about-entry').click();await page.locator('.about-identity').waitFor();};
  await settings();
  assert.equal(await page.evaluate(()=>window.__checks),0,"Opening About is network-free");
  await page.evaluate(()=>window.__holdCheck=true);
  await page.locator('#update_check').click();
  assert.equal(await page.locator('#update_check').isDisabled(),true);
  await page.locator('.about-back').click();await page.locator('.settings-about-entry').click();
  assert.equal(await page.evaluate(()=>window.__checks),1);
  await page.evaluate(()=>{window.__holdCheck=false;window.__finishCheck();});
  await page.locator('#update_download').waitFor();
  assert.match(await page.locator('#update_status').innerText(),/0\.2\.0/);
  await page.locator('#update_notes summary').click();
  assert.equal(await page.evaluate(()=>window.pwned),undefined,"Release notes are plain text");
  assert.match(await page.locator('#update_notes pre').innerText(),/<script>/);
  mkdirSync('artifacts/about-updates',{recursive:true});
  await page.screenshot({path:'artifacts/about-updates/available.png'});
  await page.locator('#update_download').click();
  await page.waitForFunction(()=>typeof window.__finishDownload==='function');
  assert.equal(await page.locator('#update_check').isDisabled(),true);
  await page.locator('.about-back').click();await page.locator('.settings-about-entry').click();
  assert.equal(await page.evaluate(()=>window.__downloads),1);
  await page.evaluate(()=>window.__callbacks[window.__progressCallback]({payload:{received:50,total:100}}));
  await page.waitForFunction(()=>document.querySelector('#update_progress').value===50);
  await page.evaluate(()=>window.__finishDownload());
  await page.locator('#update_folder').waitFor();
  assert.match(await page.locator('#update_help').innerText(),/退出应用/);
  for(const status of ['current','unpublished']) {
    await page.evaluate(status=>window.__updateResult={status,version:'0.1.25',size:0,notes:''},status);
    await page.locator('#update_check').click();
    await page.waitForFunction(()=>!document.querySelector('#update_check').disabled);
    assert.equal(await page.locator('#update_download').isHidden(),true);
    assert.match(await page.locator('#update_status').innerText(),status==='current'?/最新版本/:/暂未发布/);
  }
  await page.evaluate(()=>window.__updateError='GitHub 请求受限，请稍后重试');
  await page.locator('#update_check').click();
  await page.waitForFunction(()=>!document.querySelector('#update_check').disabled);
  assert.match(await page.locator('#update_status').innerText(),/请求受限/);
  await page.evaluate(()=>{window.__updateError=null;window.__updateResult={status:'available',version:'0.2.0',size:24000000,notes:'改善场次查询'};});
  await page.locator('#update_check').click();await page.locator('#update_download').waitFor();
  await page.locator('#update_download').click();
  await page.evaluate(()=>{window.__downloadError='校验失败，已删除';window.__finishDownload();});
  await page.getByRole('button',{name:'重新下载'}).waitFor();
  assert.match(await page.locator('#update_status').innerText(),/校验失败/);
  await page.setViewportSize({width:480,height:750});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.evaluate(()=>document.documentElement.dataset.shellTheme='dark');
  await page.screenshot({path:'artifacts/about-updates/narrow-dark.png'});
  console.log('About UI passed: manual checks, navigation, progress, retry, safe notes, empty releases, errors and narrow layout');
} finally { await browser.close(); }
