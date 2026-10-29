import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { sports, isSportEnabled } from "../lib/catalog";
import { mountPage } from "../lib/page-lifecycle";
import { mountShellSelect } from "../ui/select";
import { askConfirm } from "../ui/confirm-ask";
import { outcomeFromEvent } from "../lib/run-outcome";
import type { BookingTask } from "../types";

export type BookingPlan = { id: string; sport: string; venue: string; start: string; end: string; weekdays: number[]; enabled: boolean; account: string; first_date: string };
type Attempt = { task: BookingTask; sport: string; account: string; status: string; message: string };
type View = { plans: { rules: BookingPlan[]; paused: boolean; attempts: Attempt[] }; next: { id: string; date: string; fire_at: string }[]; wechat_running: boolean };
const days = ["一", "二", "三", "四", "五", "六", "日"];
const escape = (text: string) => text.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]!));
export const weekdaysLabel = (values: number[]) => values.length === 7 ? "每天" : "每周" + values.map(d => days[d - 1]).join("、");
const at = (iso: string) => `${iso.slice(0, 10)} ${iso.slice(11, 16)}`;

export async function renderBookingPlans(panel: Element) {
  const scope = mountPage(panel);
  let view: View;
  let editing = false, saving = false;
  let updateEditorStatus: (() => void) | undefined;
  panel.innerHTML = `<div class="page-stack plans-page">
    <div class="page-heading"><div><h2>长期计划</h2><p>设好运动日，前一天 07:00 自动预约</p></div><button type="button" class="btn primary" id="plan_add">新增计划</button></div>
    <div class="plans-summary"><div><strong id="plans_state">正在读取计划</strong><p id="plans_health" class="plan-secondary"></p></div><button type="button" class="btn ghost" id="plans_pause" hidden></button></div>
    <div id="plans_startup" class="plan-startup-hint" hidden><span>开启开机自启，登录电脑后继续执行计划</span><button type="button" class="btn ghost" id="plans_enable_startup">开启自启</button></div>
    <p class="msg" id="plans_msg" role="status"></p>
    <div id="plan_editor" hidden></div><div class="plan-list" id="plan_list"></div>
    <section id="plan_results" aria-label="最近执行"></section>
    <p class="plan-footnote">自动预约需要电脑保持唤醒、联网。关闭到托盘可继续运行；退出或关机后暂停。</p>
  </div>`;
  const message = panel.querySelector<HTMLElement>("#plans_msg")!;
  const editor = panel.querySelector<HTMLElement>("#plan_editor")!;
  const list = panel.querySelector<HTMLElement>("#plan_list")!;
  const add = panel.querySelector<HTMLButtonElement>("#plan_add")!;
  const pause = panel.querySelector<HTMLButtonElement>("#plans_pause")!;
  const fail = (e: unknown) => { if (scope.current()) { message.textContent = String(e); message.className = "msg err"; } };
  const closeMore = (event: MouseEvent | KeyboardEvent) => {
    if (event instanceof KeyboardEvent && event.key !== "Escape") return;
    list.querySelectorAll<HTMLDetailsElement>(".plan-more[open]").forEach(menu => {
      if (event instanceof MouseEvent && event.target instanceof Node && menu.contains(event.target)) return;
      menu.open = false;
      if (event instanceof KeyboardEvent) menu.querySelector("summary")?.focus();
    });
  };
  document.addEventListener("click", closeMore);
  document.addEventListener("keydown", closeMore);
  scope.onDispose(() => { document.removeEventListener("click", closeMore); document.removeEventListener("keydown", closeMore); });

  const load = async () => {
    try {
      const next = await invoke<View>("get_booking_plans");
      if (!scope.current()) return;
      view = next;
      panel.querySelector("#plans_state")!.textContent = view.plans.paused ? "自动计划已暂停" : view.plans.rules.some(r => r.enabled) ? "自动计划已开启" : "还没有启用的计划";
      panel.querySelector("#plans_health")!.textContent = `微信${view.wechat_running ? "正在运行" : "未检测到运行"} · 预约会话将在执行前单独核验`;
      pause.hidden = !view.plans.rules.length;
      pause.textContent = view.plans.paused ? "恢复计划" : "全部暂停";
      updateEditorStatus?.();
      if (!editing) paintList();
      paintResults();
    } catch (e) { fail(e); }
  };

  const save = async (rules: BookingPlan[], paused = view.plans.paused) => {
    if (saving) return;
    saving = true;
    add.disabled = pause.disabled = true;
    try {
      await invoke("save_booking_plans", { rules, paused });
      message.textContent = "";
      await load();
    } catch (e) { fail(e); throw e; }
    finally { saving = false; add.disabled = editing; pause.disabled = false; }
  };

  const move = async (id: string, to: number) => {
    if (editing || saving) return;
    const rules = [...view.plans.rules], from = rules.findIndex(r => r.id === id);
    rules.splice(to, 0, rules.splice(from, 1)[0]);
    await save(rules);
    list.querySelector<HTMLButtonElement>(`[data-edit="${id}"]`)?.focus();
  };

  function paintList() {
    if (!view.plans.rules.length) {
      list.innerHTML = `<div class="empty"><p class="empty-title">把每周运动安排好</p><p class="empty-desc">选择运动、场馆和时段，按每天、工作日或指定星期重复预约。</p><button type="button" class="btn ghost" id="plan_example">添加第一条计划</button></div>`;
      list.querySelector<HTMLButtonElement>("#plan_example")!.onclick = () => openEditor();
      return;
    }
    list.innerHTML = `<p class="plan-secondary">同轮按列表顺序依次预约，可拖动或使用上下移动按钮调整优先级。</p>` + view.plans.rules.map((r, index) => {
      const sport = sports.find(s => s.id === r.sport)!;
      const venue = sport?.venues.find(v => v.id === r.venue);
      const next = view.next.find(n => n.id === r.id);
      return `<article class="plan-card" draggable="true" data-plan="${r.id}">
        <div class="plan-card-top"><span class="plan-priority" aria-label="优先级 ${index + 1}">${index + 1}</span><div class="plan-title"><strong>${escape(sport?.label ?? r.sport)}</strong><span>${escape(venue?.label ?? r.venue)}</span></div>
        <label class="plan-enable"><input type="checkbox" data-enable="${r.id}" ${r.enabled ? "checked" : ""}><span>启用</span></label></div>
        <div class="plan-schedule"><strong>${r.start}–${r.end}</strong><span>${weekdaysLabel(r.weekdays)}</span></div>
        <p class="plan-secondary">${next ? `${next.date} 去运动 · ${view.plans.paused ? "恢复后按计划 " : ""}${at(next.fire_at)} 开抢` : "本计划已停用"}</p>
        <div class="plan-actions"><button type="button" class="btn ghost" data-edit="${r.id}">编辑</button><button type="button" class="btn ghost" data-skip="${r.id}" ${!r.enabled ? "disabled" : ""}>跳过下一次</button>
        <details class="plan-more"><summary>更多</summary><div><button type="button" data-up="${r.id}" ${index === 0 ? "disabled" : ""}>上移</button><button type="button" data-down="${r.id}" ${index === view.plans.rules.length - 1 ? "disabled" : ""}>下移</button><button type="button" data-copy="${r.id}">复制</button><button type="button" data-delete="${r.id}">删除</button></div></details></div>
      </article>`;
    }).join("");
    list.querySelectorAll<HTMLInputElement>("[data-enable]").forEach(box => box.onchange = () => {
      void save(view.plans.rules.map(r => r.id === box.dataset.enable ? { ...r, enabled: box.checked } : r)).catch(() => { box.checked = !box.checked; });
    });
    list.querySelectorAll<HTMLButtonElement>("[data-edit]").forEach(b => b.onclick = () => openEditor(view.plans.rules.find(r => r.id === b.dataset.edit)));
    list.querySelectorAll<HTMLButtonElement>("[data-copy]").forEach(b => b.onclick = () => openEditor({ ...view.plans.rules.find(r => r.id === b.dataset.copy)!, id: crypto.randomUUID(), enabled: false }, true));
    list.querySelectorAll<HTMLButtonElement>("[data-delete]").forEach(b => b.onclick = async () => {
      if (await askConfirm({ title: "删除这条计划？", lead: "已成功预约的订单会保留，之后不再自动安排。", confirmLabel: "删除计划" })) {
        void save(view.plans.rules.filter(r => r.id !== b.dataset.delete)).catch(() => {});
      }
    });
    list.querySelectorAll<HTMLButtonElement>("[data-skip]").forEach(b => b.onclick = async () => {
      b.disabled = true;
      try { await invoke("skip_booking_plan", { id: b.dataset.skip }); await load(); } catch (e) { fail(e); b.disabled = false; }
    });
    for (const [attr, offset] of [["up", -1], ["down", 1]] as const) {
      list.querySelectorAll<HTMLButtonElement>(`[data-${attr}]`).forEach(b => b.onclick = () => { void move(b.dataset[attr]!, view.plans.rules.findIndex(r => r.id === b.dataset[attr]) + offset).catch(() => {}); });
    }
    let dragged = "";
    list.querySelectorAll<HTMLElement>("[data-plan]").forEach(card => {
      card.ondragstart = event => { dragged = card.dataset.plan!; event.dataTransfer!.setData("text/plain", dragged); card.classList.add("dragging"); };
      card.ondragend = () => { card.classList.remove("dragging"); dragged = ""; };
      card.ondragover = event => event.preventDefault();
      card.ondrop = event => { event.preventDefault(); if (dragged && dragged !== card.dataset.plan) void move(dragged, view.plans.rules.findIndex(r => r.id === card.dataset.plan)).catch(() => {}); };
    });
  }

  function paintResults() {
    const attempts = [...view.plans.attempts.filter(a=>a.status==="unconfirmed"), ...view.plans.attempts.filter(a=>a.status!=="unconfirmed").slice(-12).reverse()];
    const labels: Record<string, string> = { queued: "等待执行", missed: "已错过", running: "准备 / 执行中", succeeded: "已约到", existing: "已有预约", failed: "未约到", skipped: "已跳过", cancelled: "已停止", unconfirmed: "待确认" };
    panel.querySelector("#plan_results")!.innerHTML = attempts.length ? `<h3>最近执行</h3><div class="plan-results">` + attempts.map(a => {
      const detail = a.status === "failed" ? outcomeFromEvent({ ts: "", task_id: a.task.task_id, action: "fail", result: a.message })?.title ?? "请查看运行记录" : a.status === "succeeded" ? "" : a.message;
      return `<div class="plan-result"><div><strong>${escape(a.task.sport)} · ${a.task.target_date}</strong><span>${escape(detail)}</span></div><span class="plan-result-status" data-status="${a.status}">${labels[a.status] ?? "待处理"}</span>${a.status === "unconfirmed" ? `<button type="button" class="btn ghost" data-resolve="${a.task.task_id}">已核对并跳过</button>` : ""}</div>`;
    }).join("") + `</div>` : "";
    panel.querySelectorAll<HTMLButtonElement>("[data-resolve]").forEach(button => button.onclick = async () => {
      if (await askConfirm({title:"已在我的预约或小程序核对结果？",lead:"该项会保留为已跳过，不会标记为预约成功，也不会自动再次下单。",confirmLabel:"已核对，跳过本项"})) {
        button.disabled = true;
        try {await invoke("resolve_booking_plan",{taskId:button.dataset.resolve});await load();} catch(e) {fail(e);button.disabled=false;}
      }
    });
  }

  function openEditor(existing?: BookingPlan, copying = false) {
    if (saving || !view) return;
    editing = true; editor.hidden = false; add.disabled = true; list.hidden = true;
    const enabledSports = sports.filter(isSportEnabled);
    let sport = enabledSports.find(s => s.id === existing?.sport) ?? enabledSports[0];
    let venue = sport.venues.find(v => v.id === existing?.venue) ?? sport.venues[0];
    let selected = new Set(existing?.weekdays ?? [1, 2, 3, 4, 5, 6, 7]);
    let slot = existing ? `${existing.start}|${existing.end}` : `${venue.time_slots[0].start}|${venue.time_slots[0].end}`;
    editor.innerHTML = `<form class="plan-editor"><h3>${copying ? "复制计划" : existing ? "编辑计划" : "新增计划"}</h3><div class="plan-fields"><label>运动<div id="plan_sport"></div></label><label>场馆<div id="plan_venue"></div></label><label>时段<div id="plan_slot"></div></label></div>
      <fieldset><legend>去运动的星期</legend><div class="plan-presets"><button type="button" data-days="1,2,3,4,5,6,7">每天</button><button type="button" data-days="1,2,3,4,5">工作日</button><button type="button" data-days="6,7">周末</button></div><div class="plan-weekdays">${days.map((d, i) => `<button type="button" data-day="${i + 1}" aria-label="周${d}" aria-pressed="${selected.has(i + 1)}">${d}</button>`).join("")}</div></fieldset>
      <p class="plan-preview" id="plan_preview"></p>${copying ? `<p class="plan-preview">副本保存后默认停用，请调整星期后再启用。</p>` : ""}<p class="plan-preview" id="plan_paused_hint" hidden>自动计划已暂停，保存后仍需点击“恢复计划”。</p><p class="msg" id="plan_form_msg" role="status"></p><div class="plan-actions"><button type="submit" class="btn primary"></button><button type="button" class="btn ghost" id="plan_cancel">取消</button></div></form>`;
    updateEditorStatus = () => {
      editor.querySelector<HTMLElement>("#plan_paused_hint")!.hidden = !view.plans.paused;
      editor.querySelector<HTMLButtonElement>('[type="submit"]')!.textContent = copying ? "保存为停用计划" : existing || view.plans.paused ? "保存计划" : "保存并启用";
    };
    updateEditorStatus();
    const preview = () => {
      editor.querySelectorAll<HTMLButtonElement>("[data-day]").forEach(b => b.setAttribute("aria-pressed", String(selected.has(Number(b.dataset.day)))));
      editor.querySelectorAll<HTMLButtonElement>("[data-days]").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.days!.split(",").map(Number).every(d => selected.has(d)) && b.dataset.days!.split(",").length === selected.size)));
      editor.querySelector("#plan_preview")!.textContent = selected.size ? `${weekdaysLabel([...selected].sort((a, b) => a - b))} ${slot.replace("|", "–")} 去运动，前一天 07:00 开抢` : "至少选择一天";
    };
    const slots = () => venue.time_slots.map(t => ({ value: `${t.start}|${t.end}`, label: `${t.start}–${t.end}` }));
    const slotSelect = mountShellSelect(editor.querySelector<HTMLElement>("#plan_slot")!, slots(), slot, value => { slot = value; preview(); });
    const changeVenue = () => { slot = slots()[0].value; slotSelect.setOptions(slots(), slot); preview(); };
    const venueSelect = mountShellSelect(editor.querySelector<HTMLElement>("#plan_venue")!, sport.venues.map(v => ({ value: v.id, label: v.label })), venue.id, value => { venue = sport.venues.find(v => v.id === value)!; changeVenue(); });
    mountShellSelect(editor.querySelector<HTMLElement>("#plan_sport")!, enabledSports.map(s => ({ value: s.id, label: s.label })), sport.id, value => { sport = enabledSports.find(s => s.id === value)!; venue = sport.venues[0]; venueSelect.setOptions(sport.venues.map(v => ({ value: v.id, label: v.label })), venue.id); changeVenue(); });
    editor.querySelectorAll<HTMLButtonElement>("[data-day]").forEach(b => b.onclick = () => { const day = Number(b.dataset.day); if (selected.has(day)) selected.delete(day); else selected.add(day); preview(); });
    editor.querySelectorAll<HTMLButtonElement>("[data-days]").forEach(b => b.onclick = () => { selected = new Set(b.dataset.days!.split(",").map(Number)); preview(); });
    editor.querySelectorAll<HTMLElement>(".plan-fields > label").forEach(label => label.querySelector(".shell-select-trigger")?.setAttribute("aria-label",label.firstChild?.textContent ?? "选择选项"));
    const close = () => { editing = false; updateEditorStatus = undefined; editor.hidden = true; editor.innerHTML = ""; add.disabled = false; list.hidden = false; paintList(); add.focus(); };
    editor.querySelector<HTMLButtonElement>("#plan_cancel")!.onclick = close;
    editor.querySelector<HTMLFormElement>("form")!.onsubmit = async event => {
      event.preventDefault();
      const formMessage = editor.querySelector<HTMLElement>("#plan_form_msg")!;
      if (!selected.size) { formMessage.textContent = "至少选择一天"; return; }
      const submit = editor.querySelector<HTMLButtonElement>('[type="submit"]')!; submit.disabled = true;
      editor.querySelectorAll<HTMLButtonElement>("button").forEach(b => b.disabled = true);
      try {
        formMessage.textContent = existing?.account ? "正在保存…" : "正在核验当前账号，未提交预约…";
        const account = existing?.account ?? await invoke<string>("booking_plan_account");
        if ((existing?.enabled ?? true) && view.plans.rules.some(r => r.id !== existing?.id && r.enabled && r.account === account && r.sport === sport.id && r.weekdays.some(d => selected.has(d)))) {
          throw new Error("这项运动在所选星期已有计划，请修改已有计划或换一天。");
        }
        const [start, end] = slot.split("|");
        const rule: BookingPlan = { id: existing?.id ?? crypto.randomUUID(), sport: sport.id, venue: venue.id, start, end, weekdays: [...selected].sort((a, b) => a - b), enabled: existing?.enabled ?? true, account, first_date: existing?.first_date ?? "" };
        const rules = view.plans.rules.filter(r => r.id !== rule.id);
        const index = existing ? view.plans.rules.findIndex(r => r.id === rule.id) : -1;
        rules.splice(index < 0 ? rules.length : index, 0, rule);
        await save(rules);
        if (scope.current()) { close(); await load(); }
      } catch (e) { if (scope.current()) { formMessage.textContent = String(e); editor.querySelectorAll<HTMLButtonElement>("button").forEach(b => b.disabled = false); } }
    };
    preview(); editor.scrollIntoView({ block: "nearest" }); editor.querySelector<HTMLElement>(".shell-select-trigger")?.focus();
  }

  add.onclick = () => openEditor();
  pause.onclick = () => { void save(view.plans.rules, !view.plans.paused).catch(() => {}); };
  await load();
  try {
    const startup=await invoke<{supported:boolean;enabled:boolean}>("get_startup");
    if(scope.current() && startup.supported && !startup.enabled) {
      const hint=panel.querySelector<HTMLElement>("#plans_startup")!;hint.hidden=false;
      const enable=panel.querySelector<HTMLButtonElement>("#plans_enable_startup")!;
      enable.onclick=async()=>{enable.disabled=true;try{await invoke("set_startup",{enabled:true});hint.hidden=true;}catch(e){fail(e);enable.disabled=false;}};
    }
  } catch { /* Settings retains the explicit startup status and error. */ }
  for (const event of ["plans-changed", "plans-health"]) {
    const unlisten = await listen(event, () => { void load(); });
    scope.onDispose(unlisten);
  }
}
