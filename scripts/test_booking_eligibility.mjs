import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/booking-eligibility.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { bookingBlockedReason, hangBlockedReason } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
const task = { target_date: "2026-10-15", target_time_start: "16:30", target_time_end: "18:00", fire_at: "2026-10-14T07:00:00+08:00" };
assert.match(bookingBlockedReason(task, new Date("2026-10-13T18:35:00+08:00")), /今天或明天/);
assert.equal(hangBlockedReason(task, new Date("2026-10-13T18:35:00+08:00")), null);
assert.equal(bookingBlockedReason(task, new Date("2026-10-14T07:00:00+08:00")), null);
const today = { ...task, target_date: "2026-10-13" };
assert.match(bookingBlockedReason(today, new Date("2026-10-13T06:59:59+08:00")), /07:00/);
assert.equal(bookingBlockedReason(today, new Date("2026-10-13T07:00:00+08:00")), null);
assert.match(bookingBlockedReason(today, new Date("2026-10-13T16:30:00+08:00")), /已经开始/);
assert.match(bookingBlockedReason(today, new Date("2026-10-13T08:30:00Z")), /已经开始/);
const tomorrow = { ...task, target_date: "2026-10-14" };
assert.equal(bookingBlockedReason(tomorrow, new Date("2026-10-13T23:59:59+08:00")), null);
assert.match(bookingBlockedReason(tomorrow, new Date("2026-10-14T00:00:00+08:00")), /07:00/);
assert.match(hangBlockedReason({ ...today, fire_at: "2026-10-13T19:00:00+08:00" }, new Date("2026-10-13T07:00:00+08:00")), /已经开始/);
assert.match(bookingBlockedReason({ ...today, target_time_end: "15:00" }, new Date("2026-10-13T07:00:00+08:00")), /无效/);
console.log("11 frontend eligibility checks passed");

const timeSource = readFileSync(new URL("../src/lib/time.ts", import.meta.url), "utf8");
const timeJs = ts.transpileModule(timeSource, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const timeUrl = `data:text/javascript;base64,${Buffer.from(timeJs).toString("base64")}`;
const calibrationSource = readFileSync(new URL("../src/lib/calibrate-target-date.ts", import.meta.url), "utf8");
const pureFunction = calibrationSource.slice(calibrationSource.indexOf("export function withDefaultTargetDate"), calibrationSource.indexOf("/** ms until"));
const calibrationJs = ts.transpileModule(`import { composeFireAt, defaultTargetDateISO, fireHmFromIso, localDateISO } from ${JSON.stringify(timeUrl)};\n${pureFunction}`, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { withDefaultTargetDate } = await import(`data:text/javascript;base64,${Buffer.from(calibrationJs).toString("base64")}`);
const time = await import(timeUrl);
assert.equal(time.composeFireAt("2026-10-24","07:00",10),"2026-10-23T07:00:10+08:00");
assert.equal(time.composeFireAt("2026-10-24","07:00",180),"2026-10-23T07:03:00+08:00");
assert.equal(time.fireHmFromIso("2026-10-23T07:03:00+08:00",180),"07:00");
assert.equal(time.fireHmFromIso("2026-10-22T23:00:10Z",10),"07:00");
assert.equal(time.composeFireAt("2026-10-24","23:59",300),"2026-10-24T00:04:00+08:00");
assert.equal(time.fireHmFromIso("2026-10-24T00:04:00+08:00",300),"23:59");
for (const delay of [-1,301,1.5,NaN,Infinity]) assert.throws(()=>time.composeFireAt("2026-10-24","07:00",delay));
const delayedTask={...task,fire_at:"2026-10-14T07:03:00+08:00",fire_delay_seconds:180};
const calibrated=withDefaultTargetDate(delayedTask,new Date("2026-10-22T20:00:00+08:00"));
assert.equal(calibrated.fire_at,"2026-10-23T07:03:00+08:00");
assert.deepEqual(withDefaultTargetDate(calibrated,new Date("2026-10-22T20:00:00+08:00")),calibrated);
const cases = [
  ["2026-10-13T23:59:59+08:00", "2026-10-15"],
  ["2026-10-14T00:00:00+08:00", "2026-10-15"],
  ["2026-10-14T06:59:59+08:00", "2026-10-15"],
  ["2026-10-14T07:00:00+08:00", "2026-10-16"],
  ["2026-12-31T23:59:59+08:00", "2027-01-02"],
  ["2027-01-01T00:00:00+08:00", "2027-01-02"],
];
const oldZone = process.env.TZ;
try {
  for (const zone of ["Asia/Shanghai", "America/Los_Angeles", "UTC"]) {
    process.env.TZ = zone;
    for (const [instant, expected] of cases) {
      const now = new Date(instant);
      assert.equal(time.defaultTargetDateISO(now), expected, `${zone}: ${instant}`);
      const chosen = withDefaultTargetDate({...today, fire_at:"2026-10-12T07:12:00+08:00"}, now);
      assert.equal(chosen.target_date, expected);
      assert.equal(chosen.fire_at, time.composeFireAt(expected,"07:12"), "keep the configured execution time");
      const choices = time.bookingDateOptions(now);
      assert.deepEqual(choices.map(choice => choice.label), ["今天", "明天", "后天"]);
      assert.equal(choices.length, 3);
      assert.equal(choices[0].value, time.localDateISO(now));
      assert.equal(choices[2].value, time.addDaysISO(time.localDateISO(now),2));
      for (const choice of choices) assert.equal(time.isSelectableTargetDate(choice.value,now),true);
      assert.equal(time.isSelectableTargetDate(time.addDaysISO(time.localDateISO(now),-1),now),false);
      assert.equal(time.isSelectableTargetDate(time.addDaysISO(time.localDateISO(now),3),now),false);
    }
  }
} finally { if (oldZone === undefined) delete process.env.TZ; else process.env.TZ = oldZone; }

let saved = {...today,task_id:"date-test"}, busy = false, running = false, writes = 0;
globalThis.__dateTest = {
  invoke:async()=>({armed:busy,running,last_event:{result:"SESSION_EXPIRED"}}),
  read:async()=>saved,
  save:next=>{saved=next; writes++;},
};
try {
  const mocked = calibrationSource
    .replace('import { invoke } from "@tauri-apps/api/core";', 'const invoke = globalThis.__dateTest.invoke;')
    .replace('import { log } from "./log";', 'const log = {debug(){},warn(){}};')
    .replace('from "./time"', `from ${JSON.stringify(timeUrl)}`)
    .replace('import { readSavedTask, saveTaskDraft, flushTask } from "./task-store";', 'const readSavedTask = globalThis.__dateTest.read; const saveTaskDraft = globalThis.__dateTest.save; const flushTask = async()=>{};');
  const mockedJs = ts.transpileModule(mocked,{compilerOptions:{module:ts.ModuleKind.ESNext}}).outputText;
  const {calibrateSavedTaskTargetDate:calibrate,msUntilNextDateBoundary:boundary} = await import(`data:text/javascript;base64,${Buffer.from(mockedJs).toString("base64")}`);
  assert.equal(await calibrate(new Date("2026-10-13T20:00:00+08:00")),true);
  assert.equal(saved.target_date,"2026-10-15","cold start overrides a previous manual choice");
  saved={...saved,target_date:"2026-10-14"}; const count=writes;
  assert.equal(await calibrate(new Date("2026-10-13T21:00:00+08:00")),false);
  assert.equal(writes,count,"same booking window keeps the manual choice");
  await calibrate(new Date("2026-10-14T00:00:00+08:00"));
  assert.equal(saved.target_date,"2026-10-15","midnight resets even when the default absolute day is unchanged");
  busy=true;
  assert.equal(await calibrate(new Date("2026-10-14T07:00:00+08:00")),false);
  assert.equal(saved.target_date,"2026-10-15","a scheduled task is frozen at 07:00");
  busy=false; running=true;
  assert.equal(await calibrate(new Date("2026-10-14T07:00:01+08:00")),false);
  assert.equal(saved.target_date,"2026-10-15","an executing task is also frozen");
  running=false;
  assert.equal(await calibrate(new Date("2026-10-14T07:00:02+08:00")),true);
  assert.equal(saved.target_date,"2026-10-16","idle task catches up to the new release window");
  assert.equal(boundary(new Date("2026-10-13T23:59:59+08:00")),1000);
  assert.equal(boundary(new Date("2026-10-14T06:59:59+08:00")),1000);
} finally {delete globalThis.__dateTest;}
console.log("Booking date checks passed: 0/7 boundaries, year rollover, three-day range, three timezones, cold start, manual edits and locked tasks");
