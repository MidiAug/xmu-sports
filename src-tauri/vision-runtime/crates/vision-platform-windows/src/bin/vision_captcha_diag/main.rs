//! Offline captcha OCR diag — same model as glance, with optional strategies.
//!
//! ```text
//! cargo run -p vision-platform-windows --bin vision-captcha-diag --features diag-bin -- \
//!   [--strategy default|alnum|ink_colors|no_upscale] scripts/fixtures/captcha-ocr
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use ddddocr_core::traits::Loader;
use ddddocr_core::{
    CharRestrict, ColorPreset, HsvRange, ModelMetadata, Normalization, Ocr, Resize,
};
use ddddocr_ort::loader::ModelLoader;
use ddddocr_ort::OcrRuntime;
use image::{DynamicImage, RgbImage};
use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strategy {
    /// Product-like: upscale 2.5× then ddddocr + ASCII normalize.
    Default,
    /// Restrict CTC argmax to 0-9A-Za-z (no Chinese「工」leak).
    Alnum,
    /// HSV keep dark/saturated ink (multi ColorPreset + custom).
    InkColors,
    /// Skip 2.5× upscale (model already DynamicWidth(64)).
    NoUpscale,
}

fn main() {
    let mut strategy = Strategy::Default;
    let mut dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--strategy" {
            let v = args.next().unwrap_or_default();
            strategy = match v.as_str() {
                "default" => Strategy::Default,
                "alnum" => Strategy::Alnum,
                "ink_colors" => Strategy::InkColors,
                "no_upscale" => Strategy::NoUpscale,
                other => {
                    eprintln!("unknown strategy: {other}");
                    std::process::exit(2);
                }
            };
        } else if a.starts_with('-') {
            eprintln!("unknown flag: {a}");
            std::process::exit(2);
        } else {
            dir = Some(PathBuf::from(a));
        }
    }
    let dir = dir.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../scripts/fixtures/captcha-ocr")
    });
    if !dir.is_dir() {
        eprintln!("not a directory: {}", dir.display());
        std::process::exit(2);
    }

    let re = Regex::new(r"(?i)(?:^|_)([A-Za-z0-9]{4})\.png$").expect("regex");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        })
        .collect();
    entries.sort();

    let mut ok = 0usize;
    let mut n = 0usize;
    println!("dir={} strategy={strategy:?}", dir.display());
    for path in entries {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let Some(caps) = re.captures(name) else {
            println!("skip {name}");
            continue;
        };
        let expect = caps[1].to_ascii_uppercase();
        n += 1;
        match recognize_one(&path, strategy) {
            Ok(report) => {
                let mark = if report.normalized == expect {
                    ok += 1;
                    "OK"
                } else if report.normalized.chars().count() == 4 {
                    "WRONG"
                } else {
                    "SHORT"
                };
                println!(
                    "{name:16} expect={expect} got={:<8} raw={:<14} mark={mark} ms={}",
                    report.normalized,
                    format!("{:?}", report.raw),
                    report.ms
                );
            }
            Err(e) => println!("{name:16} expect={expect} ERR={e}"),
        }
    }
    println!("summary {ok}/{n}");
    if n == 0 {
        std::process::exit(2);
    }
    if ok < n {
        std::process::exit(1);
    }
}

struct Report {
    raw: String,
    normalized: String,
    ms: u128,
}

fn recognize_one(path: &Path, strategy: Strategy) -> Result<Report, String> {
    let bytes = fs::read(path).map_err(|e| format!("read: {e}"))?;
    let rgb = image::load_from_memory(&bytes)
        .map_err(|e| format!("decode: {e}"))?
        .to_rgb8();
    let t0 = Instant::now();

    let (raw, normalized) = match strategy {
        Strategy::Default => {
            // Mirror product: upscale then ddddocr normalize.
            let (w, h) = rgb.dimensions();
            let bgra = rgb_to_bgra(&rgb);
            let got = vision_platform_windows::recognize_captcha_bgra(w, h, &bgra)?;
            let raw = predict_raw(&rgb, PredOpts::default())?;
            (raw, got)
        }
        Strategy::NoUpscale => {
            let raw = predict_raw(&rgb, PredOpts::default())?;
            (raw.clone(), normalize_code(&raw))
        }
        Strategy::Alnum => {
            let raw = predict_raw(
                &rgb,
                PredOpts {
                    alnum: true,
                    ..PredOpts::default()
                },
            )?;
            (raw.clone(), normalize_code(&raw))
        }
        Strategy::InkColors => {
            let raw = predict_raw(
                &rgb,
                PredOpts {
                    ink_colors: true,
                    alnum: true,
                    ..PredOpts::default()
                },
            )?;
            (raw.clone(), normalize_code(&raw))
        }
    };
    let ms = t0.elapsed().as_millis();
    let normalized = normalized.chars().take(4).collect::<String>();
    Ok(Report {
        raw,
        normalized,
        ms,
    })
}

#[derive(Default)]
struct PredOpts {
    alnum: bool,
    ink_colors: bool,
}

fn engine() -> Result<&'static OcrRuntime, String> {
    static ENGINE: OnceLock<OcrRuntime> = OnceLock::new();
    if let Some(e) = ENGINE.get() {
        return Ok(e);
    }
    let path = std::env::var_os("XMU_SPORTS_CAPTCHA_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("xmu-sports")
                .join("captcha")
                .join("common.onnx")
        });
    if !path.is_file() {
        return Err(format!("model missing: {}", path.display()));
    }
    let session = ModelLoader::default()
        .num_threads(2)
        .build_for_path(&path)
        .map_err(|e| format!("load: {e}"))?;
    let metadata = ModelMetadata::from_builtin_beta(
        false,
        Resize::DynamicWidth(64),
        1,
        Normalization::MinusOneToOne,
    );
    let _ = ENGINE.set(OcrRuntime::new(session, metadata));
    Ok(ENGINE.get().expect("engine set"))
}

fn predict_raw(rgb: &RgbImage, opts: PredOpts) -> Result<String, String> {
    let runtime = engine()?;
    let image = DynamicImage::ImageRgb8(rgb.clone());
    let mut builder = Ocr::builder();
    if opts.alnum {
        let alnum: Vec<String> = (b'0'..=b'9')
            .chain(b'a'..=b'z')
            .chain(b'A'..=b'Z')
            .map(|b| (b as char).to_string())
            .collect();
        builder = builder.charset_restrict(CharRestrict::CustomList(alnum));
    }
    if opts.ink_colors {
        // Multi-hue dark ink on pale blue paper — keep several presets + custom dark.
        // ColorPreset implements ColorFilter; use Custom for wide ink band.
        let ink = ColorPreset::Custom(vec![
            // Dark any-hue
            HsvRange::new((0, 40, 0), (180, 255, 140)),
            // Saturated mid tones (green/gold/purple glyphs)
            HsvRange::new((0, 80, 40), (180, 255, 200)),
        ]);
        builder = builder.color_filter(ink);
    }
    builder
        .build_with(runtime)
        .predict(&image)
        .map(|r| r.into_text())
        .map_err(|e| format!("infer: {e}"))
}

fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn rgb_to_bgra(rgb: &RgbImage) -> Vec<u8> {
    let (w, h) = rgb.dimensions();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in rgb.pixels() {
        out.extend_from_slice(&[p.0[2], p.0[1], p.0[0], 255]);
    }
    out
}
