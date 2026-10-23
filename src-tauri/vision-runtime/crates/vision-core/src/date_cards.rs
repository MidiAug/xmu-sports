//! Venue-detail date cards: weekday + MM月DD日 + bottom「暂无|预定」.
//!
//! Single-frame layer produces [`FrameDateEvidence`] only.
//! Cross-frame [`crate::scene::FusedDateEvidence`] alone may become DATE_MISMATCH.
//! Never treat 「今天」/ weekday alone as the target.

use crate::candidates::{BBox, Candidate, CandidateKind, CandidateSet};

/// Minimum absolute column tolerance; also at least `frame_w * 0.04`.
const COL_TOL_MIN: i32 = 55;

const WEEKDAYS: &[&str] = &[
    "今天", "明天", "周一", "周二", "周三", "周四", "周五", "周六", "周日",
];

/// Soft wait when date evidence is incomplete / insufficient.
pub const DATE_EVIDENCE_WAIT_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStatus {
    Bookable,
    Empty,
    Unknown,
}

/// Per-card multi-source flags (structure / MD / text / color).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DateEvidenceSources {
    pub structure_ok: bool,
    pub md_ok: bool,
    pub status_text: Option<DateStatus>,
    pub status_color: Option<DateStatus>,
}

impl DateEvidenceSources {
    /// Text + color agree on Bookable or Empty.
    pub fn dual_status_confirmed(&self) -> Option<DateStatus> {
        match (self.status_text, self.status_color) {
            (Some(t), Some(c))
                if t == c && matches!(t, DateStatus::Bookable | DateStatus::Empty) =>
            {
                Some(t)
            }
            _ => None,
        }
    }

    pub fn status_conflict(&self) -> bool {
        matches!(
            (self.status_text, self.status_color),
            (Some(DateStatus::Bookable), Some(DateStatus::Empty))
                | (Some(DateStatus::Empty), Some(DateStatus::Bookable))
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DateCard {
    pub weekday: String,
    /// Raw OCR label, e.g. 「09月21日」.
    pub date_label: String,
    pub month: u32,
    pub day: u32,
    pub status: DateStatus,
    /// Click target: status pill bbox when present.
    pub click_bbox: BBox,
    pub status_label: String,
    /// Color sample of the status pill (None when no pixels).
    pub status_color: Option<DateStatus>,
    pub sources: DateEvidenceSources,
}

/// Legacy pick result (compat). Prefer [`FrameDateEvidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePick {
    Bookable,
    Unavailable,
    Missing,
}

/// Per-frame date strip geometry / OCR quality. ROI missing ⇒ incomplete, not absent.
#[derive(Debug, Clone, PartialEq)]
pub struct DateStripCoverage {
    pub roi: Option<BBox>,
    pub visible_cols: usize,
    pub parsed_dates: Vec<(u32, u32)>,
    pub ordered: bool,
    pub continuous: bool,
    pub first: Option<(u32, u32)>,
    pub last: Option<(u32, u32)>,
    pub animating: bool,
    pub occluded: bool,
    /// 0..1 heuristic from cols + parse success.
    pub quality: f32,
    /// Goal month/day falls inside the visible [first, last] span.
    pub goal_in_visible_range: bool,
}

impl DateStripCoverage {
    pub fn empty() -> Self {
        Self {
            roi: None,
            visible_cols: 0,
            parsed_dates: Vec::new(),
            ordered: false,
            continuous: false,
            first: None,
            last: None,
            animating: false,
            occluded: false,
            quality: 0.0,
            goal_in_visible_range: false,
        }
    }

    pub fn is_sufficient(&self) -> bool {
        // Coverage proof for ConfirmedAbsent: visual LTR order + day continuity.
        // Sorting dates alone must not make `ordered` vacuously true.
        self.roi.is_some()
            && self.visible_cols >= 1
            && self.quality >= 0.4
            && self.ordered
            && self.continuous
            && !self.animating
            && !self.occluded
    }
}

/// Single-frame date evidence. Never a terminal DATE_MISMATCH.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameDateEvidence {
    Bookable(DateCard),
    UnavailableCandidate(DateCard),
    AbsentWithCoverage(DateStripCoverage),
    Incomplete { why: String },
}

/// Column tolerance: max(55, frame_width * 0.04).
pub fn col_tol(frame_w: i32) -> i32 {
    let rel = ((frame_w.max(1) as f32) * 0.04).round() as i32;
    rel.max(COL_TOL_MIN)
}

/// Normalize OCR date fragments: strip spaces, fullwidth digits, 曰→日.
pub fn normalize_date_ocr(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '曰' => '日',
            '０'..='９' => char::from_u32(u32::from(c) - u32::from('０') + u32::from('0'))
                .unwrap_or(c),
            other => other,
        })
        .collect()
}

/// Parse `YYYY-MM-DD` / `YYYY/MM/DD` into (month, day). Ignores year.
pub fn parse_goal_month_day(date: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = date.trim().split(['-', '/']).collect();
    if parts.len() != 3 {
        return None;
    }
    let m = parts[1].parse::<u32>().ok()?;
    let d = parts[2].parse::<u32>().ok()?;
    if (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((m, d))
    } else {
        None
    }
}

/// Whole-label `M月D日` / `MM月DD日` after normalization.
pub fn parse_md_label(text: &str) -> Option<(u32, u32)> {
    let t = normalize_date_ocr(text);
    let chars: Vec<char> = t.chars().collect();
    let month_i = chars.iter().position(|&c| c == '月')?;
    let day_i = chars.iter().position(|&c| c == '日')?;
    if month_i == 0 || day_i <= month_i + 1 || day_i != chars.len() - 1 {
        return None;
    }
    let m: String = chars[..month_i].iter().collect();
    let d: String = chars[month_i + 1..day_i].iter().collect();
    if m.is_empty() || d.is_empty() || !m.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !d.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month = m.parse::<u32>().ok()?;
    let day = d.parse::<u32>().ok()?;
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some((month, day))
    } else {
        None
    }
}

fn parse_status(text: &str) -> (DateStatus, &str) {
    let t = normalize_date_ocr(text);
    if t == "暂无" {
        return (DateStatus::Empty, "暂无");
    }
    if t == "预定" || t == "預訂" || t == "预约" {
        return (DateStatus::Bookable, "预定");
    }
    (DateStatus::Unknown, "")
}

fn is_weekday(label: &str) -> bool {
    let t = label.trim();
    WEEKDAYS.iter().any(|w| *w == t)
}

fn infer_frame_w(candidates: &CandidateSet) -> i32 {
    candidates
        .items
        .iter()
        .map(|c| c.bbox.x1)
        .max()
        .unwrap_or(900)
        .max(200)
}

fn parse_goal_calendar_date(date: &str) -> Option<chrono::NaiveDate> {
    let parts: Vec<&str> = date.trim().split(['-', '/']).collect();
    if parts.len() != 3 {
        return None;
    }
    chrono::NaiveDate::from_ymd_opt(
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    )
}

/// Build coverage from located cards + optional ROI envelope.
pub fn build_date_strip_coverage(
    cards: &[DateCard],
    goal_date: &str,
    frame_w: i32,
    frame_h: i32,
) -> DateStripCoverage {
    let _ = (frame_w, frame_h);
    if cards.is_empty() {
        return DateStripCoverage::empty();
    }
    // Use real calendar days. The old month*31+day key treated 09/30 → 10/01
    // as a two-day gap and rejected an otherwise complete date strip.
    let mut by_x: Vec<&DateCard> = cards.iter().collect();
    by_x.sort_by_key(|c| (c.click_bbox.x0, c.click_bbox.y0));
    let dates_ltr: Vec<(u32, u32)> = by_x.iter().map(|c| (c.month, c.day)).collect();
    let first = dates_ltr.first().copied();
    let last = dates_ltr.last().copied();
    let goal = parse_goal_calendar_date(goal_date);
    let goal_year = goal.map(|d| chrono::Datelike::year(&d)).unwrap_or(2000);
    let goal_month = goal.map(|d| chrono::Datelike::month(&d)).unwrap_or(0);
    let mut year = match (first.map(|d| d.0), goal_month) {
        (Some(12), 1) => goal_year - 1,
        (Some(1), 12) => goal_year + 1,
        _ => goal_year,
    };
    let mut previous_month = None;
    let calendar_dates: Vec<Option<chrono::NaiveDate>> = dates_ltr
        .iter()
        .map(|&(month, day)| {
            if previous_month == Some(12) && month == 1 {
                year += 1;
            }
            previous_month = Some(month);
            chrono::NaiveDate::from_ymd_opt(year, month, day)
        })
        .collect();
    let valid_dates = calendar_dates.iter().all(Option::is_some);
    let ordered = valid_dates && calendar_dates.windows(2).all(|w| w[0] <= w[1]);
    let continuous = valid_dates && calendar_dates.windows(2).all(|w| {
        w[1].unwrap().signed_duration_since(w[0].unwrap()).num_days() == 1
    });
    let mut roi = cards[0].click_bbox.clone();
    for c in cards.iter().skip(1) {
        roi.x0 = roi.x0.min(c.click_bbox.x0);
        roi.y0 = roi.y0.min(c.click_bbox.y0);
        roi.x1 = roi.x1.max(c.click_bbox.x1);
        roi.y1 = roi.y1.max(c.click_bbox.y1);
    }
    // Expand ROI upward to cover weekday/date rows roughly.
    roi.y0 = (roi.y0 - 80).max(0);
    let goal_in = match (goal, calendar_dates.first(), calendar_dates.last()) {
        (Some(g), Some(Some(f)), Some(Some(l))) => g >= *f && g <= *l,
        _ => false,
    };
    let quality = {
        let col_score = (cards.len() as f32 / 4.0).min(1.0);
        let cont = if continuous { 0.25 } else { 0.0 };
        let ord = if ordered { 0.15 } else { 0.0 };
        (0.6 * col_score + cont + ord).min(1.0)
    };
    DateStripCoverage {
        roi: Some(roi),
        visible_cols: cards.len(),
        parsed_dates: dates_ltr,
        ordered,
        continuous,
        first,
        last,
        animating: false,
        occluded: false,
        quality,
        goal_in_visible_range: goal_in,
    }
}

/// Sample status-pill color inside `bbox`. Gray ≈ 暂无; warm/mint ≈ 预定.
pub fn sample_date_status_color(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    bbox: &BBox,
) -> Option<DateStatus> {
    if frame_w == 0 || frame_h == 0 {
        return None;
    }
    let x0 = bbox.x0.max(0) as u32;
    let y0 = bbox.y0.max(0) as u32;
    let x1 = (bbox.x1 as u32).min(frame_w);
    let y1 = (bbox.y1 as u32).min(frame_h);
    if x1 <= x0.saturating_add(2) || y1 <= y0.saturating_add(2) {
        return None;
    }
    let mut warm = 0u32;
    let mut gray = 0u32;
    let mut total = 0u32;
    let step_x = ((x1 - x0) / 6).max(1);
    let step_y = ((y1 - y0) / 6).max(1);
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let i = ((y * frame_w + x) * 4) as usize;
            if i + 2 < bgra.len() {
                let b = bgra[i];
                let g = bgra[i + 1];
                let r = bgra[i + 2];
                let max = r.max(g).max(b) as i32;
                let min = r.min(g).min(b) as i32;
                let chroma = max - min;
                let lum = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) as i32;
                total += 1;
                if chroma < 28 && (60..220).contains(&lum) {
                    gray += 1;
                } else if (r as i32 > g as i32 + 15 && r as i32 > b as i32 + 10 && chroma > 35)
                    || (g as i32 > r as i32 + 10 && g as i32 > b as i32 + 10 && chroma > 35)
                {
                    warm += 1;
                }
            }
            x = x.saturating_add(step_x);
        }
        y = y.saturating_add(step_y);
    }
    if total < 4 {
        return None;
    }
    let gray_r = gray as f32 / total as f32;
    let warm_r = warm as f32 / total as f32;
    if gray_r >= 0.35 && gray_r > warm_r {
        Some(DateStatus::Empty)
    } else if warm_r >= 0.25 {
        Some(DateStatus::Bookable)
    } else {
        None
    }
}

/// Attach color samples and rebuild `sources` for each card.
pub fn enrich_date_cards_color(
    cards: &mut [DateCard],
    bgra: Option<&[u8]>,
    frame_w: u32,
    frame_h: u32,
) {
    for c in cards.iter_mut() {
        let color = bgra.and_then(|px| {
            sample_date_status_color(px, frame_w, frame_h, &c.click_bbox)
        });
        c.status_color = color;
        c.sources.status_color = color;
        c.sources.status_text = match c.status {
            DateStatus::Unknown => None,
            s => Some(s),
        };
        c.sources.structure_ok = !c.weekday.is_empty();
        c.sources.md_ok = c.month > 0 && c.day > 0;
    }
}

/// Assess one frame. Never returns a fused/terminal absence.
pub fn assess_frame_date(
    cards: &[DateCard],
    goal_date: &str,
    coverage: &DateStripCoverage,
) -> FrameDateEvidence {
    let Some((want_m, want_d)) = parse_goal_month_day(goal_date) else {
        return FrameDateEvidence::Incomplete {
            why: "goal_date_unparsed".into(),
        };
    };
    if cards.is_empty() {
        return FrameDateEvidence::Incomplete {
            why: "no_date_cards".into(),
        };
    }
    if !coverage.is_sufficient() {
        return FrameDateEvidence::Incomplete {
            why: "coverage_insufficient".into(),
        };
    }
    let matched: Vec<&DateCard> = cards
        .iter()
        .filter(|c| c.month == want_m && c.day == want_d)
        .collect();
    if matched.is_empty() {
        if coverage.goal_in_visible_range {
            return FrameDateEvidence::AbsentWithCoverage(coverage.clone());
        }
        return FrameDateEvidence::Incomplete {
            why: "goal_offscreen_or_range_unknown".into(),
        };
    }
    if matched.iter().any(|c| c.sources.status_conflict()) {
        return FrameDateEvidence::Incomplete {
            why: "status_text_color_conflict".into(),
        };
    }
    if let Some(c) = matched.iter().find(|c| c.status == DateStatus::Bookable) {
        // Color opposing bookable already handled as conflict; color None is ok.
        return FrameDateEvidence::Bookable((*c).clone());
    }
    if let Some(c) = matched.iter().find(|c| c.status == DateStatus::Empty) {
        return FrameDateEvidence::UnavailableCandidate((*c).clone());
    }
    FrameDateEvidence::Incomplete {
        why: "status_unknown".into(),
    }
}

/// Content fingerprint for scene dedup (dates + statuses).
pub fn date_cards_fingerprint(cards: &[DateCard]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for c in cards {
        c.month.hash(&mut h);
        c.day.hash(&mut h);
        (c.status as u8).hash(&mut h);
        c.status_label.hash(&mut h);
    }
    h.finish()
}

/// Join nearby OCR fragments into synthetic `M月D日` candidates when whole-label OCR splits.
fn synthesize_md_candidates(candidates: &CandidateSet, frame_w: i32) -> Vec<Candidate> {
    let tol = col_tol(frame_w);
    let texts: Vec<&Candidate> = candidates
        .items
        .iter()
        .filter(|c| !c.label.trim().is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, a) in texts.iter().enumerate() {
        let na = normalize_date_ocr(&a.label);
        // Pattern: 「09月」 + 「28」 + 「日」 or 「09月28」 + 「日」
        if na.ends_with('月') && na.chars().filter(|c| c.is_ascii_digit()).count() >= 1 {
            for (j, b) in texts.iter().enumerate() {
                if i == j {
                    continue;
                }
                let nb = normalize_date_ocr(&b.label);
                if !nb.chars().all(|c| c.is_ascii_digit()) || nb.is_empty() {
                    continue;
                }
                let dx = (b.bbox.cx() - a.bbox.cx()).abs();
                let dy = (b.bbox.cy() - a.bbox.cy()).abs();
                if dx > tol || dy > 24 {
                    continue;
                }
                // Look for 「日」 nearby
                for (k, c) in texts.iter().enumerate() {
                    if k == i || k == j {
                        continue;
                    }
                    let nc = normalize_date_ocr(&c.label);
                    if nc != "日" {
                        continue;
                    }
                    let dx2 = (c.bbox.cx() - b.bbox.cx()).abs();
                    let dy2 = (c.bbox.cy() - b.bbox.cy()).abs();
                    if dx2 > tol || dy2 > 24 {
                        continue;
                    }
                    let label = format!("{na}{nb}日");
                    if parse_md_label(&label).is_none() {
                        continue;
                    }
                    let mut bbox = a.bbox.clone();
                    bbox.x0 = bbox.x0.min(b.bbox.x0).min(c.bbox.x0);
                    bbox.y0 = bbox.y0.min(b.bbox.y0).min(c.bbox.y0);
                    bbox.x1 = bbox.x1.max(b.bbox.x1).max(c.bbox.x1);
                    bbox.y1 = bbox.y1.max(b.bbox.y1).max(c.bbox.y1);
                    out.push(Candidate {
                        id: format!("synth_md_{i}_{j}_{k}"),
                        bbox,
                        kind: CandidateKind::Text,
                        label,
                        score: 0.75,
                        sources: a.sources.clone(),
                        actionable: false,
                        template_id: None,
                        slot_meta: None,
                        evidence: vec!["joined_md_fragments".into()],
                        element_id: None,
                        element_state: None,
                        theme: None,
                        visual_bbox: None,
                        hit_bbox: None,
                    });
                }
            }
        }
        // Pattern: 「09月28」 + 「日」
        if na.contains('月')
            && !na.contains('日')
            && na.chars().any(|c| c.is_ascii_digit())
            && na.chars().filter(|c| *c == '月').count() == 1
        {
            for (j, b) in texts.iter().enumerate() {
                if i == j {
                    continue;
                }
                if normalize_date_ocr(&b.label) != "日" {
                    continue;
                }
                let dx = (b.bbox.cx() - a.bbox.cx()).abs();
                let dy = (b.bbox.cy() - a.bbox.cy()).abs();
                if dx > tol || dy > 24 {
                    continue;
                }
                let label = format!("{na}日");
                if parse_md_label(&label).is_none() {
                    continue;
                }
                let mut bbox = a.bbox.clone();
                bbox.x0 = bbox.x0.min(b.bbox.x0);
                bbox.y0 = bbox.y0.min(b.bbox.y0);
                bbox.x1 = bbox.x1.max(b.bbox.x1);
                bbox.y1 = bbox.y1.max(b.bbox.y1);
                out.push(Candidate {
                    id: format!("synth_md2_{i}_{j}"),
                    bbox,
                    kind: CandidateKind::Text,
                    label,
                    score: 0.75,
                    sources: a.sources.clone(),
                    actionable: false,
                    template_id: None,
                    slot_meta: None,
                    evidence: vec!["joined_md_fragments".into()],
                    element_id: None,
                    element_state: None,
                    theme: None,
                    visual_bbox: None,
                    hit_bbox: None,
                });
            }
        }
    }
    out
}

/// Group OCR candidates into date cards (weekday + date + 暂无|预定).
pub fn locate_date_cards(candidates: &CandidateSet) -> Vec<DateCard> {
    locate_date_cards_ex(candidates, infer_frame_w(candidates))
}

pub fn locate_date_cards_ex(candidates: &CandidateSet, frame_w: i32) -> Vec<DateCard> {
    let tol = col_tol(frame_w);
    let synth = synthesize_md_candidates(candidates, frame_w);

    let weeks: Vec<&Candidate> = candidates
        .items
        .iter()
        .filter(|c| is_weekday(&c.label))
        .collect();

    let mut date_refs: Vec<&Candidate> = candidates
        .items
        .iter()
        .filter(|c| parse_md_label(&c.label).is_some())
        .collect();
    for s in &synth {
        date_refs.push(s);
    }

    let statuses: Vec<&Candidate> = candidates
        .items
        .iter()
        .filter(|c| {
            let (st, _) = parse_status(&c.label);
            matches!(st, DateStatus::Bookable | DateStatus::Empty)
        })
        .collect();

    let mut cards: Vec<DateCard> = Vec::new();
    let mut used_dates: Vec<*const Candidate> = Vec::new();

    for week in weeks {
        let wx = week.bbox.cx();
        let wy = week.bbox.cy();
        let mut best: Option<&Candidate> = None;
        let mut best_key = (i32::MAX, i32::MAX);
        for d in &date_refs {
            if used_dates.contains(&(*d as *const Candidate)) {
                continue;
            }
            let dx = d.bbox.cx();
            let dy = d.bbox.cy();
            if dy < wy - 5 || (dx - wx).abs() >= tol {
                continue;
            }
            let key = ((dx - wx).abs(), dy - wy);
            if key < best_key {
                best_key = key;
                best = Some(*d);
            }
        }
        let Some(dhit) = best else {
            continue;
        };
        used_dates.push(dhit as *const Candidate);
        let (month, day) = match parse_md_label(&dhit.label) {
            Some(md) => md,
            None => continue,
        };
        let dy = dhit.bbox.cy();

        let mut status_hit: Option<&Candidate> = None;
        let mut status_key = (i32::MAX, i32::MAX);
        for s in &statuses {
            let sx = s.bbox.cx();
            let sy = s.bbox.cy();
            if sy < dy - 5 || (sx - wx).abs() >= tol {
                continue;
            }
            let key = ((sx - wx).abs(), (sy - (dy + 40)).abs());
            if key < status_key {
                status_key = key;
                status_hit = Some(*s);
            }
        }

        let (status, status_label, click_bbox) = if let Some(shit) = status_hit {
            let (st, lab) = parse_status(&shit.label);
            (st, lab.to_string(), shit.click_bbox().clone())
        } else {
            (
                DateStatus::Unknown,
                String::new(),
                BBox {
                    x0: (week.bbox.cx() + dhit.bbox.cx()) / 2 - 12,
                    y0: (week.bbox.cy() + dhit.bbox.cy()) / 2 - 8,
                    x1: (week.bbox.cx() + dhit.bbox.cx()) / 2 + 12,
                    y1: (week.bbox.cy() + dhit.bbox.cy()) / 2 + 8,
                },
            )
        };

        let status_text = match status {
            DateStatus::Unknown => None,
            s => Some(s),
        };
        cards.push(DateCard {
            weekday: week.label.clone(),
            date_label: dhit.label.clone(),
            month,
            day,
            status,
            click_bbox,
            status_label,
            status_color: None,
            sources: DateEvidenceSources {
                structure_ok: true,
                md_ok: true,
                status_text,
                status_color: None,
            },
        });
    }

    cards.sort_by_key(|c| (c.click_bbox.cx(), c.click_bbox.cy()));
    cards
}

/// Legacy pick. Maps Incomplete / Absent → Missing (does not imply DATE_MISMATCH).
pub fn pick_date_card<'a>(
    cards: &'a [DateCard],
    goal_date: &str,
) -> (DatePick, Option<&'a DateCard>) {
    let cov = build_date_strip_coverage(cards, goal_date, 900, 900);
    match assess_frame_date(cards, goal_date, &cov) {
        FrameDateEvidence::Bookable(_) => {
            let c = cards.iter().find(|c| {
                parse_goal_month_day(goal_date)
                    .is_some_and(|(m, d)| c.month == m && c.day == d && c.status == DateStatus::Bookable)
            });
            (DatePick::Bookable, c)
        }
        FrameDateEvidence::UnavailableCandidate(_) => {
            let c = cards.iter().find(|c| {
                parse_goal_month_day(goal_date)
                    .is_some_and(|(m, d)| c.month == m && c.day == d && c.status == DateStatus::Empty)
            });
            (DatePick::Unavailable, c)
        }
        _ => (DatePick::Missing, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::{parse_candidates, ColorRules, ParseInput};
    use crate::ocr::{OcrHit, OcrRegionCache};

    fn set(texts: &[(&str, i32, i32)]) -> CandidateSet {
        let ocr = OcrRegionCache::from_hits(
            texts
                .iter()
                .map(|(t, x, y)| OcrHit {
                    text: (*t).into(),
                    x: *x,
                    y: *y,
                    w: 48,
                    h: 18,
                    score: 0.9,
                })
                .collect(),
        );
        parse_candidates(&ParseInput {
            frame_w: 900,
            frame_h: 900,
            ocr: &ocr,
            pixels_bgra: None,
            detections: &[],
            templates: &[],
            rules: &ColorRules::default(),
        })
    }

    #[test]
    fn parses_padded_and_unpadded_md() {
        assert_eq!(parse_md_label("09月21日"), Some((9, 21)));
        assert_eq!(parse_md_label("9月21日"), Some((9, 21)));
        assert_eq!(parse_md_label("今天"), None);
        assert_eq!(parse_md_label("周二"), None);
    }

    #[test]
    fn normalizes_lookalikes_and_fullwidth() {
        assert_eq!(parse_md_label("０９月２８曰"), Some((9, 28)));
        assert_eq!(normalize_date_ocr(" 09 月 "), "09月");
    }

    #[test]
    fn goal_iso_to_month_day() {
        assert_eq!(parse_goal_month_day("2026-09-21"), Some((9, 21)));
        assert_eq!(parse_goal_month_day("2026/09/21"), Some((9, 21)));
    }

    #[test]
    fn month_end_date_strip_is_continuous_and_target_is_present() {
        let cs = set(&[
            ("周三", 100, 200), ("09月30日", 100, 225), ("暂无", 100, 250),
            ("周四", 300, 200), ("10月01日", 300, 225), ("暂无", 300, 250),
        ]);
        let cards = locate_date_cards_ex(&cs, 900);
        let cov = build_date_strip_coverage(&cards, "2026-10-01", 900, 900);
        assert_eq!(cards.len(), 2);
        assert!(cov.continuous);
        assert!(cov.is_sufficient());
        assert!(cov.goal_in_visible_range);
        assert!(matches!(
            assess_frame_date(&cards, "2026-10-01", &cov),
            FrameDateEvidence::UnavailableCandidate(c) if c.month == 10 && c.day == 1
        ));
    }

    #[test]
    fn year_end_and_leap_day_date_strips_are_continuous() {
        let year_end = set(&[
            ("周四", 100, 200), ("12月31日", 100, 225), ("暂无", 100, 250),
            ("周五", 300, 200), ("01月01日", 300, 225), ("暂无", 300, 250),
        ]);
        let cards = locate_date_cards_ex(&year_end, 900);
        let cov = build_date_strip_coverage(&cards, "2027-01-01", 900, 900);
        assert!(cov.continuous && cov.goal_in_visible_range);

        let leap = set(&[
            ("周三", 100, 200), ("02月28日", 100, 225), ("暂无", 100, 250),
            ("周四", 300, 200), ("02月29日", 300, 225), ("暂无", 300, 250),
        ]);
        let cards = locate_date_cards_ex(&leap, 900);
        assert!(build_date_strip_coverage(&cards, "2024-02-29", 900, 900).continuous);
        assert!(!build_date_strip_coverage(&cards, "2025-02-28", 900, 900).continuous);
    }

    #[test]
    fn col_tol_scales_with_frame() {
        assert_eq!(col_tol(900), 55);
        assert_eq!(col_tol(2000), 80);
    }

    #[test]
    fn bookable_card_click_is_status_not_date() {
        let cs = set(&[
            ("今天", 200, 180),
            ("09月21日", 200, 210),
            ("预定", 200, 250),
            ("周二", 360, 180),
            ("09月22日", 360, 210),
            ("暂无", 360, 250),
        ]);
        let cards = locate_date_cards(&cs);
        assert_eq!(cards.len(), 2);
        let cov = build_date_strip_coverage(&cards, "2026-09-21", 900, 900);
        let ev = assess_frame_date(&cards, "2026-09-21", &cov);
        match ev {
            FrameDateEvidence::Bookable(card) => {
                assert_eq!(card.status, DateStatus::Bookable);
                assert!(card.click_bbox.cy() > 230);
            }
            other => panic!("expected Bookable, got {other:?}"),
        }
    }

    #[test]
    fn empty_target_is_unavailable_candidate() {
        let cs = set(&[
            ("今天", 200, 180),
            ("09月21日", 200, 210),
            ("暂无", 200, 250),
        ]);
        let cards = locate_date_cards(&cs);
        let cov = build_date_strip_coverage(&cards, "2026-09-21", 900, 900);
        assert!(matches!(
            assess_frame_date(&cards, "2026-09-21", &cov),
            FrameDateEvidence::UnavailableCandidate(_)
        ));
    }

    #[test]
    fn wrong_day_in_range_is_absent_with_coverage_not_terminal() {
        let cs = set(&[
            ("周二", 200, 180),
            ("09月21日", 200, 210),
            ("预定", 200, 250),
            ("周三", 360, 180),
            ("09月23日", 360, 210),
            ("预定", 360, 250),
        ]);
        let cards = locate_date_cards(&cs);
        // Goal before visible span → incomplete (offscreen), not mismatch.
        let cov_off = build_date_strip_coverage(&cards, "2026-09-20", 900, 900);
        assert!(!cov_off.goal_in_visible_range);
        assert!(matches!(
            assess_frame_date(&cards, "2026-09-20", &cov_off),
            FrameDateEvidence::Incomplete { .. }
        ));
        // Span 21–23 with a gap (no 22 card) is not coverage proof — Incomplete.
        let cov_mid = build_date_strip_coverage(&cards, "2026-09-22", 900, 900);
        assert!(cov_mid.goal_in_visible_range);
        assert!(!cov_mid.continuous);
        assert!(!cov_mid.is_sufficient());
        assert!(matches!(
            assess_frame_date(&cards, "2026-09-22", &cov_mid),
            FrameDateEvidence::Incomplete { .. }
        ));
    }

    #[test]
    fn today_alone_is_incomplete() {
        let cs = set(&[("今天", 200, 180), ("预定", 200, 250)]);
        let cards = locate_date_cards(&cs);
        assert!(cards.is_empty());
        let cov = build_date_strip_coverage(&cards, "2026-09-21", 900, 900);
        assert!(matches!(
            assess_frame_date(&cards, "2026-09-21", &cov),
            FrameDateEvidence::Incomplete { .. }
        ));
    }

    #[test]
    fn joined_fragments_form_md_label() {
        let cs = set(&[
            ("今天", 200, 180),
            ("09月", 180, 210),
            ("28", 220, 210),
            ("日", 250, 210),
            ("预定", 200, 250),
        ]);
        let cards = locate_date_cards(&cs);
        assert!(
            cards.iter().any(|c| c.month == 9 && c.day == 28),
            "expected joined 09月+28+日, got {cards:?}"
        );
    }

    #[test]
    fn two_cards_do_not_cross_click() {
        let cs = set(&[
            ("今天", 200, 180),
            ("09月21日", 200, 210),
            ("暂无", 200, 250),
            ("周二", 360, 180),
            ("09月22日", 360, 210),
            ("预定", 360, 250),
        ]);
        let cards = locate_date_cards(&cs);
        let (pick, card) = pick_date_card(&cards, "2026-09-21");
        assert_eq!(pick, DatePick::Unavailable);
        assert!(card.is_some());
        let (pick2, card2) = pick_date_card(&cards, "2026-09-22");
        assert_eq!(pick2, DatePick::Bookable);
        let c2 = card2.unwrap();
        assert!(c2.click_bbox.cx() > 300);
    }

    #[test]
    fn gray_pill_samples_empty_status() {
        let w = 40u32;
        let h = 30u32;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for px in bgra.chunks_exact_mut(4) {
            px[0] = 160;
            px[1] = 160;
            px[2] = 160;
            px[3] = 255;
        }
        let st = sample_date_status_color(
            &bgra,
            w,
            h,
            &BBox {
                x0: 4,
                y0: 4,
                x1: 36,
                y1: 26,
            },
        );
        assert_eq!(st, Some(DateStatus::Empty));
    }

    #[test]
    fn warm_pill_samples_bookable_status() {
        let w = 40u32;
        let h = 30u32;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for px in bgra.chunks_exact_mut(4) {
            px[0] = 40; // B
            px[1] = 90; // G
            px[2] = 230; // R warm orange
            px[3] = 255;
        }
        let st = sample_date_status_color(
            &bgra,
            w,
            h,
            &BBox {
                x0: 4,
                y0: 4,
                x1: 36,
                y1: 26,
            },
        );
        assert_eq!(st, Some(DateStatus::Bookable));
    }

    #[test]
    fn text_color_conflict_is_incomplete() {
        let cs = set(&[
            ("今天", 200, 180),
            ("09月21日", 200, 210),
            ("暂无", 200, 250),
        ]);
        let mut cards = locate_date_cards(&cs);
        assert_eq!(cards.len(), 1);
        cards[0].status_color = Some(DateStatus::Bookable);
        cards[0].sources.status_color = Some(DateStatus::Bookable);
        cards[0].sources.status_text = Some(DateStatus::Empty);
        let cov = build_date_strip_coverage(&cards, "2026-09-21", 900, 900);
        assert!(matches!(
            assess_frame_date(&cards, "2026-09-21", &cov),
            FrameDateEvidence::Incomplete {
                why
            } if why == "status_text_color_conflict"
        ));
    }
}
