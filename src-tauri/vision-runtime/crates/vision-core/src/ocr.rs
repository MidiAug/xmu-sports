use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrHit {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub score: f32,
}

impl OcrHit {
    pub fn cx(&self) -> i32 {
        self.x + self.w / 2
    }
    pub fn cy(&self) -> i32 {
        self.y + self.h / 2
    }
    pub fn contains_needle(&self, needle: &str) -> bool {
        text_matches_needle(&self.text, needle)
    }
}

/// Same-frame OCR cache �?classifiers share one pass.
#[derive(Debug, Clone, Default)]
pub struct OcrRegionCache {
    hits: Vec<OcrHit>,
}

impl OcrRegionCache {
    pub fn from_hits(hits: Vec<OcrHit>) -> Self {
        let mut hits: Vec<OcrHit> = hits
            .into_iter()
            .map(|mut h| {
                h.text = normalize_ocr_text(&h.text);
                h
            })
            .collect();
        // 1080p often splits one label into single glyphs. Glue only touching
        // boxes so neighboring icons on the same row stay separate.
        join_split_labels(&mut hits);
        Self { hits }
    }

    pub fn hits(&self) -> &[OcrHit] {
        &self.hits
    }

    pub fn find_text(&self, needle: &str, min_score: f32) -> Option<&OcrHit> {
        let needle = normalize_ocr_text(needle);
        // Prefer longer / higher-score hits so line boxes beat single glyphs.
        self.hits
            .iter()
            .filter(|h| h.score >= min_score && text_matches_needle(&h.text, &needle))
            .max_by(|a, b| {
                let la = a.text.chars().count();
                let lb = b.text.chars().count();
                la.cmp(&lb).then_with(|| {
                    a.score
                        .partial_cmp(&b.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            })
    }

    pub fn body_has(&self, needle: &str, y0_frac: f32, y1_frac: f32, frame_h: i32) -> bool {
        let y0 = (frame_h as f32 * y0_frac) as i32;
        let y1 = (frame_h as f32 * y1_frac) as i32;
        let needle = normalize_ocr_text(needle);
        self.hits.iter().any(|h| {
            h.score >= 0.5 && text_matches_needle(&h.text, &needle) && h.y >= y0 && h.y <= y1
        })
    }
}

/// Drop spaces between CJK so "xiao cheng xu" glyphs join into one label.
pub fn normalize_ocr_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            let between_cjk =
                prev.map(is_cjk).unwrap_or(false) && next.map(is_cjk).unwrap_or(false);
            if between_cjk {
                continue;
            }
            if out.ends_with(' ') {
                continue;
            }
            out.push(' ');
            continue;
        }
        if c.is_control() {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// Exact containment after CJK-space normalize. One-character fuzzy is rejected
/// (perception spec: do not patch OCR typos).
pub fn labels_match_exact(hay: &str, needle: &str) -> bool {
    text_matches_needle(hay, needle)
}

fn text_matches_needle(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if hay.contains(needle) {
        return true;
    }
    let h = normalize_ocr_text(hay);
    let n = normalize_ocr_text(needle);
    if n.is_empty() {
        return false;
    }
    h.contains(&n)
}

fn join_split_labels(hits: &mut Vec<OcrHit>) {
    if hits.len() < 2 {
        return;
    }
    let mut order: Vec<usize> = (0..hits.len()).collect();
    order.sort_by(|&a, &b| hits[a].y.cmp(&hits[b].y).then(hits[a].x.cmp(&hits[b].x)));
    let mut used = vec![false; hits.len()];
    let mut extra = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let start = order[i];
        if used[start] || hits[start].text.chars().count() > 3 {
            i += 1;
            continue;
        }
        let mut group = vec![start];
        let mut cursor = start;
        let mut j = i + 1;
        while j < order.len() {
            let k = order[j];
            if used[k] || hits[k].text.chars().count() > 3 || !glyph_adjacent(&hits[cursor], &hits[k])
            {
                j += 1;
                continue;
            }
            group.push(k);
            cursor = k;
            j += 1;
        }
        if group.len() >= 2 {
            let text: String = group.iter().map(|&g| hits[g].text.as_str()).collect();
            if (2..=8).contains(&text.chars().count()) {
                let x0 = group.iter().map(|&g| hits[g].x).min().unwrap_or(0);
                let y0 = group.iter().map(|&g| hits[g].y).min().unwrap_or(0);
                let x1 = group.iter().map(|&g| hits[g].x + hits[g].w).max().unwrap_or(x0 + 1);
                let y1 = group.iter().map(|&g| hits[g].y + hits[g].h).max().unwrap_or(y0 + 1);
                let score = group
                    .iter()
                    .map(|&g| hits[g].score)
                    .fold(0.0f32, f32::max);
                extra.push(OcrHit {
                    text,
                    x: x0,
                    y: y0,
                    w: (x1 - x0).max(1),
                    h: (y1 - y0).max(1),
                    score,
                });
                for g in group {
                    used[g] = true;
                }
            }
        }
        i += 1;
    }
    hits.extend(extra);
}

fn glyph_adjacent(a: &OcrHit, b: &OcrHit) -> bool {
    let dy = (a.cy() - b.cy()).abs();
    if dy > (a.h.max(b.h) / 2).max(6) {
        return false;
    }
    let gap = b.x - (a.x + a.w);
    gap >= -2 && gap <= (a.h.max(b.h) / 2).max(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_drops_cjk_spaces() {
        // 「小 程 序」→「小程序」
        assert_eq!(
            normalize_ocr_text("\u{5c0f} \u{7a0b} \u{5e8f}"),
            "\u{5c0f}\u{7a0b}\u{5e8f}"
        );
        assert_eq!(
            normalize_ocr_text("@ \u{670b} \u{53cb} \u{5708}"),
            "@ \u{670b}\u{53cb}\u{5708}"
        );
    }

    #[test]
    fn find_text_rejects_one_wrong_cjk() {
        // 「序」misread as「孛」must not count as「小程序」.
        let ocr = OcrRegionCache::from_hits(vec![OcrHit {
            text: "\u{5c0f}\u{7a0b}\u{5b5a}".into(),
            x: 80,
            y: 200,
            w: 60,
            h: 18,
            score: 0.8,
        }]);
        assert!(ocr.find_text("\u{5c0f}\u{7a0b}\u{5e8f}", 0.5).is_none());
    }

    #[test]
    fn find_text_still_exact() {
        let ocr = OcrRegionCache::from_hits(vec![OcrHit {
            text: "\u{53a6}\u{5927}\u{4f53}\u{80b2}".into(),
            x: 10,
            y: 10,
            w: 80,
            h: 18,
            score: 0.9,
        }]);
        assert!(ocr
            .find_text("\u{53a6}\u{5927}\u{4f53}\u{80b2}", 0.5)
            .is_some());
    }

    #[test]
    fn split_glyphs_join_into_the_label() {
        let ocr = OcrRegionCache::from_hits(vec![
            OcrHit {
                text: "羽".into(),
                x: 100,
                y: 200,
                w: 14,
                h: 16,
                score: 0.8,
            },
            OcrHit {
                text: "毛".into(),
                x: 114,
                y: 200,
                w: 14,
                h: 16,
                score: 0.8,
            },
            OcrHit {
                text: "球".into(),
                x: 128,
                y: 201,
                w: 14,
                h: 16,
                score: 0.8,
            },
        ]);
        assert!(ocr.find_text("羽毛球", 0.5).is_some());
    }

    #[test]
    fn header_link_is_not_booked_status() {
        let ocr = OcrRegionCache::from_hits(vec![OcrHit {
            text: "我的预约".into(),
            x: 300,
            y: 40,
            w: 64,
            h: 16,
            score: 0.9,
        }]);
        assert!(ocr.find_text("已预约", 0.5).is_none());
    }
}
