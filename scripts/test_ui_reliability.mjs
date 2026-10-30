import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const compile = source => `data:text/javascript;base64,${Buffer.from(ts.transpileModule(source, {compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText).toString("base64")}`;
const read = path => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const {createSaveQueue} = await import(compile(read("src/lib/save-queue.ts")));
const writes = [];
let release;
const gate = new Promise(resolve => {release = resolve;});
const queue = createSaveQueue(async value => {writes.push(value); if(value === 1) await gate;}, 60_000);
queue.enqueue(1);
const first = queue.flush();
queue.enqueue(2); queue.enqueue(3);
const second = queue.flush();
assert.deepEqual(writes, [1], "a second disk write must wait for the first");
release();
await Promise.all([first,second]);
assert.deepEqual(writes, [1,3], "the newest draft wins while writes stay serialized");
let fail = true;
const attempts = [];
const retry = createSaveQueue(async value => {attempts.push(value);if(fail) throw new Error("disk unavailable");},60_000);
retry.enqueue("draft");
await assert.rejects(retry.flush(), /disk unavailable/);
fail = false;
await retry.flush();
assert.deepEqual(attempts,["draft","draft"],"failed writes retain the draft for retry");
const {mountPage,disposePage} = await import(compile(read("src/lib/page-lifecycle.ts")));
const panel = {isConnected:true};
let disposed = 0;
const before = mountPage(panel); before.onDispose(()=>disposed++);
const after = mountPage(panel);
assert.equal(before.current(),false,"an old response loses ownership after navigation");
assert.equal(after.current(),true);
disposePage(panel); disposePage(panel);
assert.equal(after.current(),false); assert.equal(disposed,1,"cleanup runs once");
const timeUrl = compile(read("src/lib/time.ts"));
const time = await import(timeUrl);
const human = await import(compile(read("src/lib/format-human.ts").replace('"./time"',JSON.stringify(timeUrl))));
assert.equal(human.formatFireAtHuman("2026-10-23T07:00:10+08:00",new Date("2026-10-23T06:59:00+08:00")),"今天 07:00:10 开抢");
const oldZone = process.env.TZ;
try {
  for (const zone of ["Asia/Shanghai","America/Los_Angeles","Asia/Tokyo","UTC"]) {
    process.env.TZ = zone;
    assert.equal(time.localDateISO(new Date("2026-10-14T16:30:00Z")),"2026-10-15",zone);
    assert.equal(time.composeFireAt("2026-10-17","07:00"),"2026-10-16T07:00:00+08:00",zone);
    assert.equal(time.defaultTargetDateISO(new Date("2026-10-14T23:00:00Z")),"2026-10-17",zone);
    assert.equal(human.formatTargetDateHuman("2026-10-15",new Date("2026-10-14T16:30:00Z")),"今天 10月15日",zone);
    assert.equal(human.formatFireAtHuman("2026-10-15T07:00:00+08:00",new Date("2026-10-14T16:30:00Z")),"今天 07:00 开抢",zone);
  }
} finally { if(oldZone === undefined) delete process.env.TZ; else process.env.TZ=oldZone; }
const outcomeUrl=compile(read("src/lib/run-outcome.ts"));
const {outcomeFromEvent}=await import(outcomeUrl);
for (const [reason,title] of [["UIA_STADIUM_ENTRY_NOT_FOUND","首页未找到场馆预约入口"],
  ["UIA_LOGIN_CONTROL_NOT_FOUND","未找到可操作的立即登录"],["UIA_AUTH_CONTROL_NOT_FOUND","未找到可操作的协议或授权控件"],
  ["UIA_SCROLL_LIMIT_REACHED","滚动查找达到步数上限"],["UIA_SEARCH_TIMEOUT","登录或场馆入口查找超时"],
  ["UIA_PORTRAIT_UNAVAILABLE","小程序无法调整为竖屏"],["UIA_WINDOW_LAYOUT_UNAVAILABLE","小程序窗口尺寸调整失败"],
  ["UIA_WINDOW_LAYOUT_UNSTABLE","小程序窗口尺寸持续回弹"],
  ["LOGIN_FOCUS_REQUIRED","小程序未能保持在前台"],["UIA_PIN_UNAVAILABLE","小程序置顶准备失败"],
  ["MINIPROGRAM_NOT_VISIBLE","厦大体育小程序打开失败"],["UIA_TARGET_MISSING","厦大体育小程序打开失败"],["MINIPROGRAM_LAUNCH_FAILED","厦大体育小程序打开失败"],["H5_ENTRY_STILL_EXPIRED","预约登录凭证仍已失效"]]) {
  const message="具体失败原因；未提交预约";
  const failed=outcomeFromEvent({action:"fail",result:"LOGIN_CONFIRM_REQUIRED",http:{reason,message}});
  assert.equal(failed.title,title);
  if(["MINIPROGRAM_NOT_VISIBLE","UIA_TARGET_MISSING","MINIPROGRAM_LAUNCH_FAILED"].includes(reason)) assert.match(failed.detail,/更多.*添加到桌面.*重试/);
  else assert.equal(failed.detail,message);
  assert.doesNotMatch(failed.title+failed.detail,/手动确认登录|立即登录、确认协议/);
}
const missingRuntime=outcomeFromEvent({action:"fail",result:"RUNTIME_FILE_MISSING"});
const unsupportedRuntime=outcomeFromEvent({action:"fail",result:"RUNTIME_UNSUPPORTED"});
assert.match(missingRuntime.title,/未找到.*文件/);
assert.doesNotMatch(missingRuntime.title,/未适配/);
assert.match(unsupportedRuntime.detail,/导出诊断日志/);
assert.match(outcomeFromEvent({action:"fail",result:"RUNTIME_ATTACH_FAILED"}).title,/无法连接/);
for(const code of ["SUBMIT_UNKNOWN","RESULT_UNCONFIRMED","ORDER_RECHECK_UNCONFIRMED"]){
  const pending=outcomeFromEvent({action:"fail",result:code});
  assert.equal(pending.kind,"info");assert.match(pending.title,/待确认/);assert.doesNotMatch(pending.title,/失败/);
  assert.equal(human.formatStepLine({action:"fail",result:code}),"查单待确认");
}
const {groupEventsIntoRuns,historyOutcomeKind}=await import(compile(read("src/lib/run-groups.ts").replace('"./run-outcome"',JSON.stringify(outcomeUrl))));
const event=(run,action,seconds)=>({task_id:"task",run_id:run,ts:`2026-10-15T07:00:0${seconds}+08:00`,action,result:action==="done"?"http_prepared":action});
const runs=groupEventsIntoRuns([event("a","spawn",0),event("b","spawn",1),event("a","click_failed",2),event("b","done",3),event("a","done",4)]);
assert.equal(runs.length,2,"interleaved runs and recovered step failures stay grouped by run_id");
assert.equal(runs[0].runId,"a"); assert.equal(runs[0].events.length,3);
assert.equal(runs[1].runId,"b");
assert.equal(historyOutcomeKind(runs[0]),'info','a successful preflight belongs to Other history');
for(const [result,kind] of [['http_auth_ready','info'],['http_prepared','info'],['http_verified','success']]) {
  const run=groupEventsIntoRuns([{...event('classification','done',0),result}])[0];
  assert.equal(historyOutcomeKind(run),kind);
}
console.log("UI reliability checks passed: serialized saves, retry, page ownership, 4 timezones, interleaved run history");
let preferences={close_to_tray:true,favorite_sport_ids:["pingpong"],venue_slot_prefs:{existing:"18:00|19:30"}};
globalThis.__prefsInvoke=async(command,args)=>{
  if(command==="get_shell_prefs")return structuredClone(preferences);
  if(command==="save_shell_prefs"){preferences=structuredClone(args.prefs);return;}
  throw Error(command);
};
try {
  const prefs=await import(compile(read("src/lib/prefs-store.ts").replace('import { invoke } from "@tauri-apps/api/core";', 'const invoke = (command, args) => globalThis.__prefsInvoke(command, args);')));
  await Promise.all([prefs.updateVenueSlotPreference("a","19:30|21:00"),prefs.updatePrefs({detailed_diagnostics:true}),prefs.updateVenueSlotPreference("b","18:00|19:30")]);
  assert.deepEqual(preferences.venue_slot_prefs,{existing:"18:00|19:30",a:"19:30|21:00",b:"18:00|19:30"});
  assert.deepEqual(preferences.favorite_sport_ids,["pingpong"]);
  assert.equal(preferences.detailed_diagnostics,true);
  await prefs.updateVenueSlotPreference("a",null);
  assert.equal(preferences.venue_slot_prefs.a,undefined);
  assert.equal(preferences.venue_slot_prefs.b,"18:00|19:30");
} finally {delete globalThis.__prefsInvoke;}
console.log("Concurrent preference edits preserve unrelated settings and venue preferences");
