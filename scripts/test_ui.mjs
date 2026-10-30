import {createServer} from "vite";
import {spawn} from "node:child_process";
import {existsSync} from "node:fs";
import assert from "node:assert/strict";

assert.ok(existsSync("dist/assets"),"Run npm run build before npm run test:ui");
const scripts=[
  "test_page_layout_ui",
  "test_shell_navigation_ui", "test_booking_plans_ui", "test_my_orders",
  "test_midnight_booking_ui", "test_schedule_delay_ui", "test_history_availability_ui",
  "test_about_ui", "test_version_update", "test_settings_layout", "test_select_popup", "test_order_recheck", "test_unified_progress_ui",
];
let server;
try {
  let base=process.env.XMU_TEST_BASE_URL;
  if(!base) {
    server=await createServer({server:{host:"127.0.0.1",port:1437,strictPort:false}});
    await server.listen();
    base=`http://127.0.0.1:${server.httpServer.address().port}`;
  }
  for(const script of scripts) {
    console.log(`UI regression: ${script}`);
    const code=await new Promise((resolve,reject)=>{
      const child=spawn(process.execPath,[`scripts/${script}.mjs`],{stdio:"inherit",windowsHide:true,env:{...process.env,XMU_TEST_BASE_URL:base}});
      child.once("error",reject);child.once("exit",code=>resolve(code));
    });
    assert.equal(code,0,`${script} failed`);
  }
} finally {await server?.close();}
