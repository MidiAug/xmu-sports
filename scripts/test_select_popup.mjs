// Exercise the real selector and CSS in an isolated browser fixture. No Tauri,
// account state, WeChat, API calls or booking actions are available here.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import {launchBrowser} from "./lib/browser.mjs";
import ts from "typescript";
const root=resolve("artifacts/select-popup-test");mkdirSync(root,{recursive:true});
const asModule=source=>`data:text/javascript;base64,${Buffer.from(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText).toString("base64")}`;
const icons=asModule(readFileSync("src/ui/icons.ts","utf8"));
const module=asModule(readFileSync("src/ui/select.ts","utf8").replace('"./icons"',JSON.stringify(icons)));
const css=readFileSync(join("dist/assets",readdirSync("dist/assets").find(name=>name.endsWith(".css"))),"utf8");
const fixture=join(root,"fixture.html");
writeFileSync(fixture,`<!doctype html><html data-shell-theme="light"><meta charset="utf-8"><style>${css}
body{margin:0}.shell{--nav-w:194px}.fixture-spacer{height:230px}.fixture-tail{height:500px}</style>
<div id="app"><div class="shell"><div class="shell-row"><aside class="nav"><div class="nav-brand">XMU Sports</div></aside><div class="shell-main"><div class="titlebar"></div><main class="main"><div class="content"><div id="panel" class="page-stack setup-page"><h2>预约场馆</h2><div class="fixture-spacer"></div><div class="booking-form-card"><div class="task-grid"><div class="settings-cell"><label class="settings-cell-title">时段</label><div id="slot"></div></div><div class="settings-cell"><label class="settings-cell-title">场馆</label><div id="venue"></div></div></div></div><button id="outside">后续设置</button><div class="fixture-tail"></div></div></div></main></div></div></div></div>
<script type="module">import {mountShellSelect,closeShellSelects} from ${JSON.stringify(module)};
window.changed=[];const options=Array.from({length:12},(_,i)=>({value:String(i),label:(8+i)+':00–'+(9+i)+':00'}));
const courts=options.map((o,i)=>({value:o.value,label:i===0?'自动选择':String(i).padStart(2,'0')+' 号场地',badge:i>0?'已锁':undefined}));
window.selects={slot:mountShellSelect(document.querySelector('#slot'),options,'3',v=>changed.push(v),{columns:true}),venue:mountShellSelect(document.querySelector('#venue'),courts,'2',v=>changed.push(v),{compact:true})};document.addEventListener('click',closeShellSelects);window.ready=true;</script>`);
const browser=await launchBrowser(process.env.SELECT_TEST_BROWSER);
let cases=0;
try {
  for(const fallback of [false,true]){
    const page=await browser.newPage();
    if(fallback)await page.addInitScript(()=>Object.defineProperty(HTMLElement.prototype,"showPopover",{value:undefined,configurable:true}));
    for(const [width,height] of [[960,720],[960,600],[1100,640],[1280,800]]){
      await page.setViewportSize({width,height});
      await page.goto(pathToFileURL(fixture).href);await page.waitForFunction(()=>window.ready);
      for(const id of ["slot","venue"]){
        const trigger=page.locator(`#${id} .shell-select-trigger`);
        await trigger.click();
        const result=await page.locator(`#${id} .shell-select-menu`).evaluate(menu=>{
          const box=menu.getBoundingClientRect(),panel=menu.closest('.content').getBoundingClientRect();
          const option=menu.querySelector('.shell-select-option');const r=option.getBoundingClientRect();
          return {left:box.left,right:box.right,top:box.top,bottom:box.bottom,panelLeft:panel.left,panelRight:panel.right,panelTop:panel.top,panelBottom:panel.bottom,hit:option.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)),native:menu.matches(':popover-open')};
        });
        assert.ok(result.left>=Math.max(8,result.panelLeft+8)-1,JSON.stringify(result));
        assert.ok(result.right<=Math.min(width-8,result.panelRight-8)+1,JSON.stringify(result));
        assert.ok(result.top>=Math.max(8,result.panelTop+8)-1,JSON.stringify(result));
        assert.ok(result.bottom<=Math.min(height-8,result.panelBottom-8)+1,JSON.stringify(result));
        assert.ok(result.hit,"a visible option must receive clicks without a panel covering it");
        if(id==='venue') assert.ok(result.right-result.left<=321,'compact court menu stays within 320px');
        assert.equal(result.native,!fallback);
        if(width===960&&height===720&&id==="slot"&&!fallback)await page.screenshot({path:join(root,"slot-fixed-960.png")});
        if(width===960&&height===720&&id==="venue"&&!fallback)await page.screenshot({path:join(root,"court-compact-960.png")});
        await page.locator(`#${id} .shell-select-option`).first().click();
        assert.equal(await trigger.getAttribute("aria-expanded"),"false");
        assert.equal(await page.locator(':popover-open').count(),0);
        await trigger.focus();await page.keyboard.press("ArrowDown");await page.keyboard.press("End");await page.keyboard.press("Enter");
        assert.equal(await page.evaluate(()=>window.changed.at(-1)),"11");
        await trigger.click();await page.keyboard.press("Escape");assert.equal(await trigger.getAttribute("aria-expanded"),"false");
        await trigger.click();await page.locator("#outside").click();assert.equal(await trigger.getAttribute("aria-expanded"),"false");
        cases++;
      }
      await page.locator('#slot .shell-select-trigger').click();
      await page.setViewportSize({width:Math.max(960,width-140),height:Math.max(600,height-80)});
      if(await page.locator('#slot').evaluate(host=>host.classList.contains('open'))){
        const box=await page.locator('#slot .shell-select-menu').boundingBox();
        assert.ok(box.x>=0&&box.x+box.width<=Math.max(960,width-140)+1,"resize repositions an open popup");
      }
      await page.evaluate(()=>window.selects.slot.setOptions([{value:'new',label:'新时段'}],'new'));
      assert.equal(await page.locator(':popover-open').count(),0,"updating options closes and releases the popup");
    }
    await page.close();
  }
  console.log(`Selector browser checks passed: ${cases} layout cases, native/fallback, left/right columns, small windows, selection, keyboard and popup cleanup`);
} finally {await browser.close();}
