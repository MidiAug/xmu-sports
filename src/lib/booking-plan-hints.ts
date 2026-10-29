export type PlanHints = {
  plans: {
    paused: boolean;
    rules: { id?: string; sport: string; account: string; enabled: boolean; weekdays: number[]; first_date: string }[];
    attempts: { sport: string; account: string; status: string; task: { target_date: string; plan_id?: string } }[];
  };
};

/** Match the actual exercise date and verified account, regardless of venue or time. */
export function hasBookingPlan(view: PlanHints | undefined, sport: string, date: string, account?: string): boolean {
  return Boolean(bookingPlanConflict(view, sport, date, account));
}

export function bookingPlanConflict(view: PlanHints | undefined, sport: string, date: string, account?: string): { ruleId?: string } | undefined {
  if (!account || !view || view.plans.paused) return;
  const weekday = new Date(`${date}T12:00:00+08:00`).getUTCDay() || 7;
  const attempts = view.plans.attempts.filter(a => a.account === account && a.sport === sport && a.task.target_date === date);
  const attempt = attempts.find(a => ["queued", "running", "succeeded", "existing", "unconfirmed"].includes(a.status));
  if (attempt) return { ruleId: attempt.task.plan_id };
  if (attempts.length) return;
  const rule = view.plans.rules.find(r => r.enabled && r.account === account && r.sport === sport && r.first_date <= date && r.weekdays.includes(weekday));
  if (rule) return { ruleId: rule.id };
}
