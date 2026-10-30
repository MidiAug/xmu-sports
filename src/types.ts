export type Tab = "setup" | "plans" | "live" | "history" | "settings";
export type FallbackOrder = "up_then_down" | "down_then_up";
export type ShellThemePref = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";
export type CloseMode = "ask" | "tray" | "quit";

export type TimeSlot = { start: string; end: string };
export type Venue = {
  id: string;
  label: string;
  ocr_label: string;
  location: string;
  has_courts: boolean;
  court_count_hint?: number;
  capacity_hint?: number;
  courts?: { id: string; name: string }[];
  observed_at?: string;
  observed_time_slots?: TimeSlot[];
  time_slots: TimeSlot[];
};
export type Sport = {
  id: string;
  label: string;
  ocr_label: string;
  /** false = placeholder; cannot favorite or book */
  enabled?: boolean;
  venues: Venue[];
};

export type BookingTask = {
  task_id: string;
  plan_id?: string;
  revision?: number;
  sport: string;
  venue: string;
  target_date: string;
  fire_at: string;
  /** fire_at already includes this delay; absent on older tasks. */
  fire_delay_seconds?: number;
  target_time_start: string;
  target_time_end: string;
  allow_time_fallback: boolean;
  time_fallback_order: FallbackOrder;
  executor: "http" | "vision" | "script" | "hybrid";
  /** Multi-court only, e.g. "01". Empty = leftmost available. */
  prefer_court?: string | null;
  /** Skip N leftmost available courts (0=leftmost). Default 2. */
  court_offset?: number;
};

export type HttpExchangeEvidence = {
  api?: string; method?: string; error?: string; error_type?: string; http_status?: number; content_type?: string;
  request?: Record<string, unknown>; response?: unknown; timeout_seconds?: {connect: number; read: number};
  phase?: string; elapsed_ms?: number;
};
export type RunEvent = {
  http?: HttpExchangeEvidence & { phase?: string; message?: string; reason?: string; verified?: boolean; court?: string; date?: string; time?: string; order_id?: number | string;
    failure_exchange?: HttpExchangeEvidence;
    captcha_requested?: boolean; execution_proof?: {add_order_calls?: number};
    diagnostics?: {http?: Record<string, {count?: number; errors?: Record<string, number>}>};
    submitted_target?: { sport?: string; venue?: string; date?: string; time?: string; end?: string; court?: string | null };
    elapsed_ms?: number; remaining_seconds?: number; candidate_index?: number;
    check_attempt?: number; check_total?: number; retry_after_seconds?: number;
    target?: { sport?: string; venue?: string; date?: string; time?: string; end?: string; court?: string | null } } | null;
  ts: string;
  task_id: string;
  run_id?: string | null;
  schema_version?: number | null;
  page_id?: string | null;
  action?: string | null;
  result?: string | null;
  evidence?: string[];
  reason_code?: string | null;
};

export type LiveStatus = {
  task: BookingTask | null;
  last_event: RunEvent | null;
  recent_events?: RunEvent[];
  running: boolean;
  armed: boolean;
  stopping?: boolean;
  operation?: "auth" | "prepare" | "booking" | "scheduled" | "recheck" | null;
  fire_at: string | null;
};

export type ShellPrefs = {
  close_to_tray: boolean;
  close_pref_set: boolean;
  close_remember_default_used: boolean;
  /** Sport catalog ids the user keeps in the booking picker; omit = all enabled */
  favorite_sport_ids?: string[];
  /** venue ocr_label → "HH:MM|HH:MM" last picked slot */
  venue_slot_prefs?: Record<string, string>;
  /** When true, observations keep full OCR hits (may include WeChat UI text). */
  detailed_diagnostics?: boolean;
  autostart_hidden?: boolean;
};

export type WechatGlance = {
  bound_title: string;
  now: string;
  next: string;
  basis: string;
  page: string;
  keyboard_open: boolean;
  recognized_captcha?: string | null;
  refuse_reason: string | null;
  candidate_count: number;
  input_invoked: boolean;
  can_click: boolean;
  intent_kind?: string;
  wait_ms?: number | null;
  /** Navigate click label (e.g. 确认预约). */
  click_label?: string;
  page_confidence?: number | null;
  date_confidence?: number | null;
  click_safety_confidence?: number | null;
  image_b64: string;
};

export type WechatProbe = {
  running: boolean;
  title: string | null;
  miniprogram: boolean;
  message: string;
};

export type SelectOption = { value: string; label: string; disabled?: boolean; badge?: string };
