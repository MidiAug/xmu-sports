# Page map（阶段 4 实拍验收）

目标：无输入地证明「认对页面 + 区分相似页 + 标出关键框」。  
**未完成此前不得进入阶段 5 点击。**

## 门禁（防虚假通过）

1. **未确认绝不评分**：`human_confirmed=false` 或 `label_source=pending` 一律 `skipped_unconfirmed`（即使有预填 `expected`）。
2. **标签必须人工独立填写**：`capture` 只写 `page_label_suggested`；看完 `frame.bmp` 后跑 `confirm` 键入 `page_label`（`label_source=human`）。禁止批量改 `human_confirmed=true`。
3. **e2e_ready**：需真实 OCR + 人工标签 + 核心路径 **holdout 齐全** + 无错/UNKNOWN + `input_invoked=false`。仅手标 OCR → 永远 false。
4. **导入**：不复制 BMP ≠ 已脱敏；OCR 敏感扫描命中须 `--ack-ocr-pii-reviewed`。

## 两类样本

| 类型 | `ocr_source` / `label_source` | 能证明 | 不能证明 |
|---|---|---|---|
| 手标 OCR | `annotated` / `annotated_fixture` | 规则收到正确文字会分类 | 真实截图 OCR 认对 |
| 实拍 WGC+系统 OCR | `windows_media` / `human` | 端到端认页 | — |

## 流程

```powershell
cargo run -p vision-platform-windows --bin vision-shadow-diag -- capture --session path_manual
# 目视 frame.bmp，遮 PII
cargo run -p vision-platform-windows --bin vision-shadow-diag -- confirm --dir $env:LOCALAPPDATA\xmu-sports\diag\page_map\path_manual
cargo run -p vision-platform-windows --bin vision-shadow-diag -- eval --dir ...\path_manual --real-ocr-only
cargo run -p vision-platform-windows --bin vision-shadow-diag -- eval --dir ...\path_manual --split holdout --real-ocr-only
```

`h` 采 holdout；场馆名无法核实 → `UNKNOWN`；场次未定位 → `SLOT_GRID_NOT_LOCALIZED` + Stop。

## 本目录手标样例

`xmu_login_gate.json` / `xmu_wx_authorize.json` — 规则回归 only，`e2e_ready=false`。
