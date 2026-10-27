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
      if(cmd.includes("version")) return "0.1.26";
      if(cmd==="check_app_update") {window.__checks++;if(window.__holdCheck) await new Promise(r=>window.__finishCheck=r);if(window.__updateError) throw window.__updateError;return window.__updateResult;}
      if(cmd==="download_app_update") {window.__downloadMirror=args?.mirror;window.__downloads++;await new Promise(r=>window.__finishDownload=r);if(window.__downloadError) throw window.__downloadError;return "C:\\Downloads\\fixture-setup.exe";}
      if(cmd==="install_app_update") {window.__installs=(window.__installs??0)+1;await new Promise(r=>window.__finishInstall=r);throw '请先结束预约、取消定时任务或完成登录，再安装更新';}
      if(cmd==="force_quit_app") {window.__quits=(window.__quits??0)+1;return;}
      if(cmd.includes("listen")) {if(args?.event==="app-update-progress") window.__progressCallback=args.handler;if(args?.event==="shell-update-exit")window.__exitCallback=args.handler;return 1;}
      return null;
    }};
  });
  const page=await context.newPage();
  await page.goto(process.env.XMU_TEST_BASE_URL ?? "http://127.0.0.1:1425");
  const settings=async()=>{await page.locator('[data-tab="settings"]').click();await page.locator('.settings-about-entry').click();await page.locator('.about-identity').waitFor();};
  await settings();
  const action=page.locator('#update_action');
  const singleButton=async()=>assert.equal(await page.locator('.about-update-button').count(),1);
  const route=page.locator('#update_route');
  assert.equal(await page.locator('#update_auto_download,#about_copy').count(),0);
  await route.locator('.shell-select-trigger').click();
  const widths=await Promise.all([action, page.locator('#about_repository'),route.locator('.shell-select-trigger'),route.locator('.shell-select-menu')].map(async el=>(await el.boundingBox()).width));
  assert.ok(widths.every(width=>Math.abs(width-widths[0])<1),'update, GitHub, selector and popup have equal widths');
  await route.locator('[data-value="gh-proxy"]').click();
  assert.equal(await page.evaluate(()=>window.__checks),0,'switching route never checks or downloads');
  await page.locator('.about-back').click();await page.locator('.settings-about-entry').click();
  assert.match(await route.locator('.shell-select-value').innerText(),/gh-proxy.com/);
  assert.equal((await page.locator('.about-back').innerText()).trim(),'返回');
  assert.equal(await page.locator('.about-page h2,.about-tagline,#update_releases,#update_folder').count(),0);
  assert.equal(await page.locator('img.about-mark').evaluate(img=>img.complete&&img.naturalWidth>0),true);
  assert.equal(await page.evaluate(()=>window.__checks),0);
  await page.evaluate(()=>{window.__holdCheck=true;window.__action=document.querySelector('#update_action');});
  await action.click();assert.equal(await action.isDisabled(),true);await singleButton();
  await page.locator('.about-back').click();await page.locator('.settings-about-entry').click();
  assert.equal(await page.evaluate(()=>window.__checks),1);
  await page.evaluate(()=>{window.__holdCheck=false;window.__finishCheck();});
  await page.getByRole('button',{name:'下载更新',exact:true}).waitFor();
  await route.locator('.shell-select-trigger').click();
  await page.evaluate(()=>window.__callbacks[window.__exitCallback]({payload:{}}));
  assert.equal(await route.locator('.shell-select-trigger').getAttribute('aria-expanded'),'false','exit reminder closes top-layer menus');
  await page.getByRole('dialog',{name:'有可用更新'}).waitFor();
  assert.equal(await page.getByRole('button',{name:'查看更新',exact:true}).isEnabled(),true);
  await page.keyboard.press('Escape');assert.equal(await page.locator('#update_exit_root').count(),0);
  await page.evaluate(()=>window.__action=document.querySelector('#update_action'));
  const box=await action.boundingBox();
  await page.locator('#update_notes summary').click();
  assert.equal(await page.evaluate(()=>window.pwned),undefined);
  assert.equal(await page.locator('#update_notes script,#update_notes img,#update_notes iframe').count(),0);
  assert.match(await page.locator('#update_notes .release-notes').innerText(),/改善场次查询/);
  mkdirSync('artifacts/about-updates',{recursive:true});
  await page.screenshot({path:'artifacts/about-updates/available.png'});
  await action.click();await page.waitForFunction(()=>typeof window.__finishDownload==='function');
  assert.equal(await page.evaluate(()=>window.__downloadMirror),'gh-proxy');
  assert.equal(await route.locator('.shell-select-trigger').isDisabled(),true);
  assert.equal(await action.isDisabled(),true);await singleButton();
  assert.equal(await page.evaluate(()=>window.__action===document.querySelector('#update_action')),true);
  await page.locator('.about-back').click();await page.locator('.settings-about-entry').click();
  assert.equal(await page.evaluate(()=>window.__downloads),1);
  await page.evaluate(()=>window.__callbacks[window.__progressCallback]({payload:{received:50,total:100}}));
  await page.getByRole('button',{name:'下载 50%',exact:true}).waitFor();
  await page.evaluate(()=>window.__finishDownload());
  await page.getByRole('button',{name:'退出并安装',exact:true}).waitFor();await singleButton();
  const installedBox=await action.boundingBox();assert.equal(box.x,installedBox.x);assert.equal(box.width,installedBox.width);
  await page.evaluate(()=>window.__callbacks[window.__exitCallback]({payload:{}}));
  await page.getByRole('button',{name:'安装并退出',exact:true}).waitFor();
  await page.getByRole('dialog',{name:'有可用更新'}).screenshot({path:'artifacts/about-updates/exit-ready.png'});
  await page.getByRole('button',{name:'安装并退出',exact:true}).click();await page.waitForFunction(()=>typeof window.__finishInstall==='function');
  assert.equal(await action.isDisabled(),true);
  await page.evaluate(()=>window.__finishInstall());
  await page.getByRole('button',{name:'重试安装',exact:true}).waitFor();
  assert.match(await page.locator('#update_status').innerText(),/结束预约/);await singleButton();
  assert.match(await page.locator('#update_exit_root .close-ask-lead').innerText(),/结束预约/);
  await page.getByRole('button',{name:'直接退出',exact:true}).click();
  assert.equal(await page.evaluate(()=>window.__quits),1);await page.keyboard.press('Escape');
  for(const status of ['current','unpublished','error']) {
    await page.reload();await settings();
    assert.match(await route.locator('.shell-select-value').innerText(),/gh-proxy.com/,'selected mirror persists across reload');
    await page.evaluate(status=>{window.__updateResult={status,version:'0.1.26',size:0,notes:''};if(status==='error')window.__updateError='GitHub 请求受限，请稍后重试';},status);
    await action.click();await page.waitForFunction(()=>!document.querySelector('#update_action').disabled);
    assert.match(await page.locator('#update_status').innerText(),status==='current'?/最新版本/:status==='unpublished'?/暂未发布/:/请求受限/);
    assert.equal(await page.locator('.about-update-detail').isHidden(),true);await singleButton();
  }
  await page.reload();await settings();
  await action.click();await page.getByRole('button',{name:'下载更新',exact:true}).waitFor();
  await action.click();await page.waitForFunction(()=>typeof window.__finishDownload==='function');
  await page.evaluate(()=>{window.__downloadError='校验失败，已删除';window.__finishDownload();});
  await page.getByRole('button',{name:'重新下载',exact:true}).waitFor();await singleButton();
  assert.match(await page.locator('#update_status').innerText(),/校验失败/);
  await page.setViewportSize({width:480,height:750});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.evaluate(()=>document.documentElement.dataset.shellTheme='dark');
  await route.locator('.shell-select-trigger').click();
  const menuBox=await route.locator('.shell-select-menu').boundingBox();
  const triggerBox=await route.locator('.shell-select-trigger').boundingBox();
  assert.ok(Math.abs(menuBox.width-triggerBox.width)<1 && menuBox.x>=0 && menuBox.x+menuBox.width<=480,'narrow popup matches trigger without overflow');
  await page.screenshot({path:'artifacts/about-updates/route-menu-dark.png'});
  await page.keyboard.press('Escape');
  await page.screenshot({path:'artifacts/about-updates/narrow-dark.png'});
  console.log('About UI passed: manual checks, navigation, progress, retry, safe notes, empty releases, errors and narrow layout');
} finally { await browser.close(); }
