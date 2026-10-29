# Desktop booking redesign QA — 2026-10-30

final result: passed

## Evidence and state

- Source visual truth: user-selected `codex-clipboard-7831a7f0-c603-4edb-ad06-6fb00abd7d0b.png` attachment (1500 × 1049 pixels).
- Implementation: `artifacts/visual-refactor/setup-light-final.png` (1200 × 840 pixels; CSS viewport 1200 × 840, DPR 1).
- Full-view comparison: `artifacts/visual-refactor/source-vs-implementation.png`. Source downsampled to 1200 × 840; implementation unscaled. The subpixel aspect-ratio difference is negligible.
- Focused comparisons: `artifacts/visual-refactor/controls-comparison.png` and `artifacts/visual-refactor/summary-comparison.png`.
- State: light theme, single booking, badminton, tomorrow October 31, scheduled execution today at 07:00. Isolated Tauri fixture with fixed local date; no real booking operations.
- Content differences: fixture/catalog supplies 19:00–20:00 rather than the illustration's 19:30–21:00; sport order follows saved catalog order. All twelve real catalog slots remain available in a scrollable two-row area. These are deliberate functional adaptations.
- Additional captures: `setup-dark-minimum-final.png`, `setup-light-minimum-final.png` at 960 × 600, plus `orders-light.png`, `plans-light.png`, and `settings-light.png` in the same artifact directory. At minimum height the content area scrolls; top navigation remains visible.
- Preview: http://127.0.0.1:1428/__design-preview (temporary local development server).

## Findings and comparison history

1. **Resolved P2 — confirmation hierarchy too small.** Earlier full-view and summary comparisons showed the time, date, execution row and primary action lacked the source's emphasis. Increased display typography and the primary action to 60px height / 20px text; enlarged supporting fields and sport labels. Post-fix evidence: final full-view and summary comparisons above.
2. **Resolved P1 — dates lost their horizontal layout during stylesheet cleanup.** A subsequent implementation capture showed vertically stacked dates. Restored the complete three-column date grid and explicit button appearance, spacing and selected state in `booking.css`; added a browser geometry assertion that all three date buttons share one row. Post-fix evidence: `setup-light-final.png`, final control comparison, and the final shell UI regression pass.
3. **Resolved P2 — popup closed while refreshing.** Repainting the order panel removed the click target before the document outside-click handler ran. Use the original event's composed path to recognize internal clicks. Popup refresh, dismissal and Escape focus now pass the existing order and shell regressions.

No actionable P0/P1/P2 findings remain in the reviewed states.

## Fidelity surfaces

- **Fonts and typography:** retained the native Segoe UI / Microsoft YaHei fallback stack. The source's display/body hierarchy is represented by a 32px page heading, large unwrapped time, medium section labels and smaller secondary controls. Exact raster glyph weights differ from the generated reference; no external font dependency was added. Date labels and selected states remain readable at the minimum desktop width.
- **Spacing and layout:** horizontal persistent header, approximately 1.65:1 editor/confirmation columns, rounded white editor and mint confirmation surface. Section dividers, date alignment and selected time chips were inspected in combined full and focused comparisons. Long catalogs scroll within controls; advanced settings expand within the editor. At 960 × 600 the page scrolls rather than introducing a portrait layout.
- **Colors and tokens:** blue primary/selected states, cool white canvas and restrained mint confirmation match the reference direction. Dark mode uses navy surfaces with a dark mint confirmation panel. Semantic locked/full/partial badges remain distinct. Surfaces and disabled hover behavior are covered by UI regression.
- **Image quality and assets:** locally generated transparent blue brand mark retains the reference's subject and direction, displayed with preserved aspect ratio. Sport/calendar/location symbols use vendored MingCute library SVGs with Apache-2.0 attribution, not hand-drawn replacements. No decorative placeholder imagery or remote runtime assets.
- **Copy and content:** visible primary navigation is 预约 / 长期计划 / 记录; 我的预约, 运行/进度 and 设置 have separate top-level locations. 场馆 and 场地偏好 distinguish venue from court. Secondary execution controls and validation messages continue to reflect real task state. The developer badge is development-only; the normal version is hidden at compact widths and remains accessible in settings/about.

## Functional validation

- `npm test`: passed.
- `npm run build`: passed after the final date-layout fix.
- `npm run test:ui`: all 11 groups passed after the final fix, using the isolated fixture server.
- `cargo check --manifest-path src-tauri/Cargo.toml -p xmu-sports --lib`: passed with two existing dead-code warnings.
- Covered interactions include direct sport/time selection, disabled and availability states, keyboard traversal, date alignment, navigation, saved plans, pending results, midnight dates, order cache/cancellation fixtures, update/about, settings, selectors, and light/dark/minimum desktop layouts.
- Existing native debug window and its loaded frontend were identified through process path and accessibility state. A native screenshot was obscured and rejected, so native pixel verification is not claimed. New 960 × 600 native minimum-size configuration requires a rebuilt/restarted host.
- Installer, real authentication recovery, real cancellation and real booking submission were not executed.

## Follow-up polish and limits

- P3: source illustration uses slightly different icon silhouettes and soft shading. Standard library icons and token surfaces provide a consistent implementation; exact illustration pixels are not the acceptance claim.
- Source does not specify every loading/error/locked screen. Existing behavior and test fixtures govern those states.
- Browser zoom and arbitrary operating-system display scaling were not exhaustively tested.

## Implementation checklist

- [x] Commit checkpoint before refactor.
- [x] Implement selected composition within the existing frontend.
- [x] Preserve execution, availability, authentication and cancellation contracts.
- [x] Compare source and implementation together; fix and recapture substantive findings.
- [x] Verify light/dark, minimum desktop, keyboard and popup behavior.
- [x] Run frontend logic/build/UI checks and Rust host check.
- [x] Keep a local isolated preview and visual evidence for review.
