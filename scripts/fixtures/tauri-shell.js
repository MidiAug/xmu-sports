// Offline host boundary for full-app UI regression and visual inspection.
export function installShellFixture(data) {
  localStorage.setItem("xmu.booking.mode", data.mode ?? "plans");
  localStorage.setItem("xmu.shellTheme", data.theme ?? "light");
  localStorage.setItem("xmu-sports.availability.v1", JSON.stringify({version:1,account:"a".repeat(64),snapshots:[],forced:[],needsLogin:false}));
  window.__calls = [];
  window.__plans = structuredClone(data.plans);
  window.__next = structuredClone(data.next);
  window.__orders = structuredClone(data.orders ?? []);
  window.__task = data.task ?? null;
  window.__live = {task:window.__task,armed:false,running:false,recent_events:[],last_event:null,fire_at:null,...data.live};
  window.__startup = {supported:true,enabled:true,hidden:true};
  window.__prefs = {favorite_sport_ids:[],venue_slot_prefs:{},autostart_hidden:true};
  const callbacks = new Map(), listeners = new Map();
  let serial = 0;
  window.__emit = (event, payload = {}) => {
    for (const [id, listener] of listeners) if (listener.event === event) callbacks.get(listener.handler)?.({event,id,payload});
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(_event,id) { listeners.delete(id); }};
  window.__TAURI_INTERNALS__ = {
    metadata:{currentWindow:{label:"main"},currentWebview:{label:"main"}},
    transformCallback(fn) { const id=++serial;callbacks.set(id,fn);return id; },
    unregisterCallback(id) { callbacks.delete(id); },
    async invoke(command,args) {
      window.__calls.push({command,args});
      if (["start_http_run","arm_hang","launch_xmu_miniprogram","reauthenticate_availability","cancel_my_order","set_startup"].includes(command)) throw new Error(`Fixture forbids ${command}`);
      if (command === "booking_plan_account") return data.account ?? "a".repeat(64);
      if (command === "get_booking_plans") {
        if(window.__plansError) throw new Error(window.__plansError);
        return {plans:structuredClone(window.__plans),next:window.__next.filter(n=>window.__plans.rules.some(r=>r.id===n.id&&r.enabled)),wechat_running:true};
      }
      if (command === "save_booking_plans") { Object.assign(window.__plans,structuredClone(args));window.__emit("plans-changed");return; }
      if (command === "skip_booking_plan") {
        if (window.__skipError) throw new Error(window.__skipError);
        const next=window.__next.find(n=>n.id===args.id);
        if (!next) throw new Error("Fixture has no next occurrence");
        const date=next.date;
        const rule=window.__plans.rules.find(r=>r.id===args.id);
        window.__plans.attempts.push({task:{task_id:`skipped-${args.id}-${date}`,plan_id:args.id,target_date:date,fire_at:next.fire_at,sport:rule.sport},sport:rule.sport,account:rule.account,status:"skipped",message:"已跳过这一次",reversible_skip:true});
        // Return a later host preview; scheduling itself is covered by Rust tests.
        next.date="2026-11-08";next.fire_at="2026-11-07T07:00:00+08:00";
        window.__emit("plans-changed");return date;
      }
      if (command === "restore_booking_plan") {
        if (window.__restoreError) throw new Error(window.__restoreError);
        const index=window.__plans.attempts.findIndex(a=>a.reversible_skip&&a.task.plan_id===args.id&&a.task.target_date===args.date);
        if (index<0) throw new Error("No reversible skip");
        const [restored]=window.__plans.attempts.splice(index,1);
        Object.assign(window.__next.find(n=>n.id===args.id),{date:args.date,fire_at:restored.task.fire_at});
        window.__emit("plans-changed");return;
      }
      if (command === "resolve_booking_plan") {window.__plans.attempts.find(a=>a.task.task_id===args.taskId).status="skipped";window.__emit("plans-changed");return;}
      if (command === "get_live_status") return structuredClone(window.__live);
      if (command === "read_task_raw") return structuredClone(window.__task);
      if (command === "save_task") {window.__task=structuredClone(args.task);return args.task;}
      if (command === "get_shell_prefs") return structuredClone(window.__prefs);
      if (command === "save_shell_prefs") {if(window.failSave)throw new Error('模拟保存失败');window.__prefs=structuredClone(args.prefs);return window.__prefs;}
      if (command === "get_startup") return window.__startup;
      if (command === "export_diagnostic_logs") return "C:/fixture-downloads/diagnostic-fixture.zip";
      if (command === "open_app_log_dir") return "fixture-logs";
      if (command === "get_my_orders") {
        if(window.__holdOrders) await new Promise(resolve=>window.__finishOrders=resolve);
        return {ok:true,orders:structuredClone(window.__orders),account_fingerprint:"a".repeat(64),fetched_at:new Date().toISOString()};
      }
      if (command === "get_venue_availability") return {ok:true,state:"availability_ready",...args,account_fingerprint:"a".repeat(64),fetched_at:new Date().toISOString(),slots:structuredClone(data.availabilitySlots ?? [])};
      if (command === "list_recent_events") return structuredClone(data.events ?? []);
      if (command === "list_pending_orders") {
        if (window.__holdPending) await new Promise(resolve=>(window.__pendingResolvers??=[]).push(resolve));
        return structuredClone(window.__pendingOrders??[]);
      }
      if (command === "probe_wechat") return {running:true};
      if (command === "plugin:app|version") return "0.1.34";
      if (command === "plugin:event|listen") {const id=++serial;listeners.set(id,args);return id;}
      if (command === "plugin:event|unlisten") {listeners.delete(args.eventId);return;}
      return null;
    },
  };
}
