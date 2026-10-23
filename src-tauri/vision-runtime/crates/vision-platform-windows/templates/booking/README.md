# Booking UI templates (from simple-booking probe)

Copied from `prototypes/simple-booking/templates/` for native Pack / offline
matching. Do not import Python. Prefer OCR + color candidates at runtime;
these PNGs are reference assets for future gray-needle conversion.

## Re-cropped (2026-09-23)

- `agree_row.png` — 未勾选圆圈 +「阅读并同意以下协议」整行。已删 `agree_radio.png`（整行可点）。
- `notice_title.png` — 「【预约须知】」标题。

运行时授权勾选：OCR 命中「阅读并同意…」后直接点文字 bbox。
