import { describe, expect, it } from 'vitest';
import {
  emptyAnalytics,
  farmingTime,
  rewardHistory,
  summarizeAnalytics,
  type Analytics,
} from './analytics';

const history: Analytics = {
  ...emptyAnalytics(),
  campaigns: [
    {
      id: 'old',
      name: 'Old campaign',
      game: 'Rust',
      farmingMs: 3600000,
      completed: true,
    },
    {
      id: 'new',
      name: 'New campaign',
      game: 'Rust',
      farmingMs: 1800000,
      completed: false,
    },
    {
      id: 'other',
      name: 'Other campaign',
      game: 'Warframe',
      farmingMs: 60000,
      completed: false,
    },
  ],
  rewards: [
    {
      id: 'one',
      campaignId: 'old',
      campaign: 'Old campaign',
      game: 'Rust',
      name: 'Assault rifle',
      image: '',
      recordedAt: '2026-09-01T12:00:00Z',
    },
    {
      id: 'two',
      campaignId: 'new',
      campaign: 'New campaign',
      game: 'Rust',
      name: 'Sleeping bag',
      image: '',
      recordedAt: '2026-10-01T12:00:00Z',
    },
  ],
};

describe('analytics', () => {
  it('aggregates saved campaigns by game without counting unclaimed games as rewarded', () => {
    const stats = summarizeAnalytics(history);
    expect(stats.farmingMs).toBe(5460000);
    expect(stats.drops).toBe(2);
    expect(stats.completed).toBe(1);
    expect(stats.gamesRewarded).toBe(1);
    expect(stats.games[0]).toEqual({
      game: 'Rust',
      farmingMs: 5400000,
      drops: 2,
    });
    expect(summarizeAnalytics(emptyAnalytics()).games).toEqual([]);
  });
  it('searches reward and campaign names, combines game filters, and sorts without mutating history', () => {
    expect(rewardHistory(history.rewards, '', '').map((r) => r.id)).toEqual([
      'two',
      'one',
    ]);
    expect(
      rewardHistory(history.rewards, '  RUST rifle ', 'Rust').map((r) => r.id),
    ).toEqual(['one']);
    expect(rewardHistory(history.rewards, 'old campaign', '').length).toBe(1);
    expect(rewardHistory(history.rewards, 'rifle', 'Warframe')).toEqual([]);
    expect(history.rewards[0].id).toBe('one');
  });
  it('displays recorded time without rounding up unearned minutes', () => {
    expect(farmingTime(0)).toBe('0m');
    expect(farmingTime(59000)).toBe('<1m');
    expect(farmingTime(3599999)).toBe('59m');
    expect(farmingTime(3660000)).toBe('1h 1m');
  });
});
