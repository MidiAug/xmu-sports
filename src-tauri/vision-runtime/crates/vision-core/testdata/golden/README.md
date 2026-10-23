# Vision Core 黄金集

> 脱敏 / 合成帧与 OCR 命中预期。Windows 实拍与 macOS 实拍分目录标注；禁止入库学号、订单、Cookie。

| id | platform | 用途 |
|---|---|---|
| `select_slot_synth` | synthetic | 场次页双证据认页 |
| `confirm_order_synth` | synthetic | 确认页 + captcha phase |

真实截图入库前须人工脱敏，并在 meta 中写 `platform=windows|macos` 与采集日期。
