# Desktop booking redesign QA — 2026-10-30

## Current follow-up: colorful direct sport grid

The latest user correction rejects the sport dropdown and explicitly restores colorful direct choices without horizontal scrolling. It supersedes the dropdown decision in the historical follow-up below.

- Visual direction: original user-selected blue/white/mint mock and the latest direct-selection correction. Baseline capture: `artifacts/compact-booking/setup-light-1100-final.png`; revised browser capture: `artifacts/sport-grid/five-sports-light.png`. Both are 1100 × 760 pixels, CSS viewport 1100 × 760, DPR 1, with the same badminton/tomorrow/19:00–20:00 fixture state. Only favorite count differs; the baseline hides its options behind a dropdown.
- Combined evidence opened and compared: `artifacts/sport-grid/dropdown-vs-grid.png` (full view) and `artifacts/sport-grid/selection-detail.png` (focused selection area). Additional evidence: `artifacts/sport-grid/eight-sports-dark-minimum.png` at 960 × 600.
- Resolved P2: hidden sport choices no longer meet the user's desired visual/interaction direction. Restored colored library icons and directly clickable choices in wrapping rows; four favorites form two columns, other counts use approximately three per row with the last row filling the available width. No internal horizontal or vertical sport scroll area. Venue sits in a compact selector at the top right of the same section.
- Typography/layout: retained the compact 24px page title and existing confirmation hierarchy; sport names use 14px text with 22px icons, grouped on one line. Status badges may wrap separately. The extra height for visible choices is intentional; the main content scrolls at minimum height while the confirmation action stays reachable.
- Colors/assets: existing subdued control fills, blue selected state, locally generated brand mark and vendored MingCute icons are unchanged. Dark mode preserves navy/mint surfaces and distinct icon accents. No new decorative imagery or font dependencies.
- Copy/state: sport names and venue are actual catalog values, with all favorites directly visible. Existing planned/locked/disabled states remain enforced; selection updates the real booking summary. No duplicated hidden sport label is needed because the group already has its accessible name.
- Validation: `npm test`, production build and all 11 UI groups passed. Updated 3/4/5/8-favorite tests cover balanced rows, no internal overflow, icon/name alignment, disabled hover/click, keyboard selection, actual summary updates, selected-time visibility after resize and first-screen primary action. A previously hardcoded delay-test URL was corrected to honor the runner's isolated base URL after a connection-refused failure; the complete suite then passed. Final browser console inspection found no error entries.
- Native development launch: `npm run tauri:dev` successfully compiled and opened this checkout's `src-tauri/target/debug/xmu-sports.exe`; process path, native window and the loaded `http://127.0.0.1:1420/` document were verified. Native screenshot and accessibility state showed the new four-sport wrapping layout with the user's existing draft. No real booking, cancellation or explicit login action was clicked. Installer acceptance and exhaustive display-scale testing remain out of scope.
- Post-comparison result: no actionable P0/P1/P2 findings in the reviewed states. Desktop development process is left running for user inspection.

final result: passed

## Historical follow-up: compact booking layout

The user corrected the first implementation: the title and sport/venue section were too large, sport selection should not require horizontal scrolling, and the two fields should share a row. This correction supersedes the first implementation's large typography and sport rail described in the historical review below.

- Source evidence: attached `codex-clipboard-96779a52-1acc-48cf-971d-3397816f71cb.png`, 1174 × 756 pixels. Its application window was cropped from (57, 27) to (1157, 756), excluding other applications, leaving 1100 × 729 pixels. This is evidence of the reported problems, not a new pixel-copy target.
- Revised implementation: `artifacts/compact-booking/setup-light-1100-final.png`, 1100 × 760 pixels, CSS viewport 1100 × 760, DPR 1. Full-view comparison: `artifacts/compact-booking/before-after.png`, using a matching 1100 × 729 crop from the implementation; no scaling. Focused comparison: `artifacts/compact-booking/selectors-comparison.png`.
- State: single booking, light theme, tomorrow October 31, today 07:00 execution. User screenshot has ping-pong at 09:00; offline fixture has badminton at 19:00. These content differences are expected and excluded from fidelity findings.
- Additional final evidence: `setup-light-minimum-final.png` and `setup-dark-minimum-final.png` at 960 × 600; `sports-open-light-final.png` at 1100 × 760 with five sports. All are under `artifacts/compact-booking/`.

### Follow-up findings and fixes

1. **Resolved P2 — oversized heading and stacked selectors.** Changed the heading from 32px to 24px, tightened page/header spacing, and placed sport and venue controls in one row with aligned 44px controls. Replaced the horizontal sport rail with the existing keyboard-accessible select component, enriched with the same library icons. Reduced confirmation type, spacing and CTA height together to retain hierarchy. Full-view and focused comparisons show the date and time choices substantially higher on the page.
2. **Resolved P2 — long sport name separated from its icon.** The first five-sport menu capture showed 动感单车室 wrapping below its icon. Grouped the icon and name in one nonwrapping inline element, with badges allowed on a separate line; widened icon-bearing two-column compact menus to 320px. Post-fix evidence: `sports-open-light-final.png`; 3/4/5/8-sport regression checks assert no horizontal overflow and names/icons stay on one line.
3. **Resolved P2 — selected time disappeared when shrinking the viewport.** A resize from 1100 × 760 to 960 × 600 reduced the visible time rows without adjusting the local scroll position. The choice group now reveals its selected time when its size changes and disconnects that observer when the page is disposed. Post-fix evidence: both minimum-window final captures; regression selects the last time, shrinks the viewport and checks it remains within the time track.

### Current fidelity and validation

- **Typography:** native font stack retained; 24px page title, 14px field labels/body controls, 18px confirmation title and 28–36px time display. Small secondary controls are visually subordinate. Names/icons stay together and date numbers retain clear hierarchy.
- **Spacing/layout:** sport/venue occupy a 72px-tall labeled row; dates and times remain horizontal grids. Default height permits three time rows, compact height two. At 960 × 600 the primary action ends at approximately y=516 and the lower preferences remain reachable in the first screen. Persistent navigation is unobstructed.
- **Color/tokens:** existing blue/white/mint and dark navy/mint direction retained. No new competing decorative surfaces; locked and planned states retain their semantic badges and fixed disabled hover appearance.
- **Assets:** existing generated local brand mark and vendored MingCute SVGs retained, with 22px sport icons in the selector and menu. No rasterized controls or hand-drawn replacements.
- **Content/interactions:** selected sport and venue are always visible; menu exposes all favorites with keyboard navigation and Escape focus restoration. Time selection and business semantics are unchanged. Source screenshot's overflow is removed rather than hidden by clipping the choices.
- Frontend unit tests and production build passed. All 11 UI groups passed after the changes, including same-row geometry, long-name layout, selected-time resize visibility, first-screen primary action, disabled states and existing flows. No browser error-level console entries were observed in the final preview. No host code changed; this follow-up did not repeat Rust checks or perform installer/real-account operations.
- Remaining limits: user screenshot and fixture differ in sport/time and order count; no exact pixel-identity claim. Browser zoom/system scaling and real installed-native rendering were not exhaustively tested.

Current implementation checklist: same-row fields, compact title/spacing, complete sport popup, long names, minimum-window selection/CTA visibility, light/dark capture, keyboard/disabled states and regression checks are complete.

final result: passed

## Historical review: initial redesign

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
