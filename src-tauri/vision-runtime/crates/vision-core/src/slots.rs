use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotState {
    Available,
    Locked,
    Unavailable,
}

pub type FallbackOrder = &'static str; // "up_then_down" | "down_then_up"

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotCandidate {
    pub start_time: String,
    pub end_time: String,
    pub court_index: Option<i32>,
    pub court_label: Option<String>,
    pub state: SlotState,
    /// Pixel coords inside Frame.content_rect — never overnight screen coords.
    pub click_x: i32,
    pub click_y: i32,
    pub evidence: Vec<String>,
}

impl SlotCandidate {
    pub fn available(&self) -> bool {
        self.state == SlotState::Available
    }
}

pub fn parse_time_label(text: &str) -> Option<String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let re = regex_lite_time(&compact)?;
    Some(re)
}

fn regex_lite_time(s: &str) -> Option<String> {
    // Minimal HH:MM parser without regex crate dependency.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b':' && j + 3 <= bytes.len() {
                let h = std::str::from_utf8(&bytes[i..j])
                    .ok()?
                    .parse::<u32>()
                    .ok()?;
                if bytes[j + 1].is_ascii_digit() && bytes[j + 2].is_ascii_digit() {
                    let m = std::str::from_utf8(&bytes[j + 1..j + 3])
                        .ok()?
                        .parse::<u32>()
                        .ok()?;
                    if h <= 23 && m <= 59 {
                        return Some(format!("{h:02}:{m:02}"));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Target first, then neighbors by distance; tie-break by up/down preference.
pub fn nearest_time_order(times: &[String], target: &str, order: &str) -> Vec<String> {
    if times.is_empty() {
        return if target.trim().is_empty() {
            vec![]
        } else {
            vec![target.to_string()]
        };
    }
    let mut normed: Vec<String> = Vec::new();
    for t in times {
        let p = parse_time_label(t).unwrap_or_else(|| t.clone());
        if !normed.contains(&p) {
            normed.push(p);
        }
    }
    let needle = parse_time_label(target).unwrap_or_else(|| target.trim().to_string());
    if needle.is_empty() {
        return normed;
    }
    let up_first = order == "up_then_down";
    if !normed.contains(&needle) {
        // Target row absent (全满/未检出) — rank visible times by clock distance.
        let target_m = time_minutes(&needle);
        normed.sort_by(|a, b| {
            let ma = time_minutes(a);
            let mb = time_minutes(b);
            let da = (ma - target_m).abs();
            let db = (mb - target_m).abs();
            da.cmp(&db).then_with(|| {
                if up_first {
                    ma.cmp(&mb)
                } else {
                    mb.cmp(&ma)
                }
            })
        });
        let mut out = vec![needle];
        out.extend(normed);
        return out;
    }
    let idx = normed.iter().position(|t| t == &needle).unwrap();
    let mut out = vec![needle];
    let max_d = idx.max(normed.len() - 1 - idx);
    for dist in 1..=max_d {
        let earlier = idx as isize - dist as isize;
        let later = idx + dist;
        if up_first {
            if earlier >= 0 {
                out.push(normed[earlier as usize].clone());
            }
            if later < normed.len() {
                out.push(normed[later].clone());
            }
        } else {
            if later < normed.len() {
                out.push(normed[later].clone());
            }
            if earlier >= 0 {
                out.push(normed[earlier as usize].clone());
            }
        }
    }
    out
}

fn time_minutes(t: &str) -> i32 {
    let parts: Vec<_> = t.split(':').collect();
    if parts.len() != 2 {
        return 0;
    }
    let h: i32 = parts[0].parse().unwrap_or(0);
    let m: i32 = parts[1].parse().unwrap_or(0);
    h * 60 + m
}

/// Reorder L→R available courts so index `min(offset, len-1)` is first;
/// then right neighbors, then the left segment skipped (for click retries).
pub fn apply_court_offset<T>(ordered: Vec<T>, offset: u32) -> Vec<T> {
    if ordered.len() <= 1 {
        return ordered;
    }
    let i = (offset as usize).min(ordered.len() - 1);
    if i == 0 {
        return ordered;
    }
    let mut ordered = ordered;
    let mut right = ordered.split_off(i);
    right.append(&mut ordered);
    right
}

pub fn pick_slots_for_time(
    candidates: &[SlotCandidate],
    target_time: &str,
    left_to_right: bool,
) -> Vec<SlotCandidate> {
    let needle = target_time.trim();
    if needle.is_empty() {
        let mut ordered: Vec<_> = candidates
            .iter()
            .filter(|c| c.available())
            .cloned()
            .collect();
        ordered.sort_by_key(|c| {
            (
                c.click_y,
                if left_to_right { c.click_x } else { -c.click_x },
            )
        });
        return ordered;
    }
    let parsed = parse_time_label(needle).unwrap_or_else(|| needle.to_string());
    let mut row: Vec<_> = candidates
        .iter()
        .filter(|c| {
            c.available()
                && (c.start_time == parsed
                    || c.start_time.ends_with(&parsed)
                    || parsed.ends_with(&c.start_time))
        })
        .cloned()
        .collect();
    row.sort_by_key(|c| if left_to_right { c.click_x } else { -c.click_x });
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time() {
        assert_eq!(parse_time_label("19:30").as_deref(), Some("19:30"));
        assert_eq!(parse_time_label("9:05").as_deref(), Some("09:05"));
    }

    #[test]
    fn fallback_when_target_absent_uses_clock_distance() {
        let times = vec![
            "09:00".into(),
            "12:00".into(),
            "18:00".into(),
            "21:00".into(),
        ];
        let order = nearest_time_order(&times, "19:30", "up_then_down");
        assert_eq!(order[0], "19:30");
        assert_eq!(order[1], "18:00", "nearest earlier when 先更早");
        assert_eq!(order[2], "21:00");
    }

    #[test]
    fn fallback_down_then_up() {
        let times = vec![
            "18:00".into(),
            "19:00".into(),
            "20:00".into(),
            "21:00".into(),
        ];
        let order = nearest_time_order(&times, "19:00", "down_then_up");
        assert_eq!(order, vec!["19:00", "20:00", "18:00", "21:00"]);
    }

    #[test]
    fn pick_ltr() {
        let cands = vec![
            SlotCandidate {
                start_time: "19:00".into(),
                end_time: String::new(),
                court_index: Some(0),
                court_label: Some("01".into()),
                state: SlotState::Available,
                click_x: 200,
                click_y: 100,
                evidence: vec![],
            },
            SlotCandidate {
                start_time: "19:00".into(),
                end_time: String::new(),
                court_index: Some(1),
                court_label: Some("02".into()),
                state: SlotState::Available,
                click_x: 100,
                click_y: 100,
                evidence: vec![],
            },
            SlotCandidate {
                start_time: "19:00".into(),
                end_time: String::new(),
                court_index: Some(2),
                court_label: Some("03".into()),
                state: SlotState::Locked,
                click_x: 50,
                click_y: 100,
                evidence: vec![],
            },
        ];
        let picked = pick_slots_for_time(&cands, "19:00", true);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].click_x, 100);
        assert_eq!(picked[1].click_x, 200);
    }

    #[test]
    fn apply_court_offset_skips_left() {
        let v = vec!["a", "b", "c", "d", "e"];
        assert_eq!(apply_court_offset(v, 2), vec!["c", "d", "e", "a", "b"]);
    }

    #[test]
    fn apply_court_offset_clamps_to_last() {
        let v = vec!["a", "b"];
        assert_eq!(apply_court_offset(v, 2), vec!["b", "a"]);
    }

    #[test]
    fn apply_court_offset_zero_and_empty() {
        assert!(apply_court_offset(Vec::<&str>::new(), 2).is_empty());
        assert_eq!(apply_court_offset(vec!["only"], 9), vec!["only"]);
        assert_eq!(apply_court_offset(vec!["a", "b", "c"], 0), vec!["a", "b", "c"]);
    }
}
