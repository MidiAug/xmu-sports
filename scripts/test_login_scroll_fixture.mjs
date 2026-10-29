// Replay the Rust viewport fixture in a real clipped browser layout, without WeChat.
// Run cargo test --lib first to generate the synthetic traces.
import assert from 'node:assert/strict';
import {readFileSync,mkdirSync,writeFileSync} from 'node:fs';
import {launchBrowser} from "./lib/browser.mjs";
import { join, resolve } from "node:path";
const output=resolve('artifacts/login-scroll-fixture');mkdirSync(output,{recursive:true});
const browser=await launchBrowser();
const css=`*{box-sizing:border-box}body{margin:0;padding:24px;background:#eef2f7;color:#21334c;font:14px 'Microsoft YaHei',sans-serif}h1{font-size:22px;margin:0 0 8px}p{margin:0 0 20px;color:#52647c}.grid{display:grid;grid-template-columns:repeat(2,360px);gap:20px}.card{background:white;border-radius:12px;padding:12px;box-shadow:0 2px 12px #19396212}h2{font-size:15px;margin:0 0 10px}.view{height:320px;overflow:auto;border:1px solid #bdcbdc;position:relative;box-sizing:content-box}.document{position:relative;background:#fafbfd}.heading{position:absolute;top:18px;width:100%;text-align:center;color:#064485;font-size:20px}.school{position:absolute;top:150px;left:85px;width:160px;height:220px;background:#e2ebf8;display:grid;place-items:center;border-radius:16px;font-size:24px;color:#084784}.control{position:absolute;left:20px;right:20px;height:50px;display:flex;align-items:center;justify-content:center;border:0;border-radius:26px;background:#064485;color:white;font:16px 'Microsoft YaHei'}.agreement{background:transparent;color:#365674;font-size:14px;justify-content:flex-start;border-radius:0}.agreement input{margin-right:8px}button:disabled{background:#d6dee8;color:#7e8b9e}.home-icons{position:absolute;top:500px;left:20px;right:20px;display:grid;grid-template-columns:1fr 1fr;gap:50px;color:#2b5781}.muted{font-size:12px;color:#65788d;margin-top:8px}`;
function markup(frame,index,title,hide=false) {
 const auth=frame.page==='authorization';const home=frame.page==='home';
 const controls=auth?`<label class="control agreement" data-control="agreement" style="top:690px"><input type="checkbox" ${frame.checked?'checked':''}>阅读并同意以下协议</label><button class="control" data-control="authorize" style="top:920px" ${frame.checked?'':'disabled'}>授权登录</button>`
 :home?'<div class="home-icons"><span>我的课程</span><span>阳光长跑</span><span>体质测试</span><span>教学视频</span></div><button class="control" data-control="stadium" style="top:1450px">场馆预约</button>'
 :'<div class="school">厦门大学</div><button class="control" data-control="login" style="top:600px">立即登录</button>';
 return `<section class="card"><h2>${title}</h2><div class="view" data-index="${index}"><div class="document" style="height:${frame.height}px;${hide?'--hidden:1':''}"><div class="heading">${home?'厦大体育':auth?'登录 · 微信授权':'厦门大学'}</div>${controls}</div></div><div class="muted">页面滚动位置：${frame.offset}px / ${frame.height-320}px</div></section>`;
}
async function render(name,title,frames,titles) {
 const page=await browser.newPage({viewport:{width:788,height:870}});
 const html=`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>${title}</title><style>${css}</style><body><h1>${title}</h1><p>合成小窗口，视口高320px，每次滚动160px；不访问微信或学校接口。</p><main class="grid">${frames.map((f,i)=>markup(f,i,titles[i],f.hide)).join('')}</main><script>document.querySelectorAll('.view').forEach((v,i)=>{v.scrollTop=${JSON.stringify(frames.map(f=>f.offset))}[i]});document.querySelectorAll('[style*="--hidden"]').forEach(v=>v.querySelectorAll('.control').forEach(e=>e.style.visibility='hidden'));</script></body></html>`;
 writeFileSync(join(output,`${name}.html`),html);
 await page.setContent(html);
 for(let i=0;i<frames.length;i++) {
  const f=frames[i];const visible=await page.locator(`[data-index="${i}"]`).evaluate((view,target)=>{
   const el=view.querySelector(`[data-control="${target}"]`);const a=view.getBoundingClientRect(),b=el.getBoundingClientRect();
   return getComputedStyle(el).visibility!=='hidden' && b.top>=a.top && b.bottom<=a.bottom;
  },f.target);
  assert.equal(visible,f.target_visible,`${name} ${i} clipped control visibility`);
 }
 await page.screenshot({path:join(output,`${name}.png`),fullPage:true});await page.close();
}
try {
 const chain=JSON.parse(readFileSync(join(output,'percent-chain.json'),'utf8'));
 const frames=chain.filter((e,i)=>e.event==='observe' && chain[i+1]?.event==='activate');
 assert.equal(frames.length,4);
 await render('chain','小窗口逐步查找四个操作',frames,['1 · 找到立即登录','2 · 找到协议勾选框','3 · 协议已勾选，找到授权登录','4 · 首页找到场馆预约']);
 const retry=JSON.parse(readFileSync(join(output,'retry-0-false.json'),'utf8'));
 const observations=retry.filter(e=>e.event==='observe' && e.page==='login');
 const bottom=observations.find(e=>e.offset===e.height-320);
 const up=retry.findIndex(e=>e.direction==='up');
 const top=retry.slice(up).find(e=>e.event==='observe' && e.offset===0);
 const found=observations.find(e=>e.target_visible);
 assert.ok(bottom && top && found);
 await render('retry','第一遍未找到：回顶后再向下查找',[
  {...observations[0],hide:true},{...bottom,hide:true},top,found,
 ],['1 · 第一遍开始查找','2 · 已到页面底部，仍未找到','3 · 返回顶部，重新观察','4 · 第二遍向下找到按钮']);
 console.log('Login viewport browser replay passed: clipped login/agreement/authorization/home controls and bottom→top→second sweep');
} finally {await browser.close();}
