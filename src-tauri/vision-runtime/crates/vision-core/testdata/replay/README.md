# Replay corpus（Plan P6 骨架）

失败 run 晋升的离线回放样本：`observe_shadow` + `decide_next` 对比 expected，输出页召回 / 意图匹配 / 误点粗指标。

## 目录约定

```text
testdata/replay/<case_id>/
  manifest.json   # case_id、tags、goal、source(run)、label_source
  frame.json      # frame 尺寸 + 脱敏 OCR hits（默认无 BMP）
  expected.json   # page / refuse_reason / intent_kind|label|reason / date
```

入库规则：

- **默认只提交 OCR 标注**，不提交微信界面截图；本地可有可选 `frame_bmp`（勿 git）
- 晋升时自动脱敏 digit-heavy / 姓名提示 / 学号等关键词（见 `desensitize_ocr_hits`）
- `human_confirmed=false` 的样本可跑通 runner，但不作为「矩阵已填满」证明

## 晋升（失败 run → 样本）

```powershell
cargo run -p vision-platform-windows --bin vision-shadow-diag -- promote `
  --run-dir "$env:LOCALAPPDATA\xmu-sports\runs\<task_id>\<run_id>" `
  --case-id my_fail_case `
  --page venue_detail `
  --date 2026-09-28 `
  --intent-kind wait `
  --intent-reason DATE_EVIDENCE_WEAK `
  --tag date_strip_partial `
  --confirm
```

需要该 run 的 observations 含 `ocr_hits`（详细诊断打开）。默认写出到  
`%LOCALAPPDATA%/xmu-sports/diag/replay/<case_id>/`；加 `--to-repo` 写到本目录（请先目视脱敏）。

## 离线 runner

```powershell
cargo run -p vision-platform-windows --bin vision-shadow-diag -- corpus --dir testdata/replay
# 或单测：
cargo test --manifest-path src-tauri/Cargo.toml -p vision-core replay
```

## 目标矩阵（本期不要求填满）

| 标签 | 含义 |
|------|------|
| `dpi_100` / `dpi_125` / `dpi_150` | 系统缩放 |
| `animating` | 过渡/滚动中帧 |
| `split_boxes` | OCR 拆框（如 `09月`+`28`+`日`） |
| `cross_month` | 日期条跨月 |
| `date_strip_partial` | 日期条覆盖不足 |
| `overlay_notice` | 提示弹窗挡操作 |
| `slot_grid_unlocalized` | 场次格未定位 |

常量：`vision_core::REPLAY_MATRIX_TAGS`。

## 与 shadow / page_map 的关系

| 目录 | 用途 |
|------|------|
| `testdata/shadow/` | 早期 annotated shadow 回归（单文件） |
| `testdata/page_map/` | 实拍认页门禁（confirm + e2e_ready） |
| `testdata/replay/` | **失败晋升闭环**：页 + 日期标注 + `decide_next` 意图 |

Annotated OCR ≠ 真机 E2E；填矩阵仍优先 page_map 实拍。
