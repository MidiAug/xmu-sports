import { renderSetup } from "./setup";
import { renderBookingPlans } from "./booking-plans";
import { mountPage, disposePage } from "../lib/page-lifecycle";

let mode = "single";
try { mode = localStorage.getItem("xmu.booking.mode") ?? "single"; } catch { /* Use the single booking form. */ }

export async function renderBooking(panel: Element) {
  const scope = mountPage(panel);
  panel.innerHTML = `<div class="booking-modes" role="tablist" aria-label="预约方式">
    <button type="button" role="tab" data-mode="single">单次预约</button>
    <button type="button" role="tab" data-mode="plans">长期计划</button>
  </div><div class="booking-content" role="tabpanel"></div>`;
  const content = panel.querySelector(".booking-content")!;
  scope.onDispose(() => disposePage(content));
  const paint = async () => {
    for (const button of panel.querySelectorAll<HTMLButtonElement>("[data-mode]")) {
      button.setAttribute("aria-selected", String(button.dataset.mode === mode));
      button.tabIndex = button.dataset.mode === mode ? 0 : -1;
    }
    if (mode === "plans") await renderBookingPlans(content);
    else await renderSetup(content);
  };
  const choose = (next: string) => {
    mode = next;
    try { localStorage.setItem("xmu.booking.mode", mode); } catch { /* Keep this session's selection. */ }
    void paint();
  };
  panel.querySelectorAll<HTMLButtonElement>("[data-mode]").forEach(button => {
    button.onclick = () => choose(button.dataset.mode!);
    button.onkeydown = event => {
      if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        choose(event.key === "Home" ? "single" : event.key === "End" ? "plans" : mode === "plans" ? "single" : "plans");
        panel.querySelector<HTMLButtonElement>(`[data-mode="${mode}"]`)!.focus();
      }
    };
  });
  await paint();
}
