import { describe, expect, it } from 'vitest';
import { queueEstimate } from './queueEstimate';
import {
  initialState,
  previewState,
  type Campaign,
  type Snapshot,
} from './model';

const now = Date.parse('2026-10-04T12:00:00Z');
const at = (minutes: number) => new Date(now + minutes * 60_000).toISOString();
function campaign(id = 'a'): Campaign {
  const c = previewState().campaigns[0];
  return {
    ...c,
    id,
    linked: true,
    startsAt: at(-100),
    endsAt: at(1000),
    drops: c.drops.map((d, i) => ({
      ...d,
      id: `${id}-${i}`,
      name: `${id} reward ${i}`,
      required: 120,
      minutes: 0,
      startsAt: at(-100),
      endsAt: at(1000),
    })),
  };
}
function state(...campaigns: Campaign[]): Snapshot {
  return {
    ...initialState(),
    account: { id: 'viewer', login: 'viewer' },
    campaigns,
    queue: campaigns.map((c) => c.id),
  };
}

describe('queue finish estimates', () => {
  it('shares parallel watch time and accounts for partial and claimed rewards', () => {
    const c = campaign();
    c.drops[0].minutes = 90;
    c.drops[1].minutes = 30;
    expect(queueEstimate(state(c), now).minutes).toBe(90);
    c.drops[1].claimed = true;
    expect(queueEstimate(state(c), now).finishAt).toBe(now + 30 * 60_000);
  });
  it('adds prerequisite chains without counting shared prerequisites twice', () => {
    const c = campaign();
    c.drops[0].minutes = 60;
    c.drops[1].prerequisites = [c.drops[0].id];
    c.drops.push({ ...c.drops[1], id: 'branch', required: 90 });
    expect(queueEstimate(state(c), now).minutes).toBe(180);
    c.drops[0].claimed = true;
    expect(queueEstimate(state(c), now).minutes).toBe(120);
  });
  it('includes future starts in the finish time without treating waiting as watch time', () => {
    const c = campaign();
    c.startsAt = at(60);
    c.drops[1].startsAt = at(180);
    const result = queueEstimate(state(c), now);
    expect(result.minutes).toBe(120);
    expect(result.finishAt).toBe(now + 300 * 60_000);
  });
  it('detects deadline risks caused by queue order and clears them after reordering', () => {
    const first = campaign('first');
    const urgent = campaign('urgent');
    urgent.drops[1].endsAt = at(180);
    const s = state(first, urgent);
    const result = queueEstimate(s, now);
    expect(result.minutes).toBe(240);
    expect(result.entries.urgent.lateRewards).toEqual(['urgent reward 1']);
    expect(result.finishAt).toBeNull();
    s.queue.reverse();
    expect(queueEstimate(s, now).entries.urgent.lateRewards).toEqual([]);
    expect(queueEstimate(s, now).finishAt).toBe(now + 240 * 60_000);
  });
  it('uses campaign deadlines and does not treat earned drops as missed', () => {
    const c = campaign();
    c.endsAt = at(-1);
    expect(queueEstimate(state(c), now).entries.a.lateRewards).toHaveLength(2);
    c.drops.forEach((d) => (d.minutes = d.required));
    const result = queueEstimate(state(c), now);
    expect(result.minutes).toBe(0);
    expect(result.entries.a.lateRewards).toEqual([]);
    expect(result.issue).toBeNull();
    expect(result.finishAt).toBeNull();
  });
  it('excludes subscription rewards but requires paid prerequisites to be claimed', () => {
    const c = campaign();
    c.drops[1].requiredSubs = 1;
    c.drops[1].required = 200;
    expect(queueEstimate(state(c), now).minutes).toBe(120);
    c.drops[0].prerequisites = [c.drops[1].id];
    expect(queueEstimate(state(c), now).partial).toBe(true);
    c.drops[1].claimed = true;
    expect(queueEstimate(state(c), now).partial).toBe(false);
  });
  it('withholds finish times for incomplete data, missing prerequisites and cycles', () => {
    for (const problem of ['missing', 'cycle', 'date', 'empty', 'progress']) {
      const c = campaign();
      if (problem === 'missing') c.drops[0].prerequisites = ['missing'];
      if (problem === 'cycle') {
        c.drops[0].prerequisites = [c.drops[1].id];
        c.drops[1].prerequisites = [c.drops[0].id];
      }
      if (problem === 'date') c.drops[0].startsAt = 'invalid';
      if (problem === 'empty') c.drops = [];
      if (problem === 'progress') c.drops[0].minutes = NaN;
      const result = queueEstimate(state(c, campaign('known')), now);
      expect(result.partial).toBe(true);
      expect(result.minutes).toBe(120);
      expect(result.finishAt).toBeNull();
    }
    const s = state(campaign());
    s.queue.unshift('missing');
    expect(queueEstimate(s, now).partial).toBe(true);
  });
  it('shows remaining time but no clock estimate while offline, unlinked or disconnected', () => {
    const s = state(campaign());
    for (const status of ['offline', 'retrying', 'unavailable', 'reconnect']) {
      s.queueStatuses.a = {
        state: status,
        message: '',
        checkedAt: null,
        retryAt: null,
      };
      expect(queueEstimate(s, now).minutes).toBe(120);
      expect(queueEstimate(s, now).finishAt).toBeNull();
    }
    s.queueStatuses = {};
    s.needsReconnect = true;
    expect(queueEstimate(s, now).finishAt).toBeNull();
    s.needsReconnect = false;
    s.campaignsCached = true;
    expect(queueEstimate(s, now).finishAt).toBeNull();
    s.campaignsCached = false;
    s.campaigns[0].linked = false;
    expect(queueEstimate(s, now).finishAt).toBeNull();
  });
  it('recalculates a paused start-now estimate and handles an empty queue', () => {
    const s = state(campaign());
    expect(queueEstimate(s, now + 60_000).finishAt).toBe(now + 121 * 60_000);
    s.queue = [];
    expect(queueEstimate(s, now).minutes).toBe(0);
    expect(queueEstimate(s, now).finishAt).toBeNull();
  });
});
