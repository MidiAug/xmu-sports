//! Post-click L2 readiness: wait for next actionable locator(s), not page taxonomy.
//!
//! Evidence (`docs/research/l2-probe-budget-bench.md`):
//! - Probe templates only on stable/force (same cadence as ROI) — never every animating frame.
//! - Matcher = vision-core down×4 + sparse ZNCC + refine (`template_band_best_score`).
//! - OR short-circuit; for 场馆预约 prefer goal sport icon before notice templates.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use vision_core::{template_band_best_score, Page, TaskGoal, TemplateNeedle};

const TPL_HIT: f32 = 0.82;
/// Coarse multi-scale ladder (bench: aspect/DPI drift on real 520×719 windows).
const BOOKING_SCALES: [f32; 5] = [0.70, 0.85, 1.0, 1.15, 1.30];

#[derive(Debug, Clone)]
pub struct UntilSpec {
    pub anchors: Vec<UntilAnchor>,
}

#[derive(Debug, Clone)]
pub enum UntilAnchor {
    Template { id: &'static str, path: PathBuf },
    SportIcon { label: String },
    RoiText {
        needle: String,
        y0: f32,
        y1: f32,
    },
}

impl UntilSpec {
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    pub fn labels_for_log(&self) -> Vec<String> {
        self.anchors
            .iter()
            .map(|a| match a {
                UntilAnchor::Template { id, .. } => (*id).to_string(),
                UntilAnchor::SportIcon { label } => label.clone(),
                UntilAnchor::RoiText { needle, .. } => needle.clone(),
            })
            .collect()
    }
}

fn booking_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/booking")
}

fn sports_dir() -> PathBuf {
    booking_dir().join("sports")
}

fn sport_template_file(label: &str) -> Option<&'static str> {
    let n = label.trim();
    Some(match n {
        "篮球" => "01_basketball.png",
        "乒乓球" => "02_pingpong.png",
        "羽毛球" => "03_badminton.png",
        "足球" => "04_football.png",
        "排球" => "05_volleyball.png",
        "游泳馆" | "游泳" => "06_swim.png",
        "动感单车室" | "动感单车" => "07_cycling.png",
        "健身房" => "08_gym.png",
        "网球" => "09_tennis.png",
        "瑜伽" => "10_yoga.png",
        "匹克球" => "11_pickleball.png",
        _ => return None,
    })
}

fn filled(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|t| !t.is_empty())
}

/// Click-driven until OR-set. Sport icon before notice for 场馆预约 (short-circuit).
pub fn until_after_click(clicked: &str, expected_page: Page, goal: &TaskGoal) -> UntilSpec {
    let clicked = clicked.trim();
    let sport = filled(goal.sport.as_deref());
    let venue = filled(goal.venue.as_deref());

    if clicked == "场馆预约" {
        let mut anchors = Vec::new();
        // Prefer goal sport: always needed; notice is an optional intermediate.
        if let Some(s) = sport {
            if sport_template_file(s).is_some() {
                anchors.push(UntilAnchor::SportIcon {
                    label: s.to_string(),
                });
            }
        }
        anchors.push(UntilAnchor::Template {
            id: "我知道了",
            path: booking_dir().join("i_know.png"),
        });
        anchors.push(UntilAnchor::Template {
            id: "预约须知",
            path: booking_dir().join("notice_title.png"),
        });
        return UntilSpec { anchors };
    }

    if clicked == "我知道了" {
        if let Some(s) = sport {
            if sport_template_file(s).is_some() {
                return UntilSpec {
                    anchors: vec![UntilAnchor::SportIcon {
                        label: s.to_string(),
                    }],
                };
            }
        }
        return UntilSpec { anchors: vec![] };
    }

    if sport.is_some_and(|s| clicked == s) || expected_page == Page::VenueList {
        let mut anchors = vec![UntilAnchor::RoiText {
            needle: "场馆列表".into(),
            y0: 0.0,
            y1: 0.25,
        }];
        if let Some(v) = venue {
            anchors.push(UntilAnchor::RoiText {
                needle: v.to_string(),
                y0: 0.05,
                y1: 0.55,
            });
        }
        return UntilSpec { anchors };
    }

    match expected_page {
        Page::VenueSportList => {
            if let Some(s) = sport {
                if sport_template_file(s).is_some() {
                    return UntilSpec {
                        anchors: vec![UntilAnchor::SportIcon {
                            label: s.to_string(),
                        }],
                    };
                }
            }
            UntilSpec { anchors: vec![] }
        }
        Page::VenueList => {
            let mut anchors = vec![UntilAnchor::RoiText {
                needle: "场馆列表".into(),
                y0: 0.0,
                y1: 0.25,
            }];
            if let Some(v) = venue {
                anchors.push(UntilAnchor::RoiText {
                    needle: v.to_string(),
                    y0: 0.05,
                    y1: 0.55,
                });
            }
            UntilSpec { anchors }
        }
        _ => UntilSpec { anchors: vec![] },
    }
}

/// True when any until-anchor is visible. Short-circuits on first hit.
pub fn any_locator_hit(
    width: u32,
    height: u32,
    bgra: &[u8],
    until: &UntilSpec,
) -> Option<String> {
    if until.is_empty() || width < 32 || height < 32 || bgra.len() < (width * height * 4) as usize
    {
        return None;
    }
    for a in &until.anchors {
        if locator_hit(width, height, bgra, a) {
            let label = match a {
                UntilAnchor::Template { id, .. } => (*id).to_string(),
                UntilAnchor::SportIcon { label } => label.clone(),
                UntilAnchor::RoiText { needle, .. } => needle.clone(),
            };
            return Some(label);
        }
    }
    None
}

fn locator_hit(width: u32, height: u32, bgra: &[u8], a: &UntilAnchor) -> bool {
    match a {
        UntilAnchor::Template { id, path } => {
            let Some(needle) = load_png_needle(id, path) else {
                return false;
            };
            let score =
                template_band_best_score(bgra, width, height, &needle, 0.0, 0.92, &BOOKING_SCALES);
            if score >= TPL_HIT {
                log::info!(
                    target: "glance",
                    "[locator] template hit id={id} score={score:.3}"
                );
                true
            } else {
                false
            }
        }
        UntilAnchor::SportIcon { label } => {
            let Some(file) = sport_template_file(label) else {
                return false;
            };
            let path = sports_dir().join(file);
            let Some(needle) = load_png_needle(label, &path) else {
                return false;
            };
            // Sport grid lives in the upper ~72%.
            let score =
                template_band_best_score(bgra, width, height, &needle, 0.0, 0.72, &BOOKING_SCALES);
            if score >= TPL_HIT {
                log::info!(
                    target: "glance",
                    "[locator] sport template hit label={label} score={score:.3}"
                );
                true
            } else {
                false
            }
        }
        UntilAnchor::RoiText { needle, y0, y1 } => {
            match roi_text_contains(width, height, bgra, *y0, *y1, needle) {
                Ok(true) => {
                    log::info!(target: "glance", "[locator] ROI text hit label={needle}");
                    true
                }
                Ok(false) => false,
                Err(e) => {
                    log::debug!(target: "glance", "[locator] ROI OCR skip: {e}");
                    false
                }
            }
        }
    }
}

fn load_png_needle(id: &str, path: &Path) -> Option<TemplateNeedle> {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, TemplateNeedle>>> =
        OnceLock::new();
    let key = path.to_string_lossy().to_string();
    let map = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = map.lock() {
        if let Some(n) = guard.get(&key) {
            return Some(n.clone());
        }
    }
    if !path.exists() {
        return None;
    }
    let rgb = image::open(path).ok()?.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 || w * h > 256 * 256 {
        return None;
    }
    let mut gray = Vec::with_capacity((w * h) as usize);
    for p in rgb.pixels() {
        let g = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8;
        gray.push(g);
    }
    let needle = TemplateNeedle {
        id: id.to_string(),
        label: id.to_string(),
        width: w,
        height: h,
        gray,
    };
    if let Ok(mut guard) = map.lock() {
        guard.insert(key, needle.clone());
    }
    Some(needle)
}

fn roi_text_contains(
    width: u32,
    height: u32,
    bgra: &[u8],
    y0_frac: f32,
    y1_frac: f32,
    needle: &str,
) -> Result<bool, String> {
    let y0 = ((height as f32) * y0_frac) as u32;
    let y1 = ((height as f32) * y1_frac).min(height as f32) as u32;
    if y1 <= y0 + 8 {
        return Ok(false);
    }
    let ch = y1 - y0;
    let mut crop = Vec::with_capacity((width * ch * 4) as usize);
    for y in y0..y1 {
        let row = (y * width * 4) as usize;
        crop.extend_from_slice(&bgra[row..row + (width * 4) as usize]);
    }
    let (regions, _) = crate::rapid_ocr::recognize_bgra(width, ch, &crop)?;
    let n = needle.trim();
    Ok(regions.hits().iter().any(|h| h.text.contains(n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sport_files_map() {
        assert_eq!(sport_template_file("健身房"), Some("08_gym.png"));
        assert!(sport_template_file("翔安校区健身房").is_none());
    }

    #[test]
    fn until_booking_entry_sport_before_notice() {
        let goal = TaskGoal {
            sport: Some("健身房".into()),
            venue: Some("翔安校区健身房".into()),
            ..TaskGoal::default()
        };
        let labels = until_after_click("场馆预约", Page::VenueSportList, &goal).labels_for_log();
        assert_eq!(labels[0], "健身房");
        assert!(labels.iter().any(|s| s == "我知道了"));
        assert!(labels.iter().any(|s| s == "预约须知"));
    }

    #[test]
    fn until_after_i_know_is_sport() {
        let goal = TaskGoal {
            sport: Some("健身房".into()),
            ..TaskGoal::default()
        };
        let u = until_after_click("我知道了", Page::VenueSportList, &goal);
        assert_eq!(u.labels_for_log(), vec!["健身房".to_string()]);
    }

    #[test]
    fn until_after_sport_includes_list_title() {
        let goal = TaskGoal {
            sport: Some("健身房".into()),
            venue: Some("翔安校区健身房".into()),
            ..TaskGoal::default()
        };
        let labels = until_after_click("健身房", Page::VenueList, &goal).labels_for_log();
        assert!(labels.iter().any(|s| s == "场馆列表"));
        assert!(labels.iter().any(|s| s == "翔安校区健身房"));
    }

    #[test]
    fn gym_png_loads_as_needle() {
        let path = sports_dir().join("08_gym.png");
        let n = load_png_needle("健身房", &path).expect("gym template");
        assert!(n.width >= 32 && n.height >= 32);
        assert_eq!(n.gray.len(), (n.width * n.height) as usize);
    }
}
