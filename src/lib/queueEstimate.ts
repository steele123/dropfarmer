import { watchReward, type Campaign, type Drop, type Snapshot } from './model';

export interface CampaignEstimate {
  minutes: number | null;
  lateRewards: string[];
  issue: string | null;
}

// Parallel rewards share watch time; prerequisite chains require successive time.
// Campaigns are projected in queue order, assuming streams and claims are available.
function project(c: Campaign, start: number) {
  const pending = c.drops.filter((d) => !d.claimed && watchReward(d));
  if (!c.drops.length) return null;
  if (!pending.length) return { minutes: 0, finish: start, lateRewards: [] };
  const campaignStart = Date.parse(c.startsAt);
  const campaignEnd = Date.parse(c.endsAt);
  if (![campaignStart, campaignEnd].every(Number.isFinite)) return null;
  const byId = new Map(c.drops.map((d) => [d.id, d]));
  const cache = new Map<string, { minutes: number; finish: number }>();
  const visiting = new Set<string>();
  const finishDrop = (d: Drop): { minutes: number; finish: number } | null => {
    if (d.claimed) return { minutes: 0, finish: start };
    if (cache.has(d.id)) return cache.get(d.id)!;
    if (
      !watchReward(d) ||
      visiting.has(d.id) ||
      !d.id ||
      ![d.required, d.minutes].every(Number.isFinite)
    )
      return null;
    const remaining = Math.max(0, d.required - Math.max(0, d.minutes));
    // Already earned rewards need claiming, not more time in an earning window.
    if (!remaining) return { minutes: 0, finish: start };
    const dropStart = Date.parse(d.startsAt);
    if (!Number.isFinite(dropStart) || !Number.isFinite(Date.parse(d.endsAt)))
      return null;
    visiting.add(d.id);
    let ready = Math.max(start, campaignStart, dropStart);
    let preceding = 0;
    for (const id of d.prerequisites) {
      const prerequisite = byId.get(id);
      const result = prerequisite ? finishDrop(prerequisite) : null;
      if (!result) return null;
      ready = Math.max(ready, result.finish);
      preceding = Math.max(preceding, result.minutes);
    }
    visiting.delete(d.id);
    const result = {
      minutes: preceding + remaining,
      finish: ready + remaining * 60_000,
    };
    cache.set(d.id, result);
    return result;
  };
  let minutes = 0;
  let finish = start;
  const lateRewards: string[] = [];
  for (const drop of pending) {
    const result = finishDrop(drop);
    if (!result) return null;
    minutes = Math.max(minutes, result.minutes);
    finish = Math.max(finish, result.finish);
    if (
      drop.minutes < drop.required &&
      result.finish > Math.min(campaignEnd, Date.parse(drop.endsAt))
    ) {
      lateRewards.push(drop.name);
    }
  }
  return { minutes, finish, lateRewards };
}

export function queueEstimate(s: Snapshot, now: number, preview = false) {
  const entries: Record<string, CampaignEstimate> = {};
  let minutes = 0;
  let partial = false;
  let cursor = now;
  let issue: string | null = s.needsReconnect
    ? 'Reconnect Twitch to estimate a finish time.'
    : !s.account && !preview
      ? 'Connect Twitch to estimate a finish time.'
      : s.campaignsCached
        ? 'Waiting for fresh campaign progress.'
        : s.error
          ? 'Waiting for Twitch to resume progress updates.'
          : null;
  const byId = new Map(s.campaigns.map((c) => [c.id, c]));
  for (const id of s.queue) {
    const c = byId.get(id);
    const result = c ? project(c, cursor) : null;
    if (!result) {
      entries[id] = {
        minutes: null,
        lateRewards: [],
        issue: 'Reward details unavailable.',
      };
      partial = true;
      issue ??= 'Refresh campaigns to load missing reward details.';
      continue;
    }
    minutes += result.minutes;
    cursor = result.finish;
    const status = s.queueStatuses[id]?.state;
    const rowIssue =
      !c!.linked && result.minutes > 0
        ? 'Link the game account first.'
        : ['offline', 'retrying', 'unavailable', 'reconnect'].includes(status)
          ? 'Waiting for an eligible stream or fresh progress.'
          : result.lateRewards.length
            ? 'Some rewards may miss their deadline.'
            : null;
    entries[id] = {
      minutes: Math.ceil(result.minutes),
      lateRewards: result.lateRewards,
      issue: rowIssue,
    };
    issue ??= rowIssue;
  }
  return {
    minutes: Math.ceil(minutes),
    partial,
    entries,
    finishAt: s.queue.length && minutes > 0 && !issue ? cursor : null,
    issue,
  };
}
