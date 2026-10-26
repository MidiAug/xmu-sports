import { createAvailabilityStore, availabilitySessionCache } from "../lib/availability-store";
import { invoke } from "@tauri-apps/api/core";
import type { BookingTask, FallbackOrder, SelectOption, ShellPrefs, Sport, Venue } from "../types";
import {
  favoriteSports,
  findSport,
  findVenue,
  isSportEnabled,
  slotLabel,
} from "../lib/catalog";
import { formatMonthDay, formatTargetDateHuman } from "../lib/format-human";
import { setMsg } from "../lib/msg";
import {
  FIRE_TIME_MIN,
  addMinutesHm,
  bookingDateOptions,
  composeFireAt,
  defaultFireTimeHm,
  defaultTargetDateISO,
  fireHmFromIso,
  hmToMinutes,
  isSelectableTargetDate,
  targetDateHint,
  todayISO,
} from "../lib/time";
import { resolvePreferredSlot, slotPrefKey } from "../lib/venue-slot-prefs";
import { mountShellSelect } from "../ui/select";
import { log } from "../lib/log";
import { mountPage } from "../lib/page-lifecycle";
import { readSavedTask, saveTaskDraft, flushTask, subscribeTaskSave } from "../lib/task-store";
import { updateVenueSlotPreference } from "../lib/prefs-store";
import { bookingBlockedReason, hangBlockedReason } from "../lib/booking-eligibility";
import { formatFireAtHuman, isFireAtPast } from "../lib/format-human";
import { askConfirm } from "../ui/confirm-ask";
import { toastError } from "../ui/toast";
import { liveStatusCache, subscribeLiveStatus } from "../state";
import { availabilityAuthNeeded, isAdminLocked, slotOption, courtOption, courtLabel, type Availability } from "../lib/availability";

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
  try {
    const prefs = await invoke<ShellPrefs>("get_shell_prefs");
    favoriteIds = prefs.favorite_sport_ids;
    venueSlotPrefs = { ...(prefs.venue_slot_prefs ?? {}) };
  } catch {
    /* ignore */
  }
  if (!scope.current()) return;
  const selectable = favoriteSports(favoriteIds);

  // Boot / 00:00 / 07:00 calibration selects the next release; tab changes keep manual choices.
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
  let fallbackOrder: FallbackOrder =
    saved?.time_fallback_order === "up_then_down" ? "up_then_down" : "down_then_up";
  let busy = Boolean(liveStatusCache?.armed || liveStatusCache?.running);
  let starting = false;
  let targetDate = date;
  let fireTime = fireHm;

  panel.innerHTML = `
    <div class="page-stack setup-page">
      <div class="page-heading"><div><h2>预约场馆</h2><p>选好运动时间，立即预约或安排定时执行</p></div></div>
      <div class="task-busy-note" id="task_busy_note" ${busy ? "" : "hidden"}>当前任务已锁定，结束后可修改。<button type="button" class="btn ghost" data-tab="live">查看运行</button></div>
      <fieldset class="task-fields" id="task_fields" ${busy ? "disabled" : ""}>
      <section class="booking-form-card">
        <div class="booking-date-section">
          <div class="booking-section-heading"><h3>预约日期</h3><span>北京时间</span></div>
          <div class="booking-date-options" id="date_options" role="group" aria-label="预约日期"></div>
          <p class="booking-date-hint" id="date_hint"></p>
        </div>
        <div class="settings-group-body task-grid">
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">运动</p>
            </div>
            <div id="sport_host"></div>
          </div>
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">场馆</p>
            </div>
            <div id="venue_host"></div>
          </div>
          <div class="settings-cell">
            <div class="booking-field-heading">
              <p class="settings-cell-title">时段</p>
              <div class="availability-panel" id="availability_panel" aria-live="polite"></div>
            </div>
            <div id="slot_host"></div>
          </div>
          <div class="settings-cell" id="court_cell">
            <div class="booking-field-heading">
              <p class="settings-cell-title">场地</p>
            </div>
            <div id="court_host"></div>
          </div>
        </div>
      </section>

      <details class="task-advanced"><summary><strong>高级选项</strong><span>执行时间与备选策略</span></summary>
      <div class="settings-group">
        <div class="settings-group-body">
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">定时执行时间</p>
              <p class="settings-cell-desc">预约日前一天执行，北京时间（最早 07:00）</p>
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
            <input class="settings-control fire-delay-input" id="fire_delay" type="number" min="0" max="300" step="1" value="${saved?.fire_delay_seconds ?? 0}" />
          </div>
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">目标时段满了就约邻近时段</p>
              <p class="settings-cell-desc">优先仍是你选的时段</p>
            </div>
            <label class="switch" title="邻近时段">
              <input type="checkbox" id="allow_fallback" aria-label="邻近时段" ${saved?.allow_time_fallback ? "checked" : ""} />
            </label>
          </div>
          <div class="settings-cell" id="fallback_order_wrap" ${saved?.allow_time_fallback ? "" : "hidden"}>
            <div>
              <p class="settings-cell-title">邻近顺序</p>
              <p class="settings-cell-desc">同样近时，先试更早还是更晚</p>
            </div>
            <div class="seg-shell">
              <div class="seg" id="fallback_order_seg">
                <button type="button" data-order="up_then_down"${
                  fallbackOrder === "up_then_down" ? ' class="on"' : ""
                }>先更早</button>
                <button type="button" data-order="down_then_up"${
                  fallbackOrder === "down_then_up" ? ' class="on"' : ""
                }>先更晚</button>
              </div>
            </div>
          </div>
        </div>
      </div>
      </details>
      </fieldset>
      <p class="msg" id="setup_msg"></p>
      <div class="task-action-bar"><div class="task-action-copy"><span class="task-summary-label">本次预约</span><strong id="task_summary"></strong><p id="task_execution"></p><p id="task_action_reason" role="status"></p></div><div class="task-actions"><button type="button" class="btn ghost" id="setup_check" title="检测并尝试恢复体育登录，不提交预约；可在目标日期开放前使用">检查登录</button><button type="button" class="btn ghost" id="setup_now">立即预约</button><button type="button" class="btn primary" id="setup_primary">定时预约</button></div></div>
    </div>
  `;

  const allowEl = panel.querySelector("#allow_fallback") as HTMLInputElement;
  const orderWrap = panel.querySelector("#fallback_order_wrap") as HTMLElement;
  const orderSeg = panel.querySelector("#fallback_order_seg") as HTMLElement;

  const sportHost = panel.querySelector("#sport_host") as HTMLElement;
  const venueHost = panel.querySelector("#venue_host") as HTMLElement;
  const slotHost = panel.querySelector("#slot_host") as HTMLElement;
  const dateOptionsEl = panel.querySelector("#date_options") as HTMLElement;
  let dateOptionsDay = "";
  const fireTimeEl = panel.querySelector("#fire_time") as HTMLElement;
  const firePrevBtn = panel.querySelector("#fire_prev") as HTMLButtonElement;
  const fireNextBtn = panel.querySelector("#fire_next") as HTMLButtonElement;
  const fireDelayEl = panel.querySelector("#fire_delay") as HTMLInputElement;
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
    const stamp=current?.fetched_at ? new Date(current.fetched_at).toLocaleString("zh-CN",{timeZone:"Asia/Shanghai",hour12:false}) : "";
    const hint=current ? `上次获取：${stamp}（北京时间）\n按日期缓存，跨天复用。人数是查询时快照；点击刷新更新状态，连续手动刷新间隔5秒。` : "只读获取学校场次状态";
    const warning=availabilityError ? `${availabilityError}${current ? "仍使用上次缓存。" : ""}`
      : current?.state==="date_not_published" ? "学校尚未发布该日期时段。"
      : availabilityStore.persistenceFailed() ? "本地缓存未保存，重载后需重新读取。"
      : current?.cache_saved===false ? "场次快照已取得，备用缓存未能保存。"
      : current?.can_choose===false ? current.notice || "学校查询时提示未开放预约。" : "";
    const label=availabilityRecovering ? "登录中" : availabilityLoading ? "刷新中" : availabilityNeedsLogin ? "重新登录" : availabilityError ? "重试" : "刷新";
    availabilityHost.innerHTML=`<button type="button" class="btn ghost" title="${escape(availabilityNeedsLogin ? "恢复微信登录，成功后自动刷新；不提交预约" : hint)}" aria-label="${label==='刷新' ? '刷新场次状态' : label}" data-refresh-availability ${availabilityLoading || availabilityRecovering || busy ? "disabled" : ""}>${label}</button>${availabilityRecovering ? '<button type="button" class="btn ghost" data-cancel-availability-login>取消</button>' : ""}${warning ? `<p class="availability-warning" title="${escape(availabilityError)}" role="status">${escape(warning)}</p>` : ""}`;
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
      const result=await invoke<{ok:boolean;code?:string;phase?:string;message?:string;reason?:string}>("reauthenticate_availability",{operationId});
      if (!scope.current() || generation!==availabilityGeneration) return;
      if (!result?.ok) {
        availabilityNeedsLogin=availabilityAuthNeeded(result ?? {});
        throw new Error(result?.code==="ACCOUNT_CHANGED" ? "登录账号已变化，未刷新" : result?.reason==="BACKGROUND_LOGIN_UNAVAILABLE" ? "请显示小程序并完成登录" : result?.code==="LOGIN_CONFIRM_REQUIRED" ? "请在微信完成登录确认" : result?.code==="HTTP_AUTH_UNVERIFIED" ? "登录身份未能核实" : "登录未完成，请重试");
      }
      availabilityNeedsLogin=false; availabilityRecovering=false; availabilityLoginId=undefined;
      window.dispatchEvent(new Event("orders-authenticated"));
      availabilityStore.reset();
      await loadAvailability(true);
    } catch(error) {
      if (!scope.current() || generation!==availabilityGeneration) return;
      availabilityError=String(error).replace(/^Error:\s*/,"");
    } finally {
      if (scope.current() && availabilityLoginId===operationId) {
        availabilityLoginId=undefined; availabilityRecovering=false; paintAvailability(); updateSummary();
      }
    }
  }

  async function loadAvailability(force=false) {
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
        throw new Error(availabilityNeedsLogin ? "登录已失效" : response?.message || "未取得场次状态");
      }
      availability=response; availabilityKey=key; availabilityNeedsLogin=availabilityStore.needsLogin();
      syncAvailabilityOptions();
    } catch(error) {
      if (!scope.current() || generation!==availabilityGeneration || key!==stateKey()) return;
      availabilityError=String(error).replace(/^Error:\s*/,"");
    } finally {
      if (scope.current() && generation===availabilityGeneration) { availabilityLoading=false; paintAvailability(); updateSummary(); }
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

  function adminBlockedReason():string|null {
    const current=availabilityKey===stateKey() ? availability : null;
    const selected=current?.slots.find(slot=>`${slot.start}|${slot.end}`===slotSelect.getValue());
    return selected && isAdminLocked(selected) ? "时段已锁，请换时段或刷新"
      : selected && courtSelect.getValue() && isAdminLocked(selected,courtSelect.getValue()) ? "场地已锁，请换场地或刷新" : null;
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
  scope.onDispose(()=>{availabilityStore.dispose();cancelAvailabilityLogin();window.clearTimeout(availabilityTimer);availabilityGeneration++;});

  type SelectApi = ReturnType<typeof mountShellSelect>;
  let sportSelect!: SelectApi;

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
  });
  const slotSelect = mountShellSelect(
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
    {
      columns: true,
    },
  );

  function sportOptions(): SelectOption[] {
    return selectable.map((s) => ({ value: s.ocr_label, label: s.label }));
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
    [{ value: "", label: "自动选择可约场地" }], "", (value) => {
      preferredCourt = value;
      scheduleSave();
      paintAvailability();
    });

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
      {value:"",label:"自动选择可约场地",badge:locked ? "全部锁定" : undefined,disabled:locked},
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

  sportSelect = mountShellSelect(sportHost, sportOptions(), defaultSport.ocr_label, (v) => {
    refreshVenues(v);
    scheduleSave();
    scheduleAvailability();
  });
  venueSelect.setOptions(venueOptions(defaultSport), defaultVenue.ocr_label);
  slotSelect.setOptions(slotOptions(defaultVenue), defaultSlotKey);
  refreshCourts(defaultVenue);
  venueSlotPrefs[defaultVenue.ocr_label] = defaultSlotKey;

  allowEl.addEventListener("change", () => {
    orderWrap.hidden = !allowEl.checked;
    scheduleSave();
  });

  orderSeg.querySelectorAll("[data-order]").forEach((btn) => {
    btn.addEventListener("click", () => {
      fallbackOrder = (btn as HTMLButtonElement).dataset.order as FallbackOrder;
      orderSeg.querySelectorAll("[data-order]").forEach((b) => b.classList.remove("on"));
      btn.classList.add("on");
      scheduleSave();
    });
  });

  function syncDateOptions() {
    const today = todayISO();
    if (dateOptionsDay !== today) {
      dateOptionsDay = today;
      dateOptionsEl.innerHTML = bookingDateOptions().map(option => `
        <button type="button" class="booking-date-option" data-date="${option.value}" aria-pressed="false">
          <span>${option.label}</span><small>${formatMonthDay(option.value)}</small>
        </button>`).join("");
    }
    dateOptionsEl.querySelectorAll<HTMLButtonElement>("[data-date]").forEach(button => {
      const selected = button.dataset.date === targetDate;
      button.classList.toggle("on", selected);
      button.setAttribute("aria-pressed", String(selected));
    });
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
    if (busy || starting || !value || !isSelectableTargetDate(value)) return;
    targetDate = value;
    syncDateOptions();
    scheduleSave();
    scheduleAvailability();
  });
  firePrevBtn.addEventListener("click", () => shiftFireTime(-1));
  fireNextBtn.addEventListener("click", () => shiftFireTime(1));
  fireDelayEl.addEventListener("input", scheduleSave);
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
      fire_at: composeFireAt(target_date, fire, fireDelayEl.valueAsNumber),
      fire_delay_seconds: fireDelayEl.valueAsNumber,
      target_time_start: start,
      target_time_end: end,
      allow_time_fallback: allowEl.checked,
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
    syncDateOptions();
    const valid = validInputs();
    if (!valid) {
      panel.querySelector("#task_action_reason")!.textContent = "请填写有效的预约日期和执行时间";
      panel.querySelectorAll<HTMLButtonElement>(".task-actions button").forEach((button) => { button.disabled = true; });
      return;
    }
    const task = readTask();
    const future = !isFireAtPast(task.fire_at);
    const blocked = valid ? (adminBlockedReason() ?? (future ? hangBlockedReason(task) : bookingBlockedReason(task))) : "请填写有效的预约日期和执行时间";
    panel.querySelector("#task_summary")!.textContent = `${task.sport}　${task.venue}　${formatTargetDateHuman(task.target_date)} ${task.target_time_start}–${task.target_time_end}`;
    const dayOffset=(Date.parse(`${task.target_date}T00:00:00+08:00`)-Date.parse(`${todayISO()}T00:00:00+08:00`))/86400_000;
    panel.querySelector("#date_hint")!.textContent = busy ? `已锁定：${formatTargetDateHuman(task.target_date)}` : [0,1].includes(dayOffset) ? "该日期可查询学校实际时段，锁定状态与人数快照见选项" : targetDateHint();
    panel.querySelector("#task_execution")!.textContent = future ? `定时执行：${formatFireAtHuman(task.fire_at)}（北京时间）` : "执行方式：立即预约";
    panel.querySelector("#task_action_reason")!.textContent = busy ? "执行配置已锁定，进度可在运行页查看" : blocked ?? "";
    const primary = panel.querySelector<HTMLButtonElement>("#setup_primary")!;
    primary.textContent = future ? "定时预约" : "立即预约";
    primary.disabled = busy || starting || availabilityRecovering || !!blocked;
    const now = panel.querySelector<HTMLButtonElement>("#setup_now")!;
    now.hidden = !future || !!bookingBlockedReason(task);
    now.disabled = busy || starting || availabilityRecovering || !valid || !!bookingBlockedReason(task) || !!adminBlockedReason();
    const check = panel.querySelector<HTMLButtonElement>("#setup_check")!;
    check.hidden = task.executor !== "http";
    check.disabled = busy || starting || availabilityRecovering || !valid;
    panel.querySelector<HTMLFieldSetElement>("#task_fields")!.disabled = busy || starting;
    panel.querySelector<HTMLElement>("#task_busy_note")!.hidden = !busy;
  }

  async function start(mode: "primary" | "now" | "prepare") {
    if (starting || busy || availabilityRecovering || !validInputs()) return;
    const task = readTask();
    const scheduled = mode === "primary" && !isFireAtPast(task.fire_at);
    const blocked = mode === "prepare" ? null : adminBlockedReason() ?? (scheduled ? hangBlockedReason(task) : bookingBlockedReason(task));
    if (blocked) { toastError("暂时无法开始", blocked); return; }
    starting = true; updateSummary();
    try {
      if (mode !== "prepare" && !await askConfirm({
        title: scheduled ? "安排定时预约？" : "现在预约？",
        lead: `${task.sport}，${task.venue}，${formatTargetDateHuman(task.target_date)} ${task.target_time_start}–${task.target_time_end}。${scheduled ? `${formatFireAtHuman(task.fire_at)}（北京时间）自动执行。` : "成功后会真实预约。"}`,
        confirmLabel: scheduled ? "定时预约" : "立即预约",
      })) return;
      if (!scope.current()) return;
      saveTaskDraft(task);
      await flushTask();
      if (!scope.current()) return;
      if (scheduled) await invoke("arm_hang");
      else await invoke("start_http_run", { until: mode === "prepare" ? "auth" : "submit" });
      if (scope.current()) document.querySelector<HTMLButtonElement>('.nav-tabs [data-tab="live"]')?.click();
    } catch (error) { if (scope.current()) toastError("未能开始", String(error)); }
    finally { starting = false; if (scope.current()) updateSummary(); }
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
  end();
}
