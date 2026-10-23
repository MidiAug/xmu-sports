//! Shared vision business core — platform-agnostic.

pub mod anchor_patch;
pub mod arm;
pub mod candidates;
pub mod classify;
pub mod confidence;
pub mod date_cards;
pub mod flow;
pub mod guard;
pub mod intent;
pub mod kbd_settle;
pub mod nav_confirm;
pub mod ocr;
pub mod page_map;
pub mod page_roi;
pub mod pending;
pub mod plan;
pub mod recovery;
pub mod replay;
pub mod scene;
pub mod shadow;
pub mod slots;
pub mod state;
pub mod submit_observe;
pub mod verify;

pub use anchor_patch::{
    relocate_anchor_patch, ANCHOR_PATCH_MAX_MAD, ANCHOR_PATCH_SEARCH_RADIUS,
};
pub use arm::{
    after_click, hit_boxes_stable, may_arm, safe_click_center, transition_holds, ArmPreconditions,
};
pub use candidates::{
    analyze_captcha_input, box_iou as candidate_box_iou, bundled_shell_templates,
    count_input_glyphs, detections_from_xyxy, find_captcha_patch, load_shell_templates,
    merge_typed_proxy, parse_candidates, pixel_is_mint, template_band_best_score, BBox, Candidate,
    CandidateKind, CandidateSet, CandidateSource, CaptchaInputState, ColorRules, DetectBox,
    ElementObservation, ParseInput, PerceptionFeed, ShellVisualState, TemplateNeedle,
    ELEMENT_AUTH_LOGIN, ELEMENT_DISCOVER, ELEMENT_MINIPROGRAM, ELEMENT_MP_SEARCH,
    ELEMENT_NOTICE_ACK, ELEMENT_SPORT_ICON, ELEMENT_VENUE_BOOKING, ELEMENT_XMU_SPORTS, NAV_IOU_MIN,
    SHELL_STATE_MARGIN,
};
pub use classify::{apply_shell_page, classify_from_hits, ClassifyInput};
pub use confidence::{
    click_safety_ok, click_safety_score, layered_confidence, LayeredConfidence, CLICK_SAFETY_MIN,
};
pub use date_cards::{
    assess_frame_date, build_date_strip_coverage, col_tol, date_cards_fingerprint,
    enrich_date_cards_color, locate_date_cards, locate_date_cards_ex, normalize_date_ocr,
    parse_goal_month_day, parse_md_label, pick_date_card, sample_date_status_color, DateCard,
    DateEvidenceSources, DatePick, DateStatus, DateStripCoverage, FrameDateEvidence,
    DATE_EVIDENCE_WAIT_MS,
};
pub use page_roi::{
    merge_ocr_hits, merge_ocr_hits_replacing_bands, need_full_ocr, page_allows_roi_priority,
    page_roi_bands, roi_quality_ok, PageRoiBand, FULL_OCR_STALE_MS,
};
pub use recovery::{
    next_date_recovery, DateRecoveryAction, DateRecoveryBudget, DateRecoveryStage,
    DATE_RECOVERY_WAIT_MS,
};
pub use replay::{
    desensitize_ocr_hits, eval_replay_case, eval_replay_corpus, load_replay_case,
    load_replay_corpus, promote_from_run_dir, promote_observation, write_replay_case,
    PromoteAnnotations, ReplayCase, ReplayCaseResult, ReplayCorpusReport, ReplayExpected,
    ReplayFrame, ReplayManifest, ReplaySource, REPLAY_MATRIX_TAGS, REPLAY_SCHEMA_VERSION,
};
pub use scene::{
    fuse_date_history, make_date_record, DateFrameRecord, FusedDateEvidence, OcrObserveCache,
    SceneState, DATE_CONFIRM_STREAK, SCENE_DATE_HISTORY,
};
pub use flow::{page_resume_step, FlowStep, STEP_ORDER};
pub use guard::{guard_action, GuardError, GuardInput};
pub use intent::{
    decide_next, decide_next_with_scene, plan_captcha_keyboard, AtomicStep, CaptchaKeyStroke,
    CaptchaKeyboardPlan, FlowDomain, NextIntent, ObservationDecision, ObserveFrameMeta, TaskGoal,
    AUTH_WAIT_POLL_MS,
};
pub use kbd_settle::{
    bottom_band_fingerprint, keyboard_done_cy, BandSettleTracker, ChromeCyTracker, KbdSettleGate,
    KBD_BAND_STREAK_NEED, KBD_BAND_Y0_FRAC, KBD_CHROME_STREAK_NEED, KBD_CHROME_TOL_PX,
};
pub use nav_confirm::{
    fragile_nav_target_changed, is_fragile_nav_label, NavLabelStreak, FRAGILE_NAV_STREAK_NEED,
};
pub use ocr::{OcrHit, OcrRegionCache};
pub use page_map::{
    box_iou, build_eval_report, effective_label_source, eval_sample, scan_ocr_sensitivity,
    suggest_key_boxes, ExpectedOutcome, FrameSize, KeyBox, LabelSource, OcrSource,
    PageMapEvalReport, PageMapSample, SampleSplit, CAPTURE_CHECKLIST, HOLDOUT_REQUIRED_PAGES,
};
pub use pending::{
    assert_submit_idempotency, pending_kind_from_click, ExecutionToken, PendingError,
    PendingExecution, PendingKind, PendingStore,
};
pub use plan::{build_click_plan, DEFAULT_PLAN_TTL_MS};
pub use submit_observe::{
    classify_outcome, latch_as_str, latch_from_texts, merge_latch, outcome_as_str,
    refuse_for_outcome, should_clear_code, structural_wrong_latch, wrong_wait_ms, SubmitLatch,
    SubmitOutcome, SubmitPre, CAPTCHA_WRONG_SETTLED_MS, CAPTCHA_WRONG_TICK_MS,
    CAPTCHA_WRONG_WAIT_MS,
};
pub use shadow::{
    observe_shadow, slots_from_ocr, CandidateBox, ShadowObserveInput, ShadowObserveResult,
    ShadowTiming, DEFAULT_MIN_CONFIDENCE,
};
pub use slots::{
    apply_court_offset, nearest_time_order, parse_time_label, pick_slots_for_time, FallbackOrder,
    SlotCandidate, SlotState,
};
pub use state::{
    can_confirm_slot, can_submit_order, CaptchaPhase, Overlay, Page, SlotCta, UiSnapshot,
};
pub use verify::{after_submit_outcome, SubmitObserveOutcome};
