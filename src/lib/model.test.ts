import { describe, expect, it } from 'vitest';
import {
  moveQueue,
  moveQueueToTop,
  previewState,
  complete,
  progress,
  campaignState,
  canQueue,
  matchesCampaign,
} from './model';
describe('campaign search', () => {
  it('finds reward and streamer names without requiring them in the campaign title', () => {
    const c = previewState().campaigns[0];
    c.name = 'Rust Isles AR';
    c.channels = ['hjune', 'hutnik'];
    c.drops[0].name = 'Assault Rifle';
    expect(matchesCampaign(c, ' HJune ')).toBe(true);
    expect(matchesCampaign(c, 'assault rifle')).toBe(true);
    expect(matchesCampaign(c, 'rust hutnik')).toBe(true);
    expect(matchesCampaign(c, 'rust islands')).toBe(false);
    expect(matchesCampaign(c, '  ')).toBe(true);
  });
});
describe('queue ordering', () => {
  it('moves a campaign straight to the top and preserves every other position', () => {
    const queue = ['a', 'b', 'c', 'd'];
    expect(moveQueueToTop(queue, 'd')).toEqual(['d', 'a', 'b', 'c']);
    expect(moveQueueToTop(queue, 'a')).toEqual(queue);
    expect(moveQueueToTop(queue, 'missing')).toEqual(queue);
    expect(moveQueueToTop([], 'missing')).toEqual([]);
    expect(queue).toEqual(['a', 'b', 'c', 'd']);
  });
  it('allows unlinked campaigns to wait in the queue but not expired or completed campaigns', () => {
    const c = previewState().campaigns[4];
    expect(c.linked).toBe(false);
    expect(canQueue(c)).toBe(true);
    c.drops.forEach((d) => (d.claimed = true));
    expect(canQueue(c)).toBe(false);
    c.drops.forEach((d) => (d.claimed = false));
    c.endsAt = '2000-01-01T00:00:00Z';
    expect(canQueue(c)).toBe(false);
  });
  it('moves a campaign one position without mutating the saved order', () => {
    const queue = ['a', 'b', 'c'];
    expect(moveQueue(queue, 'b', -1)).toEqual(['b', 'a', 'c']);
    expect(queue).toEqual(['a', 'b', 'c']);
  });
  it('preserves order at boundaries and for missing IDs', () => {
    expect(moveQueue(['a', 'b'], 'a', -1)).toEqual(['a', 'b']);
    expect(moveQueue(['a', 'b'], 'b', 1)).toEqual(['a', 'b']);
    expect(moveQueue(['a', 'b'], 'x', -1)).toEqual(['a', 'b']);
  });
});
describe('confirmed campaign state', () => {
  it('does not call a fully watched but unclaimed campaign complete', () => {
    const c = previewState().campaigns[0];
    c.drops.forEach((d) => (d.minutes = d.required));
    expect(progress(c)).toBe(100);
    expect(complete(c)).toBe(false);
    c.drops.forEach((d) => (d.claimed = true));
    expect(complete(c)).toBe(true);
  });
  it('prioritizes expiration over account linking', () => {
    const c = previewState().campaigns[4];
    expect(campaignState(c)).toBe('Link required');
    c.endsAt = '2000-01-01T00:00:00Z';
    expect(campaignState(c)).toBe('Ended');
  });
});
