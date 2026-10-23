//! Load synthetic golden OCR fixtures (no PII images required).

#[cfg(test)]
mod golden_tests {
    use serde::Deserialize;
    use vision_core::{
        classify_from_hits, CaptchaPhase, ClassifyInput, OcrHit, OcrRegionCache, Page, SlotCta,
    };

    #[derive(Deserialize)]
    struct GoldenFile {
        ocr_hits: Vec<Hit>,
        #[serde(default)]
        has_mint_slots: bool,
        #[serde(default)]
        captcha_input_chars: String,
        #[serde(default)]
        keyboard_open: bool,
        #[serde(default)]
        slot_cta: Option<String>,
        frame: FrameSize,
        expected: Expected,
    }

    #[derive(Deserialize)]
    struct Hit {
        text: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        score: f32,
    }

    #[derive(Deserialize)]
    struct FrameSize {
        width: i32,
        height: i32,
    }

    #[derive(Deserialize)]
    struct Expected {
        page: String,
        #[serde(default)]
        captcha: Option<String>,
        min_confidence: f32,
    }

    fn load(name: &str) -> GoldenFile {
        let path = format!("testdata/golden/{name}");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        serde_json::from_str(&raw).expect("parse golden")
    }

    fn run(name: &str) {
        let g = load(name);
        let hits: Vec<OcrHit> = g
            .ocr_hits
            .into_iter()
            .map(|h| OcrHit {
                text: h.text,
                x: h.x,
                y: h.y,
                w: h.w,
                h: h.h,
                score: h.score,
            })
            .collect();
        let ocr = OcrRegionCache::from_hits(hits);
        let slot_cta = match g.slot_cta.as_deref() {
            Some("ready") => SlotCta::Ready,
            Some("need_select") => SlotCta::NeedSelect,
            _ => SlotCta::Unknown,
        };
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: g.frame.width,
            frame_h: g.frame.height,
            ocr: &ocr,
            title_hint: String::new(),
            has_mint_slots: g.has_mint_slots,
            captcha_input_chars: g.captcha_input_chars,
            keyboard_open: g.keyboard_open,
            toast_kind: None,
            slot_cta,
        });
        assert_eq!(snap.page.as_str(), g.expected.page);
        assert!(snap.confidence >= g.expected.min_confidence);
        if let Some(cap) = g.expected.captcha {
            let got = match snap.captcha {
                CaptchaPhase::IdleFull => "idle_full",
                CaptchaPhase::IdleEmpty => "idle_empty",
                other => panic!("unexpected captcha {other:?}"),
            };
            assert_eq!(got, cap);
        }
        assert_ne!(snap.page, Page::Unknown);
    }

    #[test]
    fn golden_select_slot() {
        run("select_slot_synth.json");
    }

    #[test]
    fn golden_confirm_order() {
        run("confirm_order_synth.json");
    }
}
