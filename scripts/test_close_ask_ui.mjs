import assert from "node:assert/strict";
import {mkdirSync} from "node:fs";
import {launchBrowser} from "./lib/browser.mjs";
import {installShellFixture} from "./fixtures/tauri-shell.js";
import {shellData} from "./fixtures/shell-data.mjs";

const base=process.env.XMU_TEST_BASE_URL;
assert.ok(base,"Set XMU_TEST_BASE_URL to an isolated Vite server");
mkdirSync("artifacts/close-ask",{recursive:true});
const browser=await launchBrowser();
try {
  for(const theme of ["light","dark"]) {
    const context=await browser.newContext({viewport:{width:960,height:600}});
    await context.addInitScript(installShellFixture,{...shellData(),theme});
    const page=await context.newPage();
    await page.goto(base);
    await page.locator(".plan-card").first().waitFor();
    const emit=payload=>page.evaluate(payload=>window.__emit("shell-ask-close",payload),payload);
    const calls=()=>page.evaluate(()=>window.__calls.map(c=>c.command));
    const clear=()=>page.evaluate(()=>window.__calls=[]);
    await emit({armed:false,plans:true});
    await page.getByRole("dialog",{name:"自动预约已开启"}).waitFor();
    assert.equal(await page.locator("#close_ask_remember").count(),0);
    assert.equal(await page.locator("#close_ask_tray").evaluate(el=>el===document.activeElement),true);
    await page.keyboard.press("Shift+Tab");
    assert.equal(await page.locator("#close_ask_quit").evaluate(el=>el===document.activeElement),true);
    await page.keyboard.press("Tab");
    assert.equal(await page.locator("#close_ask_tray").evaluate(el=>el===document.activeElement),true);
    await page.screenshot({path:`artifacts/close-ask/plans-${theme}.png`});
    await clear();
    await page.keyboard.press("Escape");
    assert.equal(await page.locator("#close_ask_root [role=dialog]").count(),0);
    assert.ok(!(await calls()).some(c=>["hide_to_tray","force_quit_app","quit_app"].includes(c)),"Escape only dismisses the dialog");
    await emit({armed:false,plans:true});
    await page.getByRole("button",{name:"留在托盘",exact:true}).click();
    assert.ok((await calls()).includes("hide_to_tray"));
    assert.ok(!(await calls()).some(c=>["disarm_hang","save_shell_prefs","force_quit_app"].includes(c)));
    await clear();
    await emit({armed:false,plans:true});
    await page.getByRole("button",{name:"停止并退出",exact:true}).click();
    assert.equal((await calls()).filter(c=>c==="force_quit_app").length,1);
    assert.ok(!(await calls()).some(c=>["quit_app","disarm_hang","save_shell_prefs"].includes(c)));
    await clear();
    await page.evaluate(()=>window.__emit("shell-plan-exit"));
    await page.getByRole("dialog",{name:"自动预约已开启"}).waitFor();
    await page.locator(".close-ask-backdrop").click({position:{x:5,y:5}});
    assert.ok(!(await calls()).includes("hide_to_tray"),"Dismissal must not silently hide the window");
    await emit({armed:true,plans:true});
    await page.getByRole("button",{name:"停止并退出",exact:true}).click();
    const commands=await calls();
    assert.ok(commands.indexOf("disarm_hang")<commands.indexOf("force_quit_app"));
    assert.equal(commands.filter(c=>c==="force_quit_app").length,1);
    await context.close();
  }
  console.log("Close confirmation: waiting plans, active run, tray, exit and dismissal passed");
} finally {await browser.close();}
