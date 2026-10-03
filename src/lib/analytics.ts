export interface ReceivedDrop {
  id: string;
  campaignId: string;
  campaign: string;
  game: string;
  name: string;
  image: string;
  recordedAt: string;
  awardedAt?: string | null;
}

export interface Analytics {
  startedAt: string | null;
  campaigns: {
    id: string;
    name: string;
    game: string;
    farmingMs: number;
    completed: boolean;
  }[];
  rewards: ReceivedDrop[];
  error: string | null;
}

export const emptyAnalytics = (): Analytics => ({
  startedAt: null,
  campaigns: [],
  rewards: [],
  error: null,
});

export function summarizeAnalytics(data: Analytics) {
  const games = new Map<
    string,
    { game: string; farmingMs: number; drops: number }
  >();
  const entry = (name: string) => {
    const game = games.get(name) ?? { game: name, farmingMs: 0, drops: 0 };
    games.set(name, game);
    return game;
  };
  for (const campaign of data.campaigns)
    entry(campaign.game).farmingMs += campaign.farmingMs;
  for (const reward of data.rewards) entry(reward.game).drops++;
  return {
    drops: data.rewards.length,
    farmingMs: data.campaigns.reduce((total, c) => total + c.farmingMs, 0),
    completed: data.campaigns.filter((c) => c.completed).length,
    gamesRewarded: [...games.values()].filter((g) => g.drops > 0).length,
    games: [...games.values()].sort(
      (a, b) =>
        b.farmingMs - a.farmingMs ||
        b.drops - a.drops ||
        a.game.localeCompare(b.game),
    ),
  };
}

export function rewardHistory(
  rewards: ReceivedDrop[],
  query: string,
  game: string,
) {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/);
  return rewards
    .filter(
      (r) =>
        (!game || r.game === game) &&
        terms.every((term) =>
          `${r.name} ${r.game} ${r.campaign}`
            .toLocaleLowerCase()
            .includes(term),
        ),
    )
    .sort(
      (a, b) =>
        Date.parse(b.awardedAt ?? b.recordedAt) -
          Date.parse(a.awardedAt ?? a.recordedAt) ||
        a.name.localeCompare(b.name),
    );
}

export function farmingTime(ms: number) {
  if (ms > 0 && ms < 60_000) return '<1m';
  const minutes = Math.floor(ms / 60_000);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}h ${minutes % 60}m`
    : `${minutes}m`;
}
