//! Candidate types. Decision code may only click boxes in this set.

use serde::{Deserialize, Serialize};

use crate::ocr::{labels_match_exact, normalize_ocr_text, OcrRegionCache};
use crate::slots::{parse_time_label, SlotState};

pub const LABEL_IOU_DEFAULT: f32 = 0.1;
pub const DEDUP_IOU_DEFAULT: f32 = 0.3;
pub const NAV_IOU_MIN: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Icon,
    Button,
    Text,
    ListRow,
    SlotCell,
    CaptchaHit,
    KeyboardKey,
    Other,
}

impl CandidateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Button => "button",
            Self::Text => "text",
            Self::ListRow => "list_row",
            Self::SlotCell => "slot_cell",
            Self::CaptchaHit => "captcha_hit",
            Self::KeyboardKey => "keyboard_key",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    Detect,
    Ocr,
    Template,
    Color,
}

/// Visual state of the few WeChat shell tabs for which a filled treatment has
/// navigation meaning.  This is deliberately derived from the icon/row
/// template, never from the notification dot (which is optional).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellVisualState {
    Active,
    Inactive,
}

pub const ELEMENT_DISCOVER: &str = "wechat.discover_compass";
pub const ELEMENT_MINIPROGRAM: &str = "wechat.miniprogram_entry";
pub const ELEMENT_XMU_SPORTS: &str = "xmu.sports_icon";
/// Home-grid tile: purple disc above the caption 「场馆预约」.
pub const ELEMENT_VENUE_BOOKING: &str = "xmu.venue_booking";
/// Blue button at the bottom of the 「预约须知」 card. Text sits on the fill.
pub const ELEMENT_NOTICE_ACK: &str = "xmu.notice_ack";
/// 「授权登录」 only after the agreement radio is on. Fill is the same dark blue; gray means still disabled.
pub const ELEMENT_AUTH_LOGIN: &str = "xmu.auth_login";
/// Sport grid tile. Label stays the sport name; this id is only the icon identity.
pub const ELEMENT_SPORT_ICON: &str = "xmu.sport_icon";
/// Magnifier at the top-right of the mini-program panel. Not Discover's 搜一搜.
pub const ELEMENT_MP_SEARCH: &str = "wechat.mp_search";
/// Same-family active/inactive scores closer than this stay Unknown.
pub const SHELL_STATE_MARGIN: f32 = 0.04;

/// What grounding returns upward. Policy reads `element_id` + `state`, not OCR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementObservation {
    pub element_id: String,
    pub state: ShellVisualState,
    pub theme: Option<String>,
    pub visual_bbox: BBox,
    pub hit_bbox: BBox,
    pub score: f32,
    pub runner_up_gap: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl BBox {
    pub fn cx(&self) -> i32 {
        (self.x0 + self.x1) / 2
    }
    pub fn cy(&self) -> i32 {
        (self.y0 + self.y1) / 2
    }
    pub fn w(&self) -> i32 {
        (self.x1 - self.x0).max(1)
    }
    pub fn h(&self) -> i32 {
        (self.y1 - self.y0).max(1)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotMeta {
    pub start_time: String,
    pub court_index: Option<i32>,
    pub court_label: Option<String>,
    pub state: SlotState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub bbox: BBox,
    pub kind: CandidateKind,
    pub label: String,
    pub score: f32,
    pub sources: Vec<CandidateSource>,
    pub actionable: bool,
    pub template_id: Option<String>,
    pub slot_meta: Option<SlotMeta>,
    pub evidence: Vec<String>,
    /// Stable identity. Preferred over parsing `template_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_state: Option<ShellVisualState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_bbox: Option<BBox>,
    /// Safe click rectangle. Falls back to `bbox` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_bbox: Option<BBox>,
}

impl Candidate {
    pub fn shell_alias(&self) -> Option<&'static str> {
        match self.element_id.as_deref() {
            Some(ELEMENT_DISCOVER) => return Some("发现"),
            Some(ELEMENT_MINIPROGRAM) => return Some("小程序"),
            Some(ELEMENT_XMU_SPORTS) => return Some("厦大体育"),
            Some(ELEMENT_VENUE_BOOKING) => return Some("场馆预约"),
            Some(ELEMENT_NOTICE_ACK) => return Some("我知道了"),
            Some(ELEMENT_MP_SEARCH) => return Some("搜索"),
            _ => {}
        }
        let Some(id) = self.template_id.as_deref() else {
            return None;
        };
        if id == "discover_compass" || id.starts_with("discover_compass_") {
            return Some("发现");
        }
        if id == "miniprogram_entry" || id.starts_with("miniprogram_entry_") {
            return Some("小程序");
        }
        if id == "xmu_sports_icon" || id.starts_with("xmu_sports_icon") {
            return Some("厦大体育");
        }
        if id.starts_with("mp_search") {
            return Some("搜索");
        }
        None
    }

    pub fn shell_visual_state(&self) -> Option<ShellVisualState> {
        if let Some(state) = self.element_state {
            return Some(state);
        }
        let id = self.template_id.as_deref()?;
        state_from_template_id(id)
    }

    pub fn click_bbox(&self) -> &BBox {
        self.hit_bbox.as_ref().unwrap_or(&self.bbox)
    }

    pub fn observation(&self, runner_up_gap: Option<f32>) -> Option<ElementObservation> {
        let state = self.shell_visual_state()?;
        Some(ElementObservation {
            element_id: self
                .element_id
                .clone()
                .unwrap_or_else(|| self.shell_alias().unwrap_or("unknown").to_string()),
            state,
            theme: self.theme.clone(),
            visual_bbox: self
                .visual_bbox
                .clone()
                .unwrap_or_else(|| self.bbox.clone()),
            hit_bbox: self.click_bbox().clone(),
            score: self.score,
            runner_up_gap,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateSet {
    pub items: Vec<Candidate>,
    pub dedup_iou: f32,
}

impl CandidateSet {
    pub fn has_label(&self, needle: &str) -> bool {
        self.items
            .iter()
            .any(|c| labels_match_exact(&c.label, needle))
    }

    /// Click target: explicit shell identity first (highest score), then exact label.
    pub fn find_click(&self, needle: &str) -> Option<&Candidate> {
        if let Some(c) = self
            .items
            .iter()
            .filter(|c| c.actionable && c.shell_alias() == Some(needle))
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            return Some(c);
        }
        self.items
            .iter()
            .filter(|c| {
                c.actionable
                    && c.kind != CandidateKind::SlotCell
                    && labels_match_exact(&c.label, needle)
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Sport grid tile. The whole caption must be the sport, not a venue name that contains it.
    /// Icon+caption geometry wins over a bare text line.
    pub fn find_sport_tile(&self, sport: &str) -> Option<&Candidate> {
        let is_sport = |c: &&Candidate| {
            c.actionable
                && c.kind != CandidateKind::SlotCell
                && sport_caption_is(&c.label, sport)
        };
        let better = |a: &&Candidate, b: &&Candidate| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        self.items
            .iter()
            .filter(|c| is_sport(c) && c.element_id.as_deref() == Some(ELEMENT_SPORT_ICON))
            .max_by(better)
            .or_else(|| self.items.iter().filter(is_sport).max_by(better))
    }

    pub fn find_kind(&self, kind: CandidateKind) -> Option<&Candidate> {
        self.items
            .iter()
            .filter(|c| c.kind == kind && c.actionable)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Visual selection state of a shell control.
    /// A close runner-up with the opposite state is Unknown, not a forced winner.
    pub fn shell_state(&self, needle: &str) -> Option<ShellVisualState> {
        let mut ranked: Vec<&Candidate> = self
            .items
            .iter()
            .filter(|c| {
                c.actionable && c.shell_alias() == Some(needle) && c.shell_visual_state().is_some()
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let best = ranked.first()?;
        if let Some(second) = ranked.get(1) {
            let gap = best.score - second.score;
            if gap < SHELL_STATE_MARGIN && best.shell_visual_state() != second.shell_visual_state()
            {
                return None;
            }
        }
        best.shell_visual_state()
    }

    pub fn best_available_slot(&self, target_time: Option<&str>) -> Option<&Candidate> {
        let mut cells: Vec<&Candidate> = self
            .items
            .iter()
            .filter(|c| {
                c.kind == CandidateKind::SlotCell
                    && c.actionable
                    && c.slot_meta
                        .as_ref()
                        .is_some_and(|m| m.state == SlotState::Available)
            })
            .collect();
        if let Some(t) = target_time.map(str::trim).filter(|s| !s.is_empty()) {
            let needle = parse_time_label(t).unwrap_or_else(|| t.to_string());
            cells.retain(|c| c.slot_meta.as_ref().is_some_and(|m| m.start_time == needle));
        }
        cells.sort_by_key(|c| (c.bbox.cy(), c.bbox.cx()));
        cells.into_iter().next()
    }
}

fn sport_caption_is(label: &str, sport: &str) -> bool {
    let n = normalize_ocr_text(label);
    let s = normalize_ocr_text(sport);
    if s.is_empty() {
        return false;
    }
    // Venue rows such as 「思明校区健身房」 contain the sport word but are not the tile.
    n.chars().count() <= s.chars().count() + 1 && (n == s || labels_match_exact(&n, &s))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub score: f32,
    pub class_name: String,
}

/// OmniParser-style rows: x0,y0,x1,y1,score in frame pixels. Class defaults to icon.
pub fn detections_from_xyxy(rows: &[[f32; 5]]) -> Vec<DetectBox> {
    rows.iter()
        .filter(|r| r[4] >= 0.05)
        .map(|r| DetectBox {
            x0: r[0] as i32,
            y0: r[1] as i32,
            x1: r[2] as i32,
            y1: r[3] as i32,
            score: r[4],
            class_name: "icon".into(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateNeedle {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub gray: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorRules {
    pub pale_h: [u8; 2],
    pub pale_s: [u8; 2],
    pub pale_v: [u8; 2],
    pub strong_h: [u8; 2],
    pub strong_s: [u8; 2],
    pub strong_v: [u8; 2],
    pub min_area: u32,
    pub label_iou: f32,
    pub dedup_iou: f32,
}

impl Default for ColorRules {
    fn default() -> Self {
        Self {
            // Matches Python `_mint_masks` (OpenCV H 0–179).
            pale_h: [68, 98],
            pale_s: [10, 95],
            pale_v: [200, 255],
            strong_h: [35, 100],
            strong_s: [40, 255],
            strong_v: [140, 255],
            min_area: 40,
            label_iou: LABEL_IOU_DEFAULT,
            dedup_iou: DEDUP_IOU_DEFAULT,
        }
    }
}

impl ColorRules {
    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            pale_h: [u8; 2],
            pale_s: [u8; 2],
            pale_v: [u8; 2],
            strong_h: [u8; 2],
            strong_s: [u8; 2],
            strong_v: [u8; 2],
            #[serde(default = "default_area")]
            min_area: u32,
            #[serde(default = "default_label_iou")]
            label_iou: f32,
            #[serde(default = "default_dedup_iou")]
            dedup_iou: f32,
        }
        fn default_area() -> u32 {
            40
        }
        fn default_label_iou() -> f32 {
            LABEL_IOU_DEFAULT
        }
        fn default_dedup_iou() -> f32 {
            DEDUP_IOU_DEFAULT
        }
        let f: File = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        Ok(Self {
            pale_h: f.pale_h,
            pale_s: f.pale_s,
            pale_v: f.pale_v,
            strong_h: f.strong_h,
            strong_s: f.strong_s,
            strong_v: f.strong_v,
            min_area: f.min_area,
            label_iou: f.label_iou,
            dedup_iou: f.dedup_iou,
        })
    }
}

#[derive(Clone, Copy)]
pub struct PerceptionFeed<'a> {
    pub pixels_bgra: Option<&'a [u8]>,
    pub detections: &'a [DetectBox],
    pub templates: &'a [TemplateNeedle],
}

impl<'a> Default for PerceptionFeed<'a> {
    fn default() -> Self {
        Self {
            pixels_bgra: None,
            detections: &[],
            templates: &[],
        }
    }
}

pub struct ParseInput<'a> {
    pub frame_w: i32,
    pub frame_h: i32,
    pub ocr: &'a OcrRegionCache,
    pub pixels_bgra: Option<&'a [u8]>,
    pub detections: &'a [DetectBox],
    pub templates: &'a [TemplateNeedle],
    pub rules: &'a ColorRules,
}

pub(super) fn state_from_template_id(id: &str) -> Option<ShellVisualState> {
    if id.contains("_inactive") {
        Some(ShellVisualState::Inactive)
    } else if id.contains("_active") {
        Some(ShellVisualState::Active)
    } else {
        None
    }
}

pub fn box_iou(a: &BBox, b: &BBox) -> f32 {
    let x0 = a.x0.max(b.x0);
    let y0 = a.y0.max(b.y0);
    let x1 = a.x1.min(b.x1);
    let y1 = a.y1.min(b.y1);
    let iw = (x1 - x0).max(0) as f32;
    let ih = (y1 - y0).max(0) as f32;
    let inter = iw * ih;
    let area_a = a.w().max(0) as f32 * a.h().max(0) as f32;
    let area_b = b.w().max(0) as f32 * b.h().max(0) as f32;
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}
