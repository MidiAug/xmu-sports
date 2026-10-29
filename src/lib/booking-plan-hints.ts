export type PlanHints = {
  plans: {
    paused: boolean;
    rules: { sport: string; account: string; enabled: boolean; weekdays: number[]; first_date: string }[];
    attempts: { sport: string; account: string; status: string; task: { target_date: string } }[];
  };
};

/** Match the actual exercise date and verified account, regardless of venue or time. */
export function hasBookingPlan(view: PlanHints | undefined, sport: string, date: string, account?: string): boolean {
  if (!account || !view || view.plans.paused) return false;
  const weekday = new Date(`${date}T12:00:00+08:00`).getUTCDay() || 7;
  const attempts = view.plans.attempts.filter(a => a.account === account && a.sport === sport && a.task.target_date === date);
  if (attempts.some(a => ["queued", "running", "succeeded", "existing", "unconfirmed"].includes(a.status))) return true;
  if (attempts.length) return false;
  return view.plans.rules.some(r => r.enabled && r.account === account && r.sport === sport && r.first_date <= date && r.weekdays.includes(weekday));
}
