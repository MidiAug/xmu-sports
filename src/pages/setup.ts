import { createAvailabilityStore, availabilitySessionCache } from "../lib/availability-store";
import { invoke } from "@tauri-apps/api/core";
import { hasBookingPlan, bookingPlanConflict, type PlanHints } from "../lib/booking-plan-hints";
import { bookingPlans } from "../lib/booking-plans-store";
import type { BookingTask, FallbackOrder, SelectOption, ShellPrefs, Sport, Venue } from "../types";
import {
  favoriteSports,
  findSport,
  findVenue,
  isSportEnabled,
  slotLabel,
} from "../lib/catalog";
import { formatMonthDayWeekday, formatTargetDateHuman } from "../lib/format-human";
import { setMsg } from "../lib/msg";
import {
  FIRE_TIME_MIN,
  addMinutesHm,
  bookingDateOptions,
  plannedFireAt,
  defaultFireTimeHm,
  defaultTargetDateISO,
  fireHmFromIso,
  hmToMinutes,
  isSelectableTargetDate,
  targetDateHint,
  waitsExtraDay,
  todayISO,
} from "../lib/time";
import { resolvePreferredSlot, slotPrefKey } from "../lib/venue-slot-prefs";
import { mountShellSelect } from "../ui/select";
import { mountChoiceGroup } from "../ui/choice-group";
import { sportIcon, locationIcon } from "../ui/sport-icons";
import { SVG } from "../ui/icons";
import { log } from "../lib/log";
import { mountPage } from "../lib/page-lifecycle";
import { readSavedTask, saveTaskDraft, flushTask, subscribeTaskSave } from "../lib/task-store";
import { updateVenueSlotPreference } from "../lib/prefs-store";
import { bookingBlockedReason, hangBlockedReason } from "../lib/booking-eligibility";
import { formatFireAtHuman, isFireAtPast } from "../lib/format-human";
import { askConfirm } from "../ui/confirm-ask";
import { toastError } from "../ui/toast";
import { liveStatusCache, subscribeLiveStatus } from "../state";
import { availabilityAuthNeeded, isAdminLocked, isFull, slotOption, courtOption, courtLabel, type Availability } from "../lib/availability";

export async function renderSetup(panel: Element) {
  const scope = mountPage(panel);
  const end = log.time("setup", "render");
  let saved: BookingTask | null = null;
  try {
    const endRead = log.time("setup", "read_task_raw");
    const raw = await readSavedTask();
    endRead();
    if (raw && typeof raw === "object" && "task_id" in raw) saved = raw;
  } catch (error) {
    if (!scope.current()) return;
    toastError("读取预约配置失败", String(error));
    panel.textContent = "配置未能加载，请切换页面后重试。";
    return;
  }

  let favoriteIds: string[] | undefined;
  let venueSlotPrefs: Record<string, string> = {};
  let bookingPrefs: ShellPrefs | undefined;
  try {
    const prefs = await invoke<ShellPrefs>("get_shell_prefs");
    bookingPrefs = prefs;
    favoriteIds = prefs.favorite_sport_ids;
    venueSlotPrefs = { ...(prefs.venue_slot_prefs ?? {}) };
  } catch {
    /* ignore */
  }
  if (!scope.current()) return;
  const selectable = favoriteSports(favoriteIds);

  // Boundaries refresh relative labels and timing, while preserving valid actual dates.
  const locked = Boolean(liveStatusCache?.armed || liveStatusCache?.running);
  const date =
    saved?.target_date && (locked || isSelectableTargetDate(saved.target_date))
      ? saved.target_date
      : defaultTargetDateISO();
  const fireHm = saved ? fireHmFromIso(saved.fire_at, saved.fire_delay_seconds ?? 0) : defaultFireTimeHm();
  const savedSport = saved ? findSport(saved.sport) : undefined;
  const defaultSport =
    (savedSport &&
    isSportEnabled(savedSport) &&
    selectable.some((s) => s.id === savedSport.id)
      ? savedSport
      : undefined) ??
    selectable[0];
  if (!defaultSport) {
    panel.innerHTML = `
      <div class="page-stack">
        <div class="empty history-empty">
          <p class="empty-title">还没有常用运动</p>
          <p class="empty-desc">请到设置里至少勾选一项，再回到这里配置预约。</p>
          <p class="live-brief-cta">
            <button type="button" class="btn primary" data-tab="settings">去设置</button>
          </p>
        </div>
      </div>`;
    end();
    return;
  }
  const defaultVenue =
    (saved ? findVenue(defaultSport, saved.venue) : undefined) ?? defaultSport.venues[0];
  if (saved) {
    venueSlotPrefs[defaultVenue.ocr_label] = slotPrefKey(
      saved.target_time_start,
      saved.target_time_end,
    );
  }
  const defaultSlotKey = resolvePreferredSlot(
    defaultVenue.time_slots,
    venueSlotPrefs[defaultVenue.ocr_label],
  );
  const taskId = saved?.task_id || crypto.randomUUID();
  const allowFallback = bookingPrefs?.allow_time_fallback ?? saved?.allow_time_fallback ?? false;
  const fallbackOrder: FallbackOrder = bookingPrefs?.time_fallback_order ?? saved?.time_fallback_order ?? "down_then_up";
  let busy = Boolean(liveStatusCache?.armed || liveStatusCache?.running);
  let starting = false;
  let targetDate = date;
  let fireTime = fireHm;

  panel.innerHTML = `
    <div class="page-stack setup-page">
      <div class="page-heading"><h2>新建预约</h2></div>
      <div class="task-busy-note" id="task_busy_note" ${busy ? "" : "hidden"}>当前任务已锁定，结束后可修改。<button type="button" class="btn ghost" data-tab="live">查看运行</button></div>
      <div class="setup-workspace">
      <fieldset class="task-fields" id="task_fields" ${busy ? "disabled" : ""}>
      <section class="booking-form-card">
        <div class="booking-location-row">
          <div class="booking-location-heading"><span class="booking-sport-label">运动</span>
            <div class="settings-cell booking-venue-section"><p class="settings-cell-title sr-only">场馆</p><div class="venue-picker">${locationIcon}<div id="venue_host"></div></div></div>
          </div>
          <div class="settings-cell booking-sport-section"><div id="sport_host"></div></div>
        </div>
        <div class="booking-date-section">
          <div class="booking-section-heading"><h3>运动日期</h3><span id="date_months"></span></div>
          <div class="booking-date-options" id="date_options" role="group" aria-label="预约日期"></div>
          <p class="booking-date-hint" id="date_hint"></p>
        </div>
        <div class="settings-cell booking-slot-section">
          <div class="booking-field-heading"><p class="settings-cell-title">时段</p><div class="availability-panel" id="availability_panel" aria-live="polite"></div></div>
          <div id="slot_host"></div>
        </div>
        <div class="booking-preferences"><div class="settings-cell" id="court_cell"><p class="settings-cell-title">场地偏好</p><div id="court_host"></div></div>
      <details class="task-advanced"><summary><strong>更多选项</strong></summary>
      <div class="settings-group">
        <div class="settings-group-body">
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">定时执行时间</p>
              <p class="settings-cell-desc">北京时间，最早 07:00；凌晨选今天或明天在今天执行，选后天在明天执行</p>
            </div>
            <div class="stepper" role="group" aria-label="定时执行时间">
              <button type="button" class="stepper-btn" id="fire_prev" aria-label="减一分钟">−</button>
              <span id="fire_time" class="stepper-value" aria-live="polite">${fireHm}</span>
              <button type="button" class="stepper-btn" id="fire_next" aria-label="加一分钟">+</button>
            </div>
          </div>
          <div class="settings-cell">
            <div>
              <label class="settings-cell-title" for="fire_delay">延迟执行（秒）</label>
              <p class="settings-cell-desc">在上面的时间后延迟 0–300 秒开始预约，例如 10 秒即 07:00:10；查单与提交仍需要时间</p>
            </div>
            <div class="stepper fire-delay-stepper" role="group" aria-label="延迟执行秒数">
              <button type="button" class="stepper-btn" id="delay_prev" aria-label="减少一秒">−</button>
              <input class="stepper-value fire-delay-input" id="fire_delay" type="number" min="0" max="300" step="1" inputmode="numeric" value="${saved?.fire_delay_seconds ?? 0}" />
              <button type="button" class="stepper-btn" id="delay_next" aria-label="增加一秒">+</button>
            </div>
          </div>
          <div class="settings-cell"><div><p class="settings-cell-title">候选时段</p><p class="settings-cell-desc">${allowFallback ? fallbackOrder === "up_then_down" ? "已开启 · 先更早" : "已开启 · 先更晚" : "未开启"}，单次与长期计划共用</p></div><button type="button" class="btn ghost" data-tab="settings">预约偏好</button></div>
        </div>
      </div>
      </details>
        </div>
      </section>
      </fieldset>
      <aside class="task-action-bar" aria-label="本次预约">
        <div class="task-action-copy"><span class="task-summary-label">本次预约</span>
          <p class="task-summary-date" id="summary_date"></p><strong class="task-summary-time" id="summary_time"></strong>
          <div class="task-summary-place"><strong id="summary_sport"></strong><p id="summary_venue"></p></div>
          <strong class="sr-only" id="task_summary"></strong><p id="task_execution">${SVG.tabHistory}<span></span></p><p id="task_action_reason" role="status"></p>
          <button type="button" class="btn text" id="task_plan_link" data-tab="plans" hidden>查看对应计划</button>
        </div>
        <div class="task-actions"><button type="button" class="btn primary" id="setup_primary">${SVG.tabHistory}<span>定时预约</span></button><button type="button" class="btn text" id="setup_check">检查登录</button><button type="button" class="btn text" id="setup_now">立即预约</button></div>
        <p class="msg" id="setup_msg" role="status"></p>
      </aside>
      </div>
    </div>
  `;


  const sportHost = panel.querySelector("#sport_host") as HTMLElement;
  const venueHost = panel.querySelector("#venue_host") as HTMLElement;
  const slotHost = panel.querySelector("#slot_host") as HTMLElement;
  const dateOptionsEl = panel.querySelector("#date_options") as HTMLElement;
  let dateOptionsDay = "";
  const fireTimeEl = panel.querySelector("#fire_time") as HTMLElement;
  const firePrevBtn = panel.querySelector("#fire_prev") as HTMLButtonElement;
  const fireNextBtn = panel.querySelector("#fire_next") as HTMLButtonElement;
  const fireDelayEl = panel.querySelector("#fire_delay") as HTMLInputElement;
  const delayPrevBtn = panel.querySelector<HTMLButtonElement>("#delay_prev")!;
  const delayNextBtn = panel.querySelector<HTMLButtonElement>("#delay_next")!;
  const msgEl = panel.querySelector("#setup_msg");
  const availabilityHost = panel.querySelector("#availability_panel") as HTMLElement;
  let availability: Availability | null = null;
  let availabilityKey = "";
  let availabilityGeneration = 0;
  let availabilityLoading = false;
  let availabilityError = "";
  let availabilityNeedsLogin = false;
  let availabilityRecovering = false;
  let availabilityLoginId: string | undefined;
  let availabilityTimer = 0;
  const availabilityStore=createAvailabilityStore((target,force)=>invoke<Availability>("get_venue_availability",{...target,force}),Date.now,target=>scope.current() && !busy && !availabilityRecovering && target.sport===sportSelect.getValue() && target.venue===venueSelect.getValue(),availabilitySessionCache);
  availabilityNeedsLogin=availabilityStore.needsLogin();
  const availabilityTarget=()=>({sport:sportSelect.getValue(),venue:venueSelect.getValue(),date:getTargetIso()});
  const escape = (value:string) => value.replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
  const stateKey = () => JSON.stringify([sportSelect.getValue(),venueSelect.getValue(),getTargetIso()]);

  function paintAvailability() {
    if (!scope.current()) return;
    const date=getTargetIso();
    const today=todayISO();
    const tomorrow=new Date(Date.parse(`${today}T00:00:00+08:00`)+86400_000+8*3600_000).toISOString().slice(0,10);
    if (![today,tomorrow].includes(date)) {
      availabilityHost.innerHTML='';
      availabilityHost.hidden=true;
      return;
    }
    availabilityHost.hidden=false;
    const current=availabilityKey===stateKey() ? availability : null;
    const warning=availabilityError ? `${availabilityError}${current ? "仍使用上次缓存。" : ""}`
      : current?.state==="date_not_published" ? "学校尚未发布该日期时段。"
      : availabilityStore.persistenceFailed() ? "本地缓存未保存，重载后需重新读取。"
      : current?.cache_saved===false ? "场次快照已取得，备用缓存未能保存。"
      : current?.can_choose===false ? current.notice || "学校查询时提示未开放预约。" : "";
    const label=availabilityRecovering ? "登录中" : availabilityLoading ? "刷新中" : availabilityNeedsLogin ? "重新登录" : availabilityError ? "重试" : "刷新";
    availabilityHost.innerHTML=`<button type="button" class="btn ghost" aria-label="${label==='刷新' ? '刷新场次状态' : label}" data-refresh-availability ${availabilityLoading || availabilityRecovering || busy ? "disabled" : ""}>${label}</button>${availabilityRecovering ? '<button type="button" class="btn ghost" data-cancel-availability-login>取消</button>' : ""}${warning ? `<p class="availability-warning" role="status">${escape(warning)}</p>` : ""}`;
    const refreshedAt = current?.fetched_at ? new Date(current.fetched_at) : null;
    if (refreshedAt && Number.isFinite(refreshedAt.getTime())) {
      availabilityHost.querySelector<HTMLElement>("[data-refresh-availability]")!.dataset.tooltip = `更新于 ${refreshedAt.toLocaleTimeString("zh-CN", { timeZone: "Asia/Shanghai", hour: "2-digit", minute: "2-digit" })}`;
    }
    availabilityHost.querySelector<HTMLButtonElement>("[data-refresh-availability]")?.addEventListener("click",()=>{void (availabilityNeedsLogin ? restoreAvailabilityLogin() : loadAvailability(true));});
    availabilityHost.querySelector<HTMLButtonElement>("[data-cancel-availability-login]")?.addEventListener("click",()=>{
      cancelAvailabilityLogin(); availabilityGeneration++; paintAvailability(); updateSummary();
    });
  }

  function cancelAvailabilityLogin() {
    if (availabilityLoginId) void invoke("cancel_availability_login",{operationId:availabilityLoginId}).catch(()=>{});
    availabilityLoginId=undefined; availabilityRecovering=false;
  }

  async function restoreAvailabilityLogin() {
    if (busy || availabilityLoading || availabilityRecovering) return;
    const generation=++availabilityGeneration;
    const operationId=crypto.randomUUID();
    availabilityLoginId=operationId; availabilityRecovering=true; availabilityError="";
    paintAvailability(); updateSummary();
    try {
      const result=await invoke<{ok:boolean;code?:string;phase?:string;message?:string;reason?:string}>("reauthenticate_availability",{operationId,expectedAccount:availabilitySessionCache.account ?? null});
      if (!scope.current() || generation!==availabilityGeneration) return;
      if (!result?.ok) {
        availabilityNeedsLogin=availabilityAuthNeeded(result ?? {});
        throw new Error(result?.code==="ACCOUNT_CHANGED" ? "登录账号已变化，未刷新" : result?.message || (result?.code==="HTTP_AUTH_UNVERIFIED" ? "登录身份未能核实" : "小程序导航或登录核验未完成，请查看当前页面后重试"));
      }
      availabilityNeedsLogin=false; availabilityRecovering=false; availabilityLoginId=undefined;
      window.dispatchEvent(new CustomEvent("orders-authenticated",{detail:{source:"availability"}}));
      availabilityStore.reset();
      await loadAvailability(true,false);
    } catch(error) {
      if (!scope.current() || generation!==availabilityGeneration) return;
      availabilityError=String(error).replace(/^Error:\s*/,"");
    } finally {
      if (scope.current() && availabilityLoginId===operationId) {
        availabilityLoginId=undefined; availabilityRecovering=false; paintAvailability(); updateSummary();
      }
    }
  }

  async function loadAvailability(force=false, allowRecovery=true) {
    const key=stateKey();
    const date=getTargetIso();
    const difference=(Date.parse(`${date}T00:00:00+08:00`)-Date.parse(`${todayISO()}T00:00:00+08:00`))/86400_000;
    if (busy || availabilityRecovering || ![0,1].includes(difference)) { paintAvailability(); return; }
    if (!force && availabilityNeedsLogin) {paintAvailability();return;}
    const cached=availabilityStore.peek(availabilityTarget());
    if (!force && cached) {
      availability=cached; availabilityKey=key; availabilityLoading=false;
      syncAvailabilityOptions(); paintAvailability(); updateSummary(); return;
    }
    const generation=++availabilityGeneration;
    let recoverAfterRefresh=false;
    availabilityLoading=true; availabilityError="";
    paintAvailability();
    try {
      const response=await availabilityStore.get(availabilityTarget(),force);
      if (!scope.current()) return;
      if (generation!==availabilityGeneration || key!==stateKey()) {
        // A completed shared refresh may also update the other visible date.
        const current=availabilityStore.peek(availabilityTarget());
        if (!availabilityLoading && !availabilityRecovering && current && response.snapshots?.[getTargetIso()]===current) {
          availability=current;availabilityKey=stateKey();availabilityError="";
          if(!response.cached) availabilityNeedsLogin=false;
          syncAvailabilityOptions();paintAvailability();updateSummary();
        }
        return;
      }
      if (!response?.ok) {
        availabilityNeedsLogin=availabilityAuthNeeded(response ?? {});
        recoverAfterRefresh=force && allowRecovery && availabilityNeedsLogin;
        throw new Error(availabilityNeedsLogin ? "登录已失效" : response?.message || "未取得场次状态");
      }
      availability=response; availabilityKey=key; availabilityNeedsLogin=availabilityStore.needsLogin();
      syncAvailabilityOptions();
    } catch(error) {
      if (!scope.current() || generation!==availabilityGeneration || key!==stateKey()) return;
      availabilityError=String(error).replace(/^Error:\s*/,"");
    } finally {
      if (scope.current() && generation===availabilityGeneration) {
        availabilityLoading=false; paintAvailability(); updateSummary();
        if(recoverAfterRefresh) await restoreAvailabilityLogin();
      }
    }
  }

  function syncAvailabilityOptions() {
    const vn=currentVenue();
    if (!vn) return;
    const keep=slotSelect.getValue();
    const current=availabilityKey===stateKey() ? availability : null;
    const options:SelectOption[]=current?.slots.length ? current.slots.map(slotOption) : slotOptions(vn);
    // Preserve the user's target even if a refreshed snapshot locks it.
    if (!options.some(option=>option.value===keep)) options.unshift({value:keep,label:keep.replace("|","–"),badge:"该日期未提供"});
    slotSelect.setOptions(options,keep);
    refreshCourts(vn);
  }

  function selectionBlockedReason(immediate = false):string|null {
    const sport = findSport(sportSelect.getValue());
    if (sport && hasBookingPlan(planHints, sport.id, targetDate, availabilitySessionCache.account)) return "该运动当天已有长期计划，请先跳过或停用对应计划";
    const current=availabilityKey===stateKey() ? availability : null;
    const selected=current?.slots.find(slot=>`${slot.start}|${slot.end}`===slotSelect.getValue());
    return selected && isAdminLocked(selected) ? "时段已锁，请换时段或刷新"
      : selected && courtSelect.getValue() && isAdminLocked(selected,courtSelect.getValue()) ? "场地已锁，请换场地或刷新"
      : immediate && selected && isFull(selected,courtSelect.getValue()) ? "所选场次已满，请换时段或场地" : null;
  }

  function scheduleAvailability() {
    cancelAvailabilityLogin();
    window.clearTimeout(availabilityTimer);
    // Invalidate an old selection's callback immediately, before debounce expires.
    availabilityGeneration++; availabilityLoading=false; availabilityError="";
    const cached=availabilityStore.peek(availabilityTarget());
    if(cached) {availability=cached;availabilityKey=stateKey();}
    syncAvailabilityOptions();
    paintAvailability();
    updateSummary();
    if (!cached && !availabilityNeedsLogin) availabilityTimer=window.setTimeout(()=>{void loadAvailability();},250);
  }
  const onAuthenticated=(event:Event)=>{
    if(!scope.current() || availabilityRecovering || (event as CustomEvent<{source?:string}>).detail?.source==="availability") return;
    availabilityNeedsLogin=false;availabilityError="";
    void loadAvailability(true,false);
  };
  window.addEventListener("orders-authenticated",onAuthenticated);
  scope.onDispose(()=>window.removeEventListener("orders-authenticated",onAuthenticated));
  scope.onDispose(()=>{availabilityStore.dispose();cancelAvailabilityLogin();window.clearTimeout(availabilityTimer);availabilityGeneration++;});

  type SelectApi = ReturnType<typeof mountShellSelect>;
  let sportSelect!: SelectApi;
  let planHints: PlanHints | undefined;
  const refreshPlanHints = async () => {
    try { const next = await bookingPlans.read(); if (scope.current()) { planHints = next; updateSummary(); } }
    catch (error) { log.warn("setup", "plan_hints_failed", { err: String(error) }); }
  };

  async function persistVenueSlotPrefs(venue: string) {
    try {
      await updateVenueSlotPreference(venue, venueSlotPrefs[venue] ?? null);
    } catch (e) {
      log.warn("setup", "save_venue_slot_prefs_failed", { err: String(e) });
    }
  }

  function rememberCurrentSlot() {
    const venue = venueSelect.getValue().trim();
    const slot = slotSelect.getValue();
    if (!venue || !slot) return;
    if (venueSlotPrefs[venue] === slot) return;
    venueSlotPrefs = { ...venueSlotPrefs, [venue]: slot };
    void persistVenueSlotPrefs(venue);
  }

  const venueSelect = mountShellSelect(venueHost, [], "", () => {
    refreshSlots(venueSlotPrefs[venueSelect.getValue()]);
    scheduleSave();
    scheduleAvailability();
  }, { minMenuWidth: 260, align: "end" });
  const slotSelect = mountChoiceGroup(
    slotHost,
    [],
    "",
    () => {
      rememberCurrentSlot();
      const vn=currentVenue();
      if (vn) {
        const selected=availabilityKey===stateKey() ? availability?.slots.find(slot=>`${slot.start}|${slot.end}`===slotSelect.getValue()) : undefined;
        if (selected?.cells.length && preferredCourt && !selected.cells.some(cell=>cell.court===preferredCourt)) preferredCourt="";
        refreshCourts(vn);
      }
      scheduleSave();
      paintAvailability();
    },
    { label: "时段", kind: "slot" },
  );
  const advanced = panel.querySelector<HTMLDetailsElement>(".task-advanced")!;
  const scroller = panel.closest<HTMLElement>(".content")!;
  let advancedScrollFrame = 0;
  const revealAdvanced = () => {
    window.cancelAnimationFrame(advancedScrollFrame);
    if (!advanced.open) return;
    const from = scroller.scrollTop;
    const bottom = () => scroller.scrollHeight - scroller.clientHeight;
    if (bottom() <= from + 1) return;
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      scroller.scrollTop = bottom();
      return;
    }
    const started = performance.now();
    const step = (now: number) => {
      const progress = Math.min(1, (now - started) / 500);
      const eased = progress < .5 ? 4 * progress ** 3 : 1 - (-2 * progress + 2) ** 3 / 2;
      scroller.scrollTop = from + (bottom() - from) * eased;
      if (progress < 1) advancedScrollFrame = window.requestAnimationFrame(step);
    };
    advancedScrollFrame = window.requestAnimationFrame(step);
  };
  // Availability messages can change the form height after opening the options.
  const advancedLayout = new ResizeObserver(revealAdvanced);
  advanced.addEventListener("toggle", () => {
    if (advanced.open) { advancedLayout.observe(advanced.closest(".booking-form-card")!); revealAdvanced(); }
    else { advancedLayout.disconnect(); window.cancelAnimationFrame(advancedScrollFrame); }
  });
  scope.onDispose(() => { advancedLayout.disconnect(); window.cancelAnimationFrame(advancedScrollFrame); });

  function sportOptions(): SelectOption[] {
    return selectable.map(s => ({ value: s.ocr_label, label: s.label }));
  }

  function venueOptions(sp: Sport): SelectOption[] {
    return sp.venues.map((v) => ({ value: v.ocr_label, label: v.label }));
  }

  function slotOptions(vn: Venue): SelectOption[] {
    return vn.time_slots.map((t) => ({
      value: slotPrefKey(t.start, t.end),
      label: slotLabel(t),
    }));
  }

  let preferredCourt = saved?.prefer_court ?? "";
  const courtSelect = mountShellSelect(panel.querySelector("#court_host") as HTMLElement,
    [{ value: "", label: "自动选择" }], "", (value) => {
      preferredCourt = value;
      scheduleSave();
      paintAvailability();
    }, { compact: true, minMenuWidth: 380 });

  function currentVenue(): Venue | undefined {
    const sp = findSport(sportSelect.getValue());
    return sp ? findVenue(sp, venueSelect.getValue()) : undefined;
  }

  function refreshCourts(vn: Venue) {
    const courts = vn.has_courts ? vn.courts ?? [] : [];
    const current=availabilityKey===stateKey() ? availability : null;
    const selected=current?.slots.find(slot=>`${slot.start}|${slot.end}`===slotSelect.getValue());
    const locked=!!selected && isAdminLocked(selected);
    const options:SelectOption[]=[
      {value:"",label:"自动选择",badge:locked ? "全部锁定" : undefined,disabled:locked},
      ...(selected?.cells.length ? selected.cells.map(courtOption) : courts.map(court=>({value:court.name,label:courtLabel(court.name)}))),
    ];
    if (preferredCourt && !options.some(option=>option.value===preferredCourt)) {
      // Preserve an existing target that the current response cannot confirm.
      options.push({value:preferredCourt,label:courtLabel(preferredCourt)});
    }
    courtSelect.setOptions(options,preferredCourt);
    courtSelect.setDisabled(locked);
    (panel.querySelector("#court_cell") as HTMLElement).hidden = !vn.has_courts;
  }

  function getTargetIso(): string {
    return targetDate;
  }

  function refreshSlots(prefer?: string) {
    const sp = findSport(sportSelect.getValue());
    if (!sp) return;
    const vn = findVenue(sp, venueSelect.getValue());
    if (!vn) return;
    if (preferredCourt && !(vn.courts ?? []).some(court=>court.name===preferredCourt)) preferredCourt="";
    const keep = resolvePreferredSlot(vn.time_slots, prefer ?? venueSlotPrefs[vn.ocr_label]);
    if (prefer && keep !== prefer && venueSlotPrefs[vn.ocr_label] === prefer) {
      const next = { ...venueSlotPrefs };
      delete next[vn.ocr_label];
      venueSlotPrefs = next;
      void persistVenueSlotPrefs(vn.ocr_label);
    }
    slotSelect.setOptions(slotOptions(vn), keep);
    refreshCourts(vn);
  }

  function refreshVenues(sportOcr: string, preferVenue?: string) {
    const sp = findSport(sportOcr);
    if (!sp) return;
    const prefer =
      preferVenue && sp.venues.some((v) => v.ocr_label === preferVenue)
        ? preferVenue
        : sp.venues[0].ocr_label;
    venueSelect.setOptions(venueOptions(sp), prefer);
    refreshSlots(venueSlotPrefs[prefer]);
  }

  sportSelect = mountChoiceGroup(sportHost, sportOptions(), defaultSport.ocr_label, (v) => {
    refreshVenues(v);
    scheduleSave();
    scheduleAvailability();
  }, { label: "运动", kind: "sport", icon: sportIcon });
  venueSelect.setOptions(venueOptions(defaultSport), defaultVenue.ocr_label);
  slotSelect.setOptions(slotOptions(defaultVenue), defaultSlotKey);
  refreshCourts(defaultVenue);
  venueSlotPrefs[defaultVenue.ocr_label] = defaultSlotKey;


  function syncDateOptions() {
    const today = todayISO();
    const options = bookingDateOptions();
    const sport = findSport(sportSelect.getValue())?.id ?? "";
    const arranged = (date: string) => hasBookingPlan(planHints, sport, date, availabilitySessionCache.account);
    const previousDate = targetDate;
    if (!busy && !starting && isSelectableTargetDate(targetDate) && arranged(targetDate)) {
      const available = options.filter(option => !arranged(option.value));
      const next = available.find(option => option.value > targetDate) ?? available.at(-1);
      if (next) targetDate = next.value;
    }
    if (dateOptionsDay !== today) {
      dateOptionsDay = today;
      panel.querySelector("#date_months")!.textContent = [...new Set(options.map(option => `${Number(option.value.slice(5,7))}月`))].join(" / ");
      dateOptionsEl.innerHTML = options.map(option => `
        <button type="button" class="booking-date-option" data-date="${option.value}" aria-label="${option.label} ${formatMonthDayWeekday(option.value)}" aria-pressed="false">
          <span class="date-heading"><span class="date-relative">${option.label}</span><span class="date-plan-badge" hidden>已编排</span></span>
          <span class="date-body"><span class="date-calendar"><span class="date-month">${Number(option.value.slice(5,7))}月</span><strong class="date-number">${Number(option.value.slice(8))}</strong><span class="date-unit">日</span></span><span class="date-weekday">周${"日一二三四五六"[new Date(`${option.value}T00:00:00Z`).getUTCDay()]}</span></span>
        </button>`).join("");
    }
    dateOptionsEl.querySelectorAll<HTMLButtonElement>("[data-date]").forEach(button => {
      const planned = arranged(button.dataset.date!);
      const selected = !planned && button.dataset.date === targetDate;
      button.disabled = planned;
      button.querySelector<HTMLElement>(".date-plan-badge")!.hidden = !planned;
      const option = options.find(option => option.value === button.dataset.date)!;
      button.setAttribute("aria-label", `${option.label} ${formatMonthDayWeekday(option.value)}${planned ? "，已编排" : ""}`);
      button.classList.toggle("on", selected);
      button.setAttribute("aria-pressed", String(selected));
    });
    return previousDate !== targetDate;
  }

  function syncFirePrevState() {
    const atMin = hmToMinutes(fireTime) <= hmToMinutes(FIRE_TIME_MIN);
    const atMax = hmToMinutes(fireTime) >= hmToMinutes("23:59");
    firePrevBtn.disabled = atMin;
    firePrevBtn.setAttribute("aria-disabled", String(atMin));
    fireNextBtn.disabled = atMax;
    fireNextBtn.setAttribute("aria-disabled", String(atMax));
  }

  function shiftFireTime(delta: number) {
    fireTime = addMinutesHm(fireTime, delta);
    fireTimeEl.textContent = fireTime;
    syncFirePrevState();
    scheduleSave();
  }

  dateOptionsEl.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-date]");
    const value = button?.dataset.date;
    if (busy || starting || button?.disabled || !value || !isSelectableTargetDate(value)) return;
    targetDate = value;
    syncDateOptions();
    scheduleSave();
    scheduleAvailability();
  });
  firePrevBtn.addEventListener("click", () => shiftFireTime(-1));
  fireNextBtn.addEventListener("click", () => shiftFireTime(1));
  function syncDelayButtons() {
    delayPrevBtn.disabled = fireDelayEl.valueAsNumber <= 0;
    delayNextBtn.disabled = fireDelayEl.valueAsNumber >= 300;
  }
  function shiftDelay(delta: number) {
    const current = Number.isFinite(fireDelayEl.valueAsNumber) ? Math.trunc(fireDelayEl.valueAsNumber) : 0;
    fireDelayEl.value = String(Math.min(300, Math.max(0, current + delta)));
    syncDelayButtons(); scheduleSave();
  }
  delayPrevBtn.onclick = () => shiftDelay(-1);
  delayNextBtn.onclick = () => shiftDelay(1);
  fireDelayEl.addEventListener("input", () => { syncDelayButtons(); scheduleSave(); });
  syncDelayButtons();
  syncDateOptions();
  syncFirePrevState();

  function readTask(): BookingTask {
    const [start, end] = slotSelect.getValue().split("|");
    const target_date = getTargetIso();
    const fire = fireTime;
    return {
      task_id: taskId,
      revision: saved?.revision ?? 0,
      sport: sportSelect.getValue(),
      venue: venueSelect.getValue().trim(),
      target_date,
      fire_at: plannedFireAt(target_date, fire, fireDelayEl.valueAsNumber),
      fire_delay_seconds: fireDelayEl.valueAsNumber,
      target_time_start: start,
      target_time_end: end,
      allow_time_fallback: allowFallback,
      time_fallback_order: fallbackOrder,
      prefer_court: currentVenue()?.has_courts ? courtSelect.getValue() || null : null,
      court_offset: 0,
      executor: "http",
    };
  }

  function scheduleSave() {
    updateSummary();
    if (!scope.current() || busy || !validInputs()) return;
    saveTaskDraft(readTask());
  }

  function validInputs() {
    return /^\d{4}-\d{2}-\d{2}$/.test(targetDate) && (busy || isSelectableTargetDate(targetDate))
      && Number.isFinite(Date.parse(`${targetDate}T00:00:00+08:00`))
      && /^([01]\d|2[0-3]):[0-5]\d$/.test(fireTime) && fireTime >= FIRE_TIME_MIN
      && fireDelayEl.validity.valid && Number.isInteger(fireDelayEl.valueAsNumber);
  }

  function updateSummary() {
    const dateChanged = syncDateOptions();
    const conflict = bookingPlanConflict(planHints, findSport(sportSelect.getValue())?.id ?? "", targetDate, availabilitySessionCache.account);
    const planLink = panel.querySelector<HTMLButtonElement>("#task_plan_link")!;
    planLink.hidden = !conflict;
    if (conflict?.ruleId) planLink.dataset.planId = conflict.ruleId;
    else delete planLink.dataset.planId;
    const valid = validInputs();
    if (!valid) {
      panel.querySelector("#task_action_reason")!.textContent = "请填写有效的预约日期和执行时间";
      panel.querySelectorAll<HTMLButtonElement>(".task-actions button").forEach((button) => { button.disabled = true; });
      return;
    }
    const task = readTask();
    const future = !isFireAtPast(task.fire_at);
    const blocked = valid ? (selectionBlockedReason(!future) ?? (future ? hangBlockedReason(task) : bookingBlockedReason(task))) : "请填写有效的预约日期和执行时间";
    panel.querySelector("#task_summary")!.textContent = `${task.sport}　${task.venue}　${formatTargetDateHuman(task.target_date)} ${task.target_time_start}–${task.target_time_end}`;
    panel.querySelector("#summary_date")!.textContent = `${formatTargetDateHuman(task.target_date)}　周${"日一二三四五六"[new Date(`${task.target_date}T00:00:00Z`).getUTCDay()]}`;
    panel.querySelector("#summary_time")!.textContent = `${task.target_time_start} – ${task.target_time_end}`;
    panel.querySelector("#summary_sport")!.textContent = task.sport;
    panel.querySelector("#summary_venue")!.textContent = task.venue;
    const dayOffset=(Date.parse(`${task.target_date}T00:00:00+08:00`)-Date.parse(`${todayISO()}T00:00:00+08:00`))/86400_000;
    const hint = panel.querySelector("#date_hint")!;
    hint.textContent = busy ? `已锁定：${formatTargetDateHuman(task.target_date)}` : [0,1].includes(dayOffset) ? "" : targetDateHint(new Date(), task.target_date);
    hint.classList.toggle("is-warning", !busy && waitsExtraDay(task.target_date));
    hint.setAttribute("role", "status");
    panel.querySelector("#task_execution span")!.textContent = future ? `${formatFireAtHuman(task.fire_at).replace(/ 开抢$/, "")} 定时执行` : "执行方式：立即预约";
    panel.querySelector("#task_action_reason")!.textContent = busy ? "执行配置已锁定，进度可在运行页查看" : blocked ?? "";
    const primary = panel.querySelector<HTMLButtonElement>("#setup_primary")!;
    primary.querySelector("span")!.textContent = future ? "定时预约" : "立即预约";
    primary.disabled = busy || starting || availabilityRecovering || !!blocked;
    const now = panel.querySelector<HTMLButtonElement>("#setup_now")!;
    now.hidden = !future || !!bookingBlockedReason(task);
    now.disabled = busy || starting || availabilityRecovering || !valid || !!bookingBlockedReason(task) || !!selectionBlockedReason(true);
    const check = panel.querySelector<HTMLButtonElement>("#setup_check")!;
    check.hidden = task.executor !== "http";
    check.disabled = busy || starting || availabilityRecovering || !valid;
    panel.querySelector<HTMLFieldSetElement>("#task_fields")!.disabled = busy || starting;
    panel.querySelector<HTMLElement>("#task_busy_note")!.hidden = !busy;
    if (dateChanged) {
      saveTaskDraft(task);
      scheduleAvailability();
    }
  }

  async function start(mode: "primary" | "now" | "prepare") {
    if (starting || busy || availabilityRecovering || !validInputs()) return;
    const task = readTask();
    const scheduled = mode === "primary" && !isFireAtPast(task.fire_at);
    const blocked = mode === "prepare" ? null : selectionBlockedReason(!scheduled) ?? (scheduled ? hangBlockedReason(task) : bookingBlockedReason(task));
    if (blocked) { toastError("暂时无法开始", blocked); return; }
    const trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    let dismissed = false;
    starting = true; updateSummary();
    try {
      if (mode !== "prepare" && !await askConfirm({
        title: scheduled ? "安排定时预约？" : "现在预约？",
        lead: scheduled ? "届时自动执行，请保持电脑唤醒和联网。" : "确认后将向学校提交预约。",
        summary: { label: formatMonthDayWeekday(task.target_date), value: `${task.target_time_start} – ${task.target_time_end}` },
        details: [
          { label: "运动", value: task.sport },
          { label: "场馆", value: task.venue },
          { label: "执行时间", value: scheduled ? `${formatFireAtHuman(task.fire_at).replace(/ 开抢$/, "")}（北京时间）` : "现在" },
        ],
        confirmLabel: scheduled ? "定时预约" : "立即预约",
      })) { dismissed = true; return; }
      if (!scope.current()) return;
      saveTaskDraft(task);
      await flushTask();
      if (!scope.current()) return;
      if (scheduled) await invoke("arm_hang");
      else await invoke("start_http_run", { until: mode === "prepare" ? "auth" : "submit" });
      if (scope.current()) document.querySelector<HTMLButtonElement>('.nav-tabs [data-tab="live"]')?.click();
    } catch (error) { if (scope.current()) toastError("未能开始", String(error)); }
    finally {
      starting = false;
      if (scope.current()) { updateSummary(); if (dismissed) trigger?.focus(); }
    }
  }
  panel.querySelector<HTMLButtonElement>("#setup_primary")!.onclick = () => { void start("primary"); };
  panel.querySelector<HTMLButtonElement>("#setup_now")!.onclick = () => { void start("now"); };
  panel.querySelector<HTMLButtonElement>("#setup_check")!.onclick = () => { void start("prepare"); };
  scope.onDispose(subscribeTaskSave((_state, error) => {
    if (error) setMsg(msgEl, `保存失败：${String(error)}。修改后可重试。`, "err");
    else setMsg(msgEl, "", "");
  }));
  scope.onDispose(subscribeLiveStatus((status) => {
    const nextBusy=status.running || status.armed;
    const changed=busy!==nextBusy;
    busy=nextBusy; updateSummary();
    if (changed) { paintAvailability(); if (!busy) scheduleAvailability(); }
  }));
  const clock = window.setInterval(() => { if (scope.current()) updateSummary(); }, 1000);
  scope.onDispose(() => window.clearInterval(clock));
  updateSummary();
  scheduleAvailability();
  void refreshPlanHints();
  scope.onDispose(bookingPlans.subscribe(snapshot => {
    if (snapshot.view && !snapshot.error) { planHints = snapshot.view; updateSummary(); }
  }));
  end();
}
