import { describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import type { DownloadEvent, Update } from '@tauri-apps/plugin-updater';
import { createUpdater } from './updater';

function candidate() {
  return {
    version: '0.2.0',
    body: 'Fix campaign loading.',
    download: vi.fn(async (event?: (event: DownloadEvent) => void) => {
      event?.({ event: 'Started', data: { contentLength: 100 } });
      event?.({ event: 'Progress', data: { chunkLength: 40 } });
      event?.({ event: 'Progress', data: { chunkLength: 60 } });
      event?.({ event: 'Finished' });
    }),
    install: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  };
}

describe('app updates', () => {
  it('does not treat an unavailable release server as up to date', async () => {
    const updater = createUpdater({
      check: vi.fn().mockRejectedValue(new Error('404')),
      pause: vi.fn(),
    });
    await updater.check();
    expect(get(updater).phase).toBe('error');
    expect(get(updater).error).toContain('published release');
  });

  it('handles a successful check with no newer version', async () => {
    const updater = createUpdater({ check: async () => null, pause: vi.fn() });
    await updater.check();
    expect(get(updater).phase).toBe('current');
  });

  it('verifies the download before pausing and installing, and tracks progress', async () => {
    const update = candidate();
    const pause = vi.fn(async () => {});
    const updater = createUpdater({
      check: async () => update as unknown as Update,
      pause,
    });
    await updater.check();
    expect(get(updater)).toMatchObject({
      phase: 'available',
      version: '0.2.0',
    });
    await updater.install();
    expect(get(updater)).toMatchObject({
      phase: 'installing',
      downloaded: 100,
      total: 100,
    });
    expect(update.download.mock.invocationCallOrder[0]).toBeLessThan(
      pause.mock.invocationCallOrder[0],
    );
    expect(pause.mock.invocationCallOrder[0]).toBeLessThan(
      update.install.mock.invocationCallOrder[0],
    );
    expect(update.close).toHaveBeenCalledOnce();
  });

  it('never pauses or installs when signature verification or download fails', async () => {
    const update = candidate();
    update.download.mockRejectedValueOnce(new Error('Invalid signature'));
    const pause = vi.fn();
    const updater = createUpdater({
      check: async () => update as unknown as Update,
      pause,
    });
    await updater.check();
    await updater.install();
    expect(pause).not.toHaveBeenCalled();
    expect(update.install).not.toHaveBeenCalled();
    expect(get(updater).phase).toBe('error');
    expect(update.close).toHaveBeenCalledOnce();
  });

  it('does not launch the installer when pausing fails', async () => {
    const update = candidate();
    const updater = createUpdater({
      check: async () => update as unknown as Update,
      pause: vi.fn().mockRejectedValue(new Error('Cannot save queue')),
    });
    await updater.check();
    await updater.install();
    expect(update.install).not.toHaveBeenCalled();
    expect(get(updater).error).toContain('Could not pause');
  });

  it('closes a late check result after disposal and prevents duplicate checks', async () => {
    const update = candidate();
    let resolve!: (update: Update) => void;
    const check = vi.fn(
      () =>
        new Promise<Update>((r) => {
          resolve = r;
        }),
    );
    const updater = createUpdater({ check, pause: vi.fn() });
    const pending = updater.check();
    await Promise.resolve();
    await updater.check();
    updater.dispose();
    resolve(update as unknown as Update);
    await pending;
    expect(check).toHaveBeenCalledOnce();
    expect(update.close).toHaveBeenCalledOnce();
    expect(update.install).not.toHaveBeenCalled();
  });
});
