import { emptyAnalytics, type Analytics } from './analytics';

export interface Drop {
  id: string;
  name: string;
  image: string;
  required: number;
  requiredSubs?: number;
  minutes: number;
  claimed: boolean;
  claimId: string | null;
  prerequisites: string[];
  startsAt: string;
  endsAt: string;
}
export interface Campaign {
  id: string;
  name: string;
  game: string;
  gameId: string;
  image: string;
  linked: boolean;
  startsAt: string;
  endsAt: string;
  channels: string[];
  drops: Drop[];
}
export interface Snapshot {
  analytics: Analytics;
  account: { id: string; login: string } | null;
  loginMethod: 'browser' | 'code' | null;
  campaigns: Campaign[];
  campaignNotice: string | null;
  campaignsCached: boolean;
  queue: string[];
  queueStatuses: Record<string, QueueStatus>;
  trayEnabled: boolean;
  notifications: NotificationSettings;
  needsReconnect: boolean;
  sleepAfterQueue: {
    enabled: boolean;
    secondsRemaining: number | null;
    supported: boolean;
  };
  running: boolean;
  status: string;
  activeCampaign: string | null;
  channel: { id: string; login: string; name: string } | null;
  logs: { time: string; message: string; level: string }[];
  lastSync: string | null;
  autoClaim: boolean;
  error: string | null;
}
export interface QueueStatus {
  state: string;
  message: string;
  checkedAt: string | null;
  retryAt: string | null;
}
export interface NotificationSettings {
  rewards: boolean;
  queue: boolean;
  reconnect: boolean;
}
export interface LoginCode {
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}
export const initialState = (): Snapshot => ({
  analytics: emptyAnalytics(),
  account: null,
  loginMethod: null,
  campaigns: [],
  campaignNotice: null,
  campaignsCached: false,
  queue: [],
  queueStatuses: {},
  trayEnabled: true,
  notifications: { rewards: false, queue: false, reconnect: false },
  needsReconnect: false,
  sleepAfterQueue: { enabled: false, secondsRemaining: null, supported: false },
  running: false,
  status: 'Twitch not connected',
  activeCampaign: null,
  channel: null,
  logs: [],
  lastSync: null,
  autoClaim: true,
  error: null,
});
export const complete = (c: Campaign) =>
  c.drops.length > 0 && c.drops.every((d) => d.claimed);
export function matchesCampaign(c: Campaign, query: string): boolean {
  const text = [c.game, c.name, ...c.channels, ...c.drops.map((d) => d.name)]
    .join(' ')
    .toLowerCase();
  return query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .every((term) => text.includes(term));
}
export const watchReward = (d: Drop) =>
  d.required > 0 && !(d.requiredSubs ?? 0);
export const hasWatchRewards = (c: Campaign) =>
  c.drops.some((d) => !d.claimed && watchReward(d));
export const watchRestriction = (c: Campaign) =>
  complete(c) || hasWatchRewards(c)
    ? ''
    : c.drops.some((d) => !d.claimed && (d.requiredSubs ?? 0) > 0)
      ? 'Subscription required'
      : 'No watch-time rewards';
export const canQueue = (c: Campaign) =>
  hasWatchRewards(c) && new Date(c.endsAt).getTime() > Date.now();
export function progress(c: Campaign) {
  const total = c.drops.reduce((n, d) => n + d.required, 0);
  return total
    ? Math.min(
        100,
        Math.round(
          (c.drops.reduce(
            (n, d) => n + (d.claimed ? d.required : d.minutes),
            0,
          ) /
            total) *
            100,
        ),
      )
    : 0;
}
export function moveQueue(
  queue: string[],
  id: string,
  direction: number,
): string[] {
  const next = [...queue];
  const i = next.indexOf(id);
  const j = i + direction;
  if (i < 0 || j < 0 || j >= next.length) return next;
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}
export function moveQueueToTop(queue: string[], id: string): string[] {
  return queue.includes(id)
    ? [id, ...queue.filter((entry) => entry !== id)]
    : [...queue];
}
export function deadline(iso: string) {
  const hours = Math.ceil((new Date(iso).getTime() - Date.now()) / 3600000);
  return hours <= 0
    ? 'Ended'
    : hours < 24
      ? `${hours}h left`
      : `${Math.ceil(hours / 24)}d left`;
}
export function campaignState(c: Campaign) {
  return complete(c)
    ? 'Completed'
    : new Date(c.endsAt).getTime() <= Date.now()
      ? 'Ended'
      : watchRestriction(c)
        ? watchRestriction(c)
        : new Date(c.startsAt).getTime() > Date.now()
          ? 'Upcoming'
          : !c.linked
            ? 'Link required'
            : 'Available';
}
export const duration = (m: number) =>
  m >= 60 ? `${Math.floor(m / 60)}h${m % 60 ? ` ${m % 60}m` : ''}` : `${m}m`;
export function previewState(): Snapshot {
  const start = new Date(Date.now() - 86400000).toISOString();
  const examples = [
    [
      'Rust',
      'Frontier collection',
      'Hazmat supply crate',
      'Explorer sleeping bag',
      72,
      2,
    ],
    [
      'Sea of Thieves',
      'Treasures of the tide',
      'Obsidian compass',
      'Midnight sails',
      20,
      5,
    ],
    [
      'Warframe',
      'Across the Origin System',
      'Galactic glyph',
      'Tenno supply cache',
      0,
      3,
    ],
    [
      'World of Warcraft',
      'A new adventure',
      'Traveler’s companion',
      'Enchanted satchel',
      0,
      7,
    ],
    [
      'Overwatch 2',
      'Heroes assemble',
      'Victory spray',
      'Champion player icon',
      0,
      4,
    ],
    [
      'No Man’s Sky',
      'Beyond the horizon',
      'Explorer starship',
      'Atlas companion',
      0,
      6,
    ],
  ] as const;
  const campaigns: Campaign[] = examples.map(
    ([game, name, a, b, minutes, days], i) => {
      const endsAt = new Date(
        Date.now() + (i === 0 ? 90 * 60000 : days * 86400000),
      ).toISOString();
      return {
        id: `sample-${i}`,
        name,
        game,
        gameId: String(i),
        image: '',
        linked: i !== 4,
        startsAt: start,
        endsAt,
        channels: [],
        drops: [a, b].map((name, j) => ({
          id: `${i}-${j}`,
          name,
          image: '',
          required: 120,
          minutes: j === 0 ? minutes : 0,
          claimed: false,
          claimId: null,
          prerequisites: [],
          startsAt: start,
          endsAt,
        })),
      };
    },
  );
  return {
    ...initialState(),
    analytics: {
      startedAt: new Date(Date.now() - 7 * 86400000).toISOString(),
      error: null,
      campaigns: campaigns.slice(0, 3).map((c, i) => ({
        id: `history-${c.id}`,
        name: c.name,
        game: c.game,
        farmingMs: (240 - i * 60) * 60000,
        completed: true,
      })),
      rewards: campaigns.slice(0, 3).flatMap((c, i) =>
        c.drops.map((d, j) => ({
          id: d.id,
          campaignId: `history-${c.id}`,
          campaign: c.name,
          game: c.game,
          name: d.name,
          image: d.image,
          recordedAt: new Date(
            Date.now() - (i + 1) * 86400000 + j * 3600000,
          ).toISOString(),
        })),
      ),
    },
    campaigns,
    queue: ['sample-0', 'sample-1'],
    status: 'Preview · sample campaigns',
    logs: [
      {
        time: new Date().toISOString(),
        level: 'info',
        message:
          'Preview loaded. These are sample campaigns; no Twitch activity is sent.',
      },
    ],
  };
}
