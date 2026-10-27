// Fake wall clock and public release responses; no network, download or install.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const HOUR = 3_600_000, DAY = 24 * HOUR;
const original = { now: Date.now, setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout };
let now = 1_790_000_000_000, serial = 0, timerId = 0, calls = [], response = 'current', busy = false;
let downloadMode='ok',downloadArgs;
const timers = new Map(), storage = new Map();
globalThis.window = new EventTarget();
globalThis.document = new EventTarget();
globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) };
Date.now = () => now;
globalThis.setTimeout = (callback, delay) => { const id = ++timerId; timers.set(id, { at: now + delay, callback }); return id; };
globalThis.clearTimeout = id => timers.delete(id);
globalThis.__autoInvoke = async (command,args) => {
  calls.push(command);
  if (command === 'check_app_update') {
    if (response === 'error') throw Error('GitHub 请求受限');
    return { status: response, version: '0.2.0', size: 100, notes: 'public notes' };
  }
  if (command === 'download_app_update') {
    downloadArgs=args;
    if(downloadMode==='paused')throw Error('UPDATE_PAUSED');
    if(downloadMode==='error')throw Error('连接中断');
    return 'verified.exe';
  }
  throw Error(`Unexpected action: ${command}`);
};
const source = readFileSync('src/lib/app-update.ts', 'utf8').replace(/^import .*;\r?\n/gm, '');
async function load() {
  const js = ts.transpileModule(`const invoke=globalThis.__autoInvoke,listen=async()=>()=>{};\n${source}\n// instance ${serial++}`, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
  const api = await import('data:text/javascript;base64,' + Buffer.from(js).toString('base64'));
  let view;
  api.watchAppUpdate(value => view = value);
  return { api, view: () => view, start: (enabled = true, version = '0.1.29') => api.startAutoAppUpdate({ version, enabled, isBusy: () => busy }) };
}
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
async function advance(ms) {
  const end = now + ms;
  while (true) {
    const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) break;
    now = next[1].at; timers.delete(next[0]); next[1].callback(); await settle();
  }
  now = end; await settle();
}
function reset() { storage.clear();storage.set('xmu.appUpdate.autoDownload.v1','false'); timers.clear(); calls = []; response = 'current'; busy = false;downloadMode='ok'; }
try {
  let app = await load(); app.start(false)();
  assert.equal(timers.size, 0); assert.equal(calls.length, 0, 'development mode never schedules a request');
  let stop = app.start();
  await advance(29_999); assert.equal(calls.length, 0);
  await advance(1); assert.equal(calls.length, 1); assert.equal(app.view().phase, 'ready');
  stop(); app = await load(); stop = app.start();
  await advance(60_000); assert.equal(calls.length, 1, 'restart preserves daily cooldown');
  window.dispatchEvent(new Event('focus')); document.dispatchEvent(new Event('visibilitychange'));
  await advance(DAY - 60_000); assert.equal(calls.length, 2, 'long-running app checks once the day expires');
  stop(); const count = calls.length; window.dispatchEvent(new Event('online')); await advance(DAY);
  assert.equal(calls.length, count, 'disposed scheduler stays stopped');

  reset(); response = 'error'; app = await load(); stop = app.start(); await advance(30_000);
  assert.equal(app.view().phase, 'error'); assert.equal(calls.length, 1);
  stop(); app = await load(); stop = app.start();
  await advance(HOUR - 1); assert.equal(calls.length, 1, 'failure cooldown survives restart');
  response = 'current'; await advance(1); assert.equal(calls.length, 2);
  stop();

  reset(); busy = true; app = await load(); stop = app.start();
  await advance(30_000); assert.equal(calls.length, 0, 'booking delays checks');
  busy = false; await advance(60_000); assert.equal(calls.length, 1);
  stop();

  reset(); response = 'available'; app = await load(); stop = app.start(); await advance(30_000);
  assert.equal(app.view().release.status, 'available'); stop();
  app = await load(); stop = app.start();
  assert.equal(app.view().release.status, 'available', 'known update icon can return without a request');
  await advance(DAY + 30_000); assert.equal(calls.length, 1, 'pending update is not overwritten');
  await app.api.performAppUpdate();
  assert.deepEqual(calls, ['check_app_update', 'check_app_update', 'download_app_update'], 'restored metadata must be revalidated before explicit download');
  await advance(DAY); assert.equal(app.view().phase, 'downloaded', 'automatic checking preserves the installer');
  stop();

  reset(); storage.set('xmu.appUpdate.v1.0.1.29', '{broken'); app = await load(); stop = app.start();
  await advance(30_000); assert.equal(calls.length, 1, 'corrupt storage does not block checking'); stop();
  app = await load(); stop = app.start(true, '0.1.30'); await advance(30_000);
  assert.equal(calls.length, 2, 'new installed versions have their own cooldown'); stop();
  assert.ok(calls.every(command => command === 'check_app_update'));

  reset(); app = await load(); stop = app.start(); await advance(30_000);
  await advance(HOUR); await app.api.checkAppUpdate(); assert.equal(calls.length, 2);
  await advance(DAY - HOUR); assert.equal(calls.length, 2, 'manual success postpones automatic checking');
  await advance(HOUR); assert.equal(calls.length, 3); stop();

  reset(); storage.set('xmu.appUpdate.v1.0.1.29', JSON.stringify({ nextCheckAt: now + 99 * DAY }));
  app = await load(); stop = app.start(); await advance(30_000);
  assert.equal(calls.length, 1, 'clock rollback or invalid future timestamp cannot suppress checking forever'); stop();
  reset(); globalThis.localStorage = { getItem() { throw Error('unavailable'); }, setItem() { throw Error('quota'); } };
  app = await load(); stop = app.start(); await advance(30_000); await advance(HOUR);
  assert.equal(calls.length, 1, 'storage errors retain in-memory cooldown'); stop();
  globalThis.localStorage = {getItem:key=>storage.get(key)??null,setItem:(key,value)=>storage.set(key,value)};
  reset();storage.delete('xmu.appUpdate.autoDownload.v1');response='available';app=await load();stop=app.start();
  assert.equal(app.api.autoDownloadUpdate(),true,'automatic download defaults on');
  await advance(30_000);
  assert.equal(app.view().phase,'downloaded');
  assert.deepEqual(calls,['check_app_update','download_app_update']);
  assert.deepEqual(downloadArgs,{mirror:'official',automatic:true});
  assert.ok(!calls.includes('install_app_update'),'installation is always explicit');stop();

  reset();storage.delete('xmu.appUpdate.autoDownload.v1');response='available';downloadMode='paused';app=await load();stop=app.start();
  await advance(30_000);assert.equal(app.view().phase,'ready');assert.equal(app.view().error,undefined);
  busy=true;await advance(60_000);assert.equal(calls.length,2);
  busy=false;downloadMode='ok';await advance(60_000);assert.equal(app.view().phase,'downloaded');stop();

  reset();storage.delete('xmu.appUpdate.autoDownload.v1');response='available';downloadMode='error';app=await load();stop=app.start();
  await advance(30_000);assert.equal(app.view().failedAt,'download');
  const failures=calls.length;await advance(HOUR-1);assert.equal(calls.length,failures);
  downloadMode='ok';await advance(1);assert.equal(app.view().phase,'downloaded');stop();
  console.log('Automatic updates passed: checks, opt-out, idle download, pause/resume, hourly failure backoff, no automatic install, restart and cleanup');
} finally {
  Date.now = original.now; globalThis.setTimeout = original.setTimeout; globalThis.clearTimeout = original.clearTimeout;
}
