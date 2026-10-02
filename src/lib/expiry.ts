import { duration, type Campaign, type Drop } from './model';

export interface ExpiryWarning {
  dropId: string;
  level: 'ended' | 'insufficient' | 'soon';
  message: string;
  detail: string;
  deadline: number;
}

// Use a lower bound: independent rewards may progress together; prerequisites cannot.
// Queue delays, unavailable channels and claim delays can only make completion later.
export function expiryWarnings(c: Campaign, now: number): ExpiryWarning[] {
  const campaignEnd = Date.parse(c.endsAt);
  const campaignStart = Date.parse(c.startsAt);
  if (!Number.isFinite(campaignEnd) || !Number.isFinite(campaignStart))
    return [];
  const byId = new Map(c.drops.map((d) => [d.id, d]));
  const earliestFinish = (
    d: Drop,
    visiting = new Set<string>(),
  ): number | null => {
    if (d.claimed) return now;
    if (visiting.has(d.id)) return null;
    const start = Date.parse(d.startsAt);
    if (!Number.isFinite(start)) return null;
    const next = new Set(visiting).add(d.id);
    let ready = Math.max(now, campaignStart, start);
    for (const id of d.prerequisites) {
      const prerequisite = byId.get(id);
      const finish = prerequisite ? earliestFinish(prerequisite, next) : null;
      if (finish === null) return null;
      ready = Math.max(ready, finish);
    }
    return ready + Math.max(0, d.required - d.minutes) * 60_000;
  };
  const warnings: ExpiryWarning[] = [];
  for (const d of c.drops) {
    if (d.claimed) continue;
    const end = Math.min(campaignEnd, Date.parse(d.endsAt));
    if (!Number.isFinite(end)) continue;
    const remaining = end - now;
    const ready = d.required > 0 && d.minutes >= d.required;
    const finish = earliestFinish(d);
    const base = { dropId: d.id, deadline: end };
    if (remaining <= 0) {
      warnings.push({
        ...base,
        level: 'ended',
        message: `${d.name}: earning window ended`,
        detail: ready
          ? 'Watch time is complete. Check Inventory for claim availability.'
          : 'No watch time remains for this reward.',
      });
    } else if (!ready && finish !== null && finish > end) {
      warnings.push({
        ...base,
        level: 'insufficient',
        message: `Not enough time for ${d.name}`,
        detail: `At least ${duration(Math.ceil((finish - now) / 60_000))} until completion, including prerequisites and scheduled starts; ${duration(Math.ceil(remaining / 60_000))} before the deadline. Queue and offline delays are extra.`,
      });
    } else if (remaining <= 24 * 3_600_000) {
      warnings.push({
        ...base,
        level: 'soon',
        message: `${d.name}: ${duration(Math.ceil(remaining / 60_000))} left`,
        detail: ready
          ? 'Watch time is complete. Check Inventory to claim this reward.'
          : 'Reward window ends within 24 hours. Queue and offline delays may put it at risk.',
      });
    }
  }
  const priority = { insufficient: 0, soon: 1, ended: 2 };
  return warnings.sort(
    (a, b) => priority[a.level] - priority[b.level] || a.deadline - b.deadline,
  );
}
