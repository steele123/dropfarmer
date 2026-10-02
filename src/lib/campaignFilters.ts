import { complete, matchesCampaign, progress, type Campaign } from './model';

export interface CampaignFilters {
  game: string;
  status: 'all' | 'active' | 'upcoming' | 'ended';
  progress: 'all' | 'not-started' | 'in-progress' | 'claimable' | 'completed';
  account: 'all' | 'linked' | 'unlinked';
  queue: 'all' | 'queued' | 'not-queued';
  endingSoon: boolean;
  sort: 'default' | 'ending' | 'starting' | 'game' | 'progress';
}

export const defaultCampaignFilters = (): CampaignFilters => ({
  game: '',
  status: 'all',
  progress: 'all',
  account: 'all',
  queue: 'all',
  endingSoon: false,
  sort: 'default',
});

export function campaignViewChanged(filters: CampaignFilters, search: string) {
  const defaults = defaultCampaignFilters();
  return (
    !!search.trim() ||
    (Object.keys(defaults) as (keyof CampaignFilters)[]).some(
      (key) => filters[key] !== defaults[key],
    )
  );
}

export function filterCampaigns(
  campaigns: Campaign[],
  filters: CampaignFilters,
  search: string,
  queue: string[],
  now = Date.now(),
): Campaign[] {
  const queued = new Set(queue);
  const filtered = campaigns.filter((c) => {
    if (
      !matchesCampaign(c, search) ||
      (filters.game && c.game !== filters.game)
    )
      return false;
    const start = Date.parse(c.startsAt);
    const end = Date.parse(c.endsAt);
    if (filters.status === 'active' && !(start <= now && now < end))
      return false;
    if (filters.status === 'upcoming' && !(start > now && end > start))
      return false;
    if (filters.status === 'ended' && !(end <= now)) return false;
    if (filters.endingSoon && !(end > now && end <= now + 72 * 60 * 60 * 1000))
      return false;
    if (filters.account === 'linked' && !c.linked) return false;
    if (filters.account === 'unlinked' && c.linked) return false;
    if (filters.queue === 'queued' && !queued.has(c.id)) return false;
    if (filters.queue === 'not-queued' && queued.has(c.id)) return false;
    const started = c.drops.some((d) => d.claimed || d.minutes > 0);
    if (filters.progress === 'not-started' && (!c.drops.length || started))
      return false;
    if (filters.progress === 'in-progress' && (!started || complete(c)))
      return false;
    if (filters.progress === 'completed' && !complete(c)) return false;
    if (
      filters.progress === 'claimable' &&
      !c.drops.some((d) => !d.claimed && !!d.claimId)
    )
      return false;
    return true;
  });
  // Missing dates sort last; filtering and sorting never change Twitch's data or the queue.
  const date = (value: string, descending = false) => {
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed * (descending ? -1 : 1) : Infinity;
  };
  switch (filters.sort) {
    case 'ending':
      return filtered.sort((a, b) => date(a.endsAt) - date(b.endsAt));
    case 'starting':
      return filtered.sort(
        (a, b) => date(a.startsAt, true) - date(b.startsAt, true),
      );
    case 'game':
      return filtered.sort(
        (a, b) => a.game.localeCompare(b.game) || a.name.localeCompare(b.name),
      );
    case 'progress':
      return filtered.sort((a, b) => progress(b) - progress(a));
    default:
      return filtered;
  }
}
