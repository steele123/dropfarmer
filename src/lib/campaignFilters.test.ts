import { describe, expect, it } from 'vitest';
import {
  defaultCampaignFilters,
  filterCampaigns,
  campaignViewChanged,
} from './campaignFilters';
import { previewState, type Campaign } from './model';

const now = Date.parse('2026-10-01T12:00:00Z');
const hour = 60 * 60 * 1000;
function campaign(id: string, overrides: Partial<Campaign> = {}): Campaign {
  const c = previewState().campaigns[0];
  return {
    ...c,
    id,
    startsAt: new Date(now - hour).toISOString(),
    endsAt: new Date(now + hour).toISOString(),
    drops: c.drops.map((d) => ({ ...d, minutes: 0 })),
    ...overrides,
  };
}

describe('campaign filters', () => {
  it('combines game, search, linking, and queue filters', () => {
    const target = campaign('target', { channels: ['hJune'], linked: false });
    const data = [
      target,
      campaign('linked'),
      campaign('unqueued', { linked: false, channels: ['hJune'] }),
      campaign('other-game', { game: 'Warframe' }),
    ];
    const filters = {
      ...defaultCampaignFilters(),
      game: 'Rust',
      account: 'unlinked',
      queue: 'queued',
    } as const;
    expect(
      filterCampaigns(
        data,
        filters,
        'hjune hazmat',
        ['target', 'linked'],
        now,
      ).map((c) => c.id),
    ).toEqual(['target']);
    expect(
      filterCampaigns(
        data,
        { ...filters, queue: 'not-queued' },
        'hjune',
        ['target'],
        now,
      ).map((c) => c.id),
    ).toEqual(['unqueued']);
  });

  it('uses exact time boundaries and does not treat linking as campaign status', () => {
    const active = campaign('active', {
      linked: false,
      startsAt: new Date(now).toISOString(),
    });
    const upcoming = campaign('upcoming', {
      startsAt: new Date(now + hour).toISOString(),
      endsAt: new Date(now + 2 * hour).toISOString(),
    });
    const ended = campaign('ended', { endsAt: new Date(now).toISOString() });
    const invalid = campaign('invalid', {
      startsAt: 'invalid',
      endsAt: 'invalid',
    });
    const data = [active, upcoming, ended, invalid];
    for (const status of ['active', 'upcoming', 'ended'] as const) {
      expect(
        filterCampaigns(
          data,
          { ...defaultCampaignFilters(), status },
          '',
          [],
          now,
        ).map((c) => c.id),
      ).toEqual([status]);
    }
  });

  it('ending soon includes the next 72 hours and excludes expired or invalid dates', () => {
    const data = [
      campaign('boundary', { endsAt: new Date(now + 72 * hour).toISOString() }),
      campaign('later', {
        endsAt: new Date(now + 72 * hour + 1).toISOString(),
      }),
      campaign('ended', { endsAt: new Date(now).toISOString() }),
      campaign('invalid', { endsAt: '' }),
    ];
    expect(
      filterCampaigns(
        data,
        { ...defaultCampaignFilters(), endingSoon: true },
        '',
        [],
        now,
      ).map((c) => c.id),
    ).toEqual(['boundary']);
  });

  it('distinguishes unstarted, partial, fully watched, claimable, and claimed rewards', () => {
    const unstarted = campaign('unstarted');
    const partial = campaign('partial');
    partial.drops[0].claimed = true;
    const watched = campaign('watched');
    watched.drops.forEach((d) => {
      d.minutes = d.required;
    });
    const claimable = campaign('claimable');
    claimable.drops[0].claimId = 'claim-id';
    claimable.drops[0].minutes = 120;
    const claimed = campaign('claimed');
    claimed.drops.forEach((d) => {
      d.claimed = true;
      d.claimId = 'old-id';
    });
    const empty = campaign('empty', { drops: [] });
    const data = [unstarted, partial, watched, claimable, claimed, empty];
    const find = (
      progress: ReturnType<typeof defaultCampaignFilters>['progress'],
    ) =>
      filterCampaigns(
        data,
        { ...defaultCampaignFilters(), progress },
        '',
        [],
        now,
      ).map((c) => c.id);
    expect(find('not-started')).toEqual(['unstarted']);
    expect(find('in-progress')).toEqual(['partial', 'watched', 'claimable']);
    expect(find('claimable')).toEqual(['claimable']);
    expect(find('completed')).toEqual(['claimed']);
  });

  it('sorts without changing the source list or queue and puts invalid dates last', () => {
    const a = campaign('a', {
      game: 'Zebra',
      endsAt: new Date(now + 10 * hour).toISOString(),
    });
    const b = campaign('b', {
      game: 'Alpha',
      startsAt: new Date(now).toISOString(),
    });
    b.drops[0].minutes = 60;
    const invalid = campaign('invalid', { endsAt: 'invalid', startsAt: '' });
    const data = [invalid, a, b];
    const queue = ['a', 'b'];
    const find = (sort: ReturnType<typeof defaultCampaignFilters>['sort']) =>
      filterCampaigns(
        data,
        { ...defaultCampaignFilters(), sort },
        '',
        queue,
        now,
      ).map((c) => c.id);
    expect(find('ending')).toEqual(['b', 'a', 'invalid']);
    expect(find('starting')).toEqual(['b', 'a', 'invalid']);
    expect(find('game')).toEqual(['b', 'invalid', 'a']);
    expect(find('progress')[0]).toBe('b');
    expect(find('default')).toEqual(['invalid', 'a', 'b']);
    expect(data.map((c) => c.id)).toEqual(['invalid', 'a', 'b']);
    expect(queue).toEqual(['a', 'b']);
  });

  it('recognizes search, filters, and sorting when clearing the view', () => {
    expect(campaignViewChanged(defaultCampaignFilters(), '   ')).toBe(false);
    expect(campaignViewChanged(defaultCampaignFilters(), 'rust')).toBe(true);
    expect(
      campaignViewChanged({ ...defaultCampaignFilters(), sort: 'game' }, ''),
    ).toBe(true);
    expect(
      campaignViewChanged(
        { ...defaultCampaignFilters(), endingSoon: true },
        '',
      ),
    ).toBe(true);
  });
});
