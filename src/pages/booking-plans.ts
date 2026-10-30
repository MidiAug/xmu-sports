import { invoke } from "@tauri-apps/api/core";
import { sports, isSportEnabled } from "../lib/catalog";
import { mountPage } from "../lib/page-lifecycle";
import { mountShellSelect } from "../ui/select";
import { askConfirm } from "../ui/confirm-ask";
import { outcomeFromEvent } from "../lib/run-outcome";
import { formatEventTimeHuman, formatMonthDay, formatMonthDayWeekday } from "../lib/format-human";
import { WEEKDAYS as days, weekdaysLabel, nextPlanRound, restorablePlanSkips, type BookingPlan, type BookingPlansView } from "../lib/booking-plans";
import { bookingPlans } from "../lib/booking-plans-store";
import { SVG } from "../ui/icons";
import { sportIcon } from "../ui/sport-icons";
import { availabilitySessionCache } from "../lib/availability-store";
import { patchHtml } from "../ui/patch-html";
import { skipPlanOnce, restorePlanSkip } from "../ui/plan-skip";

const escape = (text: string) => text.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]!));
export async function renderBookingPlans(panel: Element, options?: { focusPlanId?: string }) {
  const scope = mountPage(panel);
  let view: BookingPlansView;
  let editing = false, saving = false;
  let updateEditorStatus: (() => void) | undefined;
  panel.innerHTML = `<div class="page-stack plans-page">
    <div class="page-heading"><div><h2>长期计划</h2><p>安排每周运动，前一天 07:00 自动预约</p></div><button type="button" class="btn primary" id="plan_add" disabled>新增计划</button></div>
    <section id="plan_attention" class="plan-attention" aria-label="需要处理" hidden></section>
    <section class="plans-summary" aria-label="自动预约状态"><div class="plan-state"><div class="plan-state-heading"><span class="plan-state-dot" aria-hidden="true"></span><strong id="plans_state">正在读取计划</strong><span id="plans_count"></span></div><p id="plans_health" class="plan-secondary"></p></div><div class="plan-next"><span id="plans_next_label">下次开抢</span><strong id="plans_next">正在读取</strong><p id="plans_exercise"></p></div><button type="button" class="btn ghost" id="plans_pause" hidden></button></section>
    <div id="plans_startup" class="plan-startup-hint" hidden><span>登录电脑后继续自动预约</span><button type="button" class="btn text" id="plans_enable_startup">开启自启</button></div>
    <p class="msg" id="plans_msg" role="status"></p>
    <button type="button" class="btn ghost" id="plans_retry" hidden>重新读取计划</button>
    <div id="plan_editor" hidden></div><div class="plan-list" id="plan_list"></div>
    <section id="plan_results" aria-label="最近执行"></section>
    <p class="plan-footnote">自动预约需要电脑保持唤醒、联网。关闭到托盘可继续运行；退出或关机期间不执行，重新打开后继续。</p>
  </div>`;
  const message = panel.querySelector<HTMLElement>("#plans_msg")!;
  const editor = panel.querySelector<HTMLElement>("#plan_editor")!;
  const list = panel.querySelector<HTMLElement>("#plan_list")!;
  const add = panel.querySelector<HTMLButtonElement>("#plan_add")!;
  const pause = panel.querySelector<HTMLButtonElement>("#plans_pause")!;
  const setText = (selector: string, value: string) => {
    const element = panel.querySelector(selector)!;
    if (element.textContent !== value) element.textContent = value;
  };
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

  const load = async (force = false) => {
    try {
      const next = await bookingPlans.read(force);
      if (!scope.current() || view === next) return;
      view = next;
      panel.querySelector<HTMLElement>("#plans_retry")!.hidden = true;
      if (message.classList.contains("err")) { message.textContent = ""; message.className = "msg"; }
      const enabled = view.plans.rules.filter(r => r.enabled).length;
      const pending = view.plans.attempts.filter(a => a.status === "unconfirmed");
      panel.querySelector<HTMLElement>(".plans-page")!.dataset.empty = String(!view.plans.rules.length && !pending.length);
      const round = nextPlanRound(view);
      setText("#plans_state", view.plans.paused ? "自动计划已暂停" : view.plans.rules.some(r => r.enabled) ? "自动计划已开启" : "还没有启用的计划");
      panel.querySelector(".plans-summary")!.setAttribute("data-state", pending.length ? "attention" : view.plans.paused || !enabled ? "paused" : "enabled");
      setText("#plans_count", enabled ? `${enabled} 项启用` : "");
      setText("#plans_health", `微信${view.wechat_running ? "正在运行" : "未检测到运行"}，会话将在执行前核验`);
      setText("#plans_next_label", view.plans.paused ? "恢复后按计划开抢" : round?.items.some(i => i.status === "running") ? "当前轮次" : "下次开抢");
      setText("#plans_next", round ? formatEventTimeHuman(round.fireAt) : "等待启用计划");
      const roundSports = round ? [...new Set(round.items.map(item => sports.find(s => s.id === item.sport || s.ocr_label === item.sport)?.label ?? item.sport))].join("、") : "";
      setText("#plans_exercise", round ? `预约 ${formatMonthDayWeekday(round.date)} 的${roundSports}` : "添加运动、场馆和时段即可开始");
      pause.hidden = !view.plans.rules.length;
      setText("#plans_pause", view.plans.paused ? "恢复计划" : "全部暂停");
      pause.disabled = view.plans.paused && pending.length > 0;
      add.disabled = editing;
      updateEditorStatus?.();
      if (!editing) paintList();
      paintResults();
    } catch (e) { fail(e); if (scope.current()) panel.querySelector<HTMLElement>("#plans_retry")!.hidden = false; }
  };

  const save = async (rules: BookingPlan[], paused = view.plans.paused) => {
    if (saving) return;
    saving = true;
    add.setAttribute("aria-disabled", "true");
    pause.setAttribute("aria-disabled", "true");
    try {
      await invoke("save_booking_plans", { rules, paused });
      message.textContent = "";
      await load(true);
    } catch (e) { fail(e); throw e; }
    finally { saving = false; add.removeAttribute("aria-disabled"); pause.removeAttribute("aria-disabled"); add.disabled = editing; pause.disabled = view.plans.paused && view.plans.attempts.some(a => a.status === "unconfirmed"); updateEditorStatus?.(); }
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
      list.innerHTML = `<section class="plan-empty" aria-label="开始安排运动"><div class="plan-empty-art" aria-hidden="true"><svg viewBox="0 0 80 80" fill="none"><rect x="13" y="16" width="54" height="52" rx="12"/><path d="M13 32h54M28 11v12M52 11v12"/><path class="plan-empty-check" d="m29 49 8 8 16-18"/></svg><span class="plan-empty-sport sport-icon" data-sport="羽毛球">${sportIcon("羽毛球")}</span></div><h3>安排你的每周运动</h3><button type="button" class="btn primary" id="plan_example"><span aria-hidden="true">＋</span>添加第一条计划</button></section>`;
      list.querySelector<HTMLButtonElement>("#plan_example")!.onclick = () => openEditor();
      return;
    }
    patchHtml(list, `<div class="plan-list-heading"><h3>每周安排 <span>${view.plans.rules.length}</span></h3><span>按序预约，可在更多中调整</span></div>` + view.plans.rules.map((r, index) => {
      const sport = sports.find(s => s.id === r.sport)!;
      const venue = sport?.venues.find(v => v.id === r.venue);
      const next = view.next.find(n => n.id === r.id);
      const skipped = restorablePlanSkips(view, r);
      return `<article class="plan-card" draggable="true" data-plan="${r.id}" data-enabled="${r.enabled}">
        <div class="plan-card-top"><span class="plan-priority plan-sport-icon sport-icon" data-sport="${escape(sport?.label ?? r.sport)}" aria-label="优先级 ${index + 1}">${sportIcon(sport?.label ?? r.sport)}<small>${index + 1}</small></span><div class="plan-title"><strong>${escape(sport?.label ?? r.sport)}</strong><span>${escape(venue?.label ?? r.venue)}</span></div>
        <label class="switch plan-enable"><input type="checkbox" aria-label="启用${escape(sport?.label ?? r.sport)}计划" data-enable="${r.id}" ${r.enabled ? "checked" : ""}></label></div>
        <div class="plan-schedule"><strong>${r.start}<span class="plan-time-divider">–</span>${r.end}</strong><span>${weekdaysLabel(r.weekdays)}</span></div>
        <div class="plan-week-strip" aria-label="重复星期：${weekdaysLabel(r.weekdays)}">${days.map((day, i) => `<span data-selected="${r.weekdays.includes(i + 1)}" aria-hidden="true">${day}</span>`).join("")}</div>
        <div class="plan-dates">${next ? `<div><span>下次运动</span><strong>${formatMonthDayWeekday(next.date)}</strong></div><div><span>${view.plans.paused ? "恢复后按计划开抢" : "自动开抢"}</span><strong>${escape(formatEventTimeHuman(next.fire_at))}</strong></div>` : `<div><span>本计划已停用</span><strong>启用后继续安排</strong></div>`}</div>
        <div class="plan-actions">${skipped.length ? `<div class="plan-skips"><span class="plan-skips-label">已跳过</span><div class="plan-skip-chips">${skipped.map(a => `<span class="plan-skip-chip"><span>${formatMonthDay(a.task.target_date)}</span><button type="button" data-restore-plan="${r.id}" data-restore-date="${escape(a.task.target_date)}" data-tooltip="恢复本次" aria-label="恢复${formatMonthDayWeekday(a.task.target_date)}的${escape(sport?.label ?? r.sport)}预约">${SVG.undo}</button></span>`).join("")}</div></div>` : ""}<button type="button" class="btn text" data-edit="${r.id}" aria-label="编辑计划" data-tooltip="编辑">${SVG.edit}</button>
        <details class="plan-more"><summary data-tooltip="更多" aria-label="更多操作">${SVG.more}</summary><div><button type="button" data-skip="${r.id}" ${!next ? "disabled" : ""}>${next ? `跳过 ${formatMonthDayWeekday(next.date)}` : "跳过下一次"}</button><button type="button" data-up="${r.id}" ${index === 0 ? "disabled" : ""}>上移</button><button type="button" data-down="${r.id}" ${index === view.plans.rules.length - 1 ? "disabled" : ""}>下移</button><button type="button" data-delete="${r.id}">删除</button></div></details></div>
      </article>`;
    }).join(""));
    list.querySelectorAll<HTMLInputElement>("[data-enable]").forEach(box => {
      box.checked = view.plans.rules.find(r => r.id === box.dataset.enable)!.enabled;
      box.onchange = () => {
        if (saving) { box.checked = view.plans.rules.find(r => r.id === box.dataset.enable)!.enabled; return; }
        void save(view.plans.rules.map(r => r.id === box.dataset.enable ? { ...r, enabled: box.checked } : r)).catch(() => { box.checked = !box.checked; });
      };
    });
    list.querySelectorAll<HTMLButtonElement>("[data-edit]").forEach(b => b.onclick = () => openEditor(view.plans.rules.find(r => r.id === b.dataset.edit)));
    list.querySelectorAll<HTMLButtonElement>("[data-delete]").forEach(b => b.onclick = async () => {
      if (await askConfirm({ title: "删除这条计划？", lead: "已成功预约的订单会保留，之后不再自动安排。", confirmLabel: "删除计划" })) {
        void save(view.plans.rules.filter(r => r.id !== b.dataset.delete)).catch(() => {});
      }
    });
    list.querySelectorAll<HTMLButtonElement>("[data-skip]").forEach(b => b.onclick = async () => {
      b.disabled = true;
      try { await skipPlanOnce(b.dataset.skip!); } catch (e) { fail(e); b.disabled = false; }
    });
    list.querySelectorAll<HTMLButtonElement>("[data-restore-plan]").forEach(button => button.onclick = async () => {
      button.disabled = true;
      try { await restorePlanSkip(button.dataset.restorePlan!, button.dataset.restoreDate!); }
      catch (error) { fail(error); button.disabled = false; }
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
    const pending = view.plans.attempts.filter(a => a.status === "unconfirmed");
    const attention = panel.querySelector<HTMLElement>("#plan_attention")!;
    attention.hidden = !pending.length;
    attention.innerHTML = pending.length ? `<div class="plan-attention-heading"><h3>${pending.length} 项预约结果待确认</h3><span>需要处理</span></div><p>请先在我的预约或小程序核对结果，再恢复自动计划。</p>` + pending.map(a => `<div class="plan-attention-item"><div><strong>${escape(a.task.sport)} ${formatMonthDayWeekday(a.task.target_date)}</strong><details class="plan-attention-details"><summary>查看原因</summary><p>${escape(a.message)}</p></details></div><button type="button" class="btn" data-resolve="${a.task.task_id}">已核对并跳过</button></div>`).join("") : "";
    const attempts = view.plans.attempts.filter(a => !["unconfirmed", "queued", "running"].includes(a.status)).slice(-5).reverse();
    const labels: Record<string, string> = { queued: "等待执行", missed: "已错过", running: "准备 / 执行中", succeeded: "已约到", existing: "已有预约", failed: "未约到", skipped: "已跳过", cancelled: "已停止", unconfirmed: "待确认" };
    panel.querySelector("#plan_results")!.innerHTML = attempts.length ? `<h3>最近执行</h3><div class="plan-results">` + attempts.map(a => {
      const detail = a.status === "failed" ? outcomeFromEvent({ ts: "", task_id: a.task.task_id, action: "fail", result: a.message })?.title ?? "请查看运行记录" : a.status === "succeeded" ? "" : a.message;
      return `<div class="plan-result"><div><strong>${escape(a.task.sport)} ${formatMonthDayWeekday(a.task.target_date)}</strong><span>${escape(detail)}</span></div><span class="plan-result-status" data-status="${a.status}">${labels[a.status] ?? "待处理"}</span></div>`;
    }).join("") + `</div>` : "";
    panel.querySelectorAll<HTMLButtonElement>("[data-resolve]").forEach(button => button.onclick = async () => {
      if (await askConfirm({title:"已在我的预约或小程序核对结果？",lead:"该项会保留为已跳过，不会标记为预约成功，也不会自动再次下单。",confirmLabel:"已核对，跳过本项"})) {
        button.disabled = true;
        try {await invoke("resolve_booking_plan",{taskId:button.dataset.resolve});await load(true);} catch(e) {fail(e);button.disabled=false;}
      }
    });
  }

  function openEditor(existing?: BookingPlan) {
    if (saving || !view) return;
    editing = true; editor.hidden = false; add.disabled = true; list.hidden = true;
    panel.querySelector(".plans-page")!.setAttribute("data-editing", "true");
    const enabledSports = sports.filter(isSportEnabled);
    let sport = enabledSports.find(s => s.id === existing?.sport) ?? enabledSports[0];
    let venue = sport.venues.find(v => v.id === existing?.venue) ?? sport.venues[0];
    let selected = new Set(existing?.weekdays ?? [1, 2, 3, 4, 5, 6, 7]);
    let editorAccount = existing?.account ?? availabilitySessionCache.account;
    let submitting = false;
    let accountRequest: Promise<string> | undefined;
    const verifyAccount = () => accountRequest ??= invoke<string>("booking_plan_account").catch(error => { accountRequest = undefined; throw error; });
    let slot = existing ? `${existing.start}|${existing.end}` : `${venue.time_slots[0].start}|${venue.time_slots[0].end}`;
    editor.innerHTML = `<form class="plan-editor"><div class="plan-editor-main"><h3>${existing ? "编辑计划" : "新增计划"}</h3><div class="plan-fields"><label>运动<div id="plan_sport"></div></label><label>场馆<div id="plan_venue"></div></label><label>时段<div id="plan_slot"></div></label></div>
      <fieldset><legend>去运动的星期</legend><div class="plan-presets"><button type="button" data-days="1,2,3,4,5,6,7">每天</button><button type="button" data-days="1,2,3,4,5">工作日</button><button type="button" data-days="6,7">周末</button></div><div class="plan-weekdays">${days.map((d, i) => `<button type="button" data-day="${i + 1}" aria-label="周${d}" aria-pressed="${selected.has(i + 1)}">${d}</button>`).join("")}</div></fieldset>
      </div><aside class="plan-editor-summary"><span class="plan-editor-eyebrow">前一天自动开抢</span><strong class="plan-editor-fire">07:00</strong><p class="plan-preview" id="plan_preview"></p><p class="plan-preview" id="plan_paused_hint" hidden>自动计划已暂停，保存后仍需点击“恢复计划”。</p><p class="msg" id="plan_form_msg" role="status"></p><div class="plan-actions"><button type="submit" class="btn primary"></button><button type="button" class="btn ghost" id="plan_cancel">取消</button></div><p class="plan-editor-condition">预约时保持电脑唤醒、联网</p></aside></form>`;
    updateEditorStatus = () => {
      editor.querySelector<HTMLElement>("#plan_paused_hint")!.hidden = !view.plans.paused;
      editor.querySelector<HTMLButtonElement>('[type="submit"]')!.textContent = existing || view.plans.paused ? "保存计划" : "保存并启用";
      preview();
    };
    const preview = () => {
      editor.querySelectorAll<HTMLButtonElement>("[data-day]").forEach(b => b.setAttribute("aria-pressed", String(selected.has(Number(b.dataset.day)))));
      editor.querySelectorAll<HTMLButtonElement>("[data-days]").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.days!.split(",").map(Number).every(d => selected.has(d)) && b.dataset.days!.split(",").length === selected.size)));
      const options = slots();
      if (!options.some(option => option.value === slot && !option.disabled)) {
        slot = options.find(option => !option.disabled)?.value ?? slot;
      }
      const unavailable = options.find(option => option.value === slot)?.disabled;
      slotSelect.setOptions(options, slot);
      editor.querySelector<HTMLButtonElement>('[type="submit"]')!.disabled = saving || submitting || !selected.size || !!unavailable;
      editor.querySelector("#plan_preview")!.textContent = !selected.size ? "至少选择一天" : unavailable ? "时段均已编排，请调整星期" : `${weekdaysLabel([...selected].sort((a, b) => a - b))} · ${slot.replace("|", "–")}`;
    };
    const slots = () => venue.time_slots.map(t => {
      const account = editorAccount;
      const arranged = (existing?.enabled ?? true) && !!account && view.plans.rules.some(r =>
        r.id !== existing?.id && r.enabled && r.account === account && r.sport === sport.id
        && r.weekdays.some(day => selected.has(day)) && r.start < t.end && t.start < r.end);
      return { value: `${t.start}|${t.end}`, label: `${t.start}–${t.end}`, badge: arranged ? "已编排" : undefined, disabled: arranged };
    });
    const slotSelect = mountShellSelect(editor.querySelector<HTMLElement>("#plan_slot")!, slots(), slot, value => { slot = value; preview(); });
    const changeVenue = () => { slot = slots()[0].value; slotSelect.setOptions(slots(), slot); preview(); };
    const venueSelect = mountShellSelect(editor.querySelector<HTMLElement>("#plan_venue")!, sport.venues.map(v => ({ value: v.id, label: v.label })), venue.id, value => { venue = sport.venues.find(v => v.id === value)!; changeVenue(); });
    mountShellSelect(editor.querySelector<HTMLElement>("#plan_sport")!, enabledSports.map(s => ({ value: s.id, label: s.label })), sport.id, value => { sport = enabledSports.find(s => s.id === value)!; venue = sport.venues[0]; venueSelect.setOptions(sport.venues.map(v => ({ value: v.id, label: v.label })), venue.id); changeVenue(); });
    editor.querySelectorAll<HTMLButtonElement>("[data-day]").forEach(b => b.onclick = () => { const day = Number(b.dataset.day); if (selected.has(day)) selected.delete(day); else selected.add(day); preview(); });
    editor.querySelectorAll<HTMLButtonElement>("[data-days]").forEach(b => b.onclick = () => { selected = new Set(b.dataset.days!.split(",").map(Number)); preview(); });
    editor.querySelectorAll<HTMLElement>(".plan-fields > label").forEach(label => label.querySelector(".shell-select-trigger")?.setAttribute("aria-label",label.firstChild?.textContent ?? "选择选项"));
    const close = () => {
      editing = false; updateEditorStatus = undefined; editor.hidden = true; editor.innerHTML = "";
      panel.querySelector(".plans-page")!.removeAttribute("data-editing");
      add.disabled = false; list.hidden = false; paintList(); add.focus();
    };
    editor.querySelector<HTMLButtonElement>("#plan_cancel")!.onclick = close;
    editor.querySelector<HTMLFormElement>("form")!.onsubmit = async event => {
      event.preventDefault();
      if (submitting) return;
      const formMessage = editor.querySelector<HTMLElement>("#plan_form_msg")!;
      if (!selected.size) { formMessage.textContent = "至少选择一天"; return; }
      submitting = true;
      const submit = editor.querySelector<HTMLButtonElement>('[type="submit"]')!; submit.disabled = true;
      editor.querySelectorAll<HTMLButtonElement>("button").forEach(b => b.disabled = true);
      try {
        formMessage.textContent = existing?.account ? "正在保存…" : "正在核验当前账号，未提交预约…";
        const account = existing?.account ?? await verifyAccount();
        formMessage.textContent = "正在保存计划…";
        if ((existing?.enabled ?? true) && view.plans.rules.some(r => r.id !== existing?.id && r.enabled && r.account === account && r.sport === sport.id && r.weekdays.some(d => selected.has(d)))) {
          throw new Error("这项运动在所选星期已有计划，请修改已有计划或换一天。");
        }
        const [start, end] = slot.split("|");
        const rule: BookingPlan = { id: existing?.id ?? crypto.randomUUID(), sport: sport.id, venue: venue.id, start, end, weekdays: [...selected].sort((a, b) => a - b), enabled: existing?.enabled ?? true, account, first_date: existing?.first_date ?? "" };
        const rules = view.plans.rules.filter(r => r.id !== rule.id);
        const index = existing ? view.plans.rules.findIndex(r => r.id === rule.id) : -1;
        rules.splice(index < 0 ? rules.length : index, 0, rule);
        await save(rules);
        if (scope.current()) close();
      } catch (e) { if (scope.current()) { formMessage.textContent = String(e); editor.querySelectorAll<HTMLButtonElement>("button").forEach(b => b.disabled = false); } }
      finally { submitting = false; if (scope.current() && editing) preview(); }
    };
    if (!existing) {
      const form = editor.querySelector("form")!;
      void verifyAccount().then(account => {
        if (scope.current() && form.isConnected) { editorAccount = account; preview(); }
      }).catch(() => { /* Saving retains the explicit login check and error. */ });
    }
    updateEditorStatus(); editor.scrollIntoView({ block: "nearest" }); editor.querySelector<HTMLElement>(".shell-select-trigger")?.focus();
  }

  add.onclick = () => openEditor();
  panel.querySelector<HTMLButtonElement>("#plans_retry")!.onclick = () => { void load(true); };
  pause.onclick = () => { void save(view.plans.rules, !view.plans.paused).catch(() => {}); };
  scope.onDispose(bookingPlans.subscribe(snapshot => {
    if (snapshot.error) { fail(snapshot.error); panel.querySelector<HTMLElement>("#plans_retry")!.hidden = false; }
    else void load();
  }));
  await load();
  if (scope.current() && options?.focusPlanId) {
    const target = [...list.querySelectorAll<HTMLElement>("[data-plan]")].find(card => card.dataset.plan === options.focusPlanId);
    target?.scrollIntoView({ block: "nearest" });
    target?.querySelector<HTMLButtonElement>("[data-edit]")?.focus();
  }
  try {
    const startup=await invoke<{supported:boolean;enabled:boolean}>("get_startup");
    if(scope.current() && startup.supported && !startup.enabled) {
      const hint=panel.querySelector<HTMLElement>("#plans_startup")!;hint.hidden=false;
      const enable=panel.querySelector<HTMLButtonElement>("#plans_enable_startup")!;
      enable.onclick=async()=>{enable.disabled=true;try{await invoke("set_startup",{enabled:true});hint.hidden=true;}catch(e){fail(e);enable.disabled=false;}};
    }
  } catch { /* Settings retains the explicit startup status and error. */ }
}
