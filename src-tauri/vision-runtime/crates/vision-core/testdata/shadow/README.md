# Shadow annotated / replay set (Windows)

Desensitized **OCR-hit annotations** for Phase 4 Shadow Mode. No raw screenshots or PII in git.

| Fixture | Page intent | Expected refuse |
|---|---|---|
| `select_slot_normal.json` | 场次页（有时间文字） | `SLOT_GRID_NOT_LOCALIZED`（**禁止**弱启发 Click） |
| `popup_notice.json` | 提示弹窗 | `OVERLAY_BLOCKS_ACTION` |
| `select_slot_empty.json` | 场次页无格 | `NO_AVAILABLE_SLOTS` |
| `unknown_wechat_chat.json` | 微信聊天（非预约） | `UNKNOWN_PAGE` |
| `wechat_mp_panel.json` | 小程序面板（有厦大体育） | Shadow `ActionPlan` 仍 Stop；`decide_next` → 打开厦大体育 |

登录门 / 授权见 `../page_map/`。Replay via `cargo test -p vision-core`。

候选框期望不写进这些 OCR JSON。颜色格、模板别名、检测去重的回归在 `vision-core` 的 `candidates` 单测里，用合成像素，不把截图入库。

