import { writable } from 'svelte/store';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { invoke } from '@tauri-apps/api/core';

type Phase =
  | 'idle'
  | 'checking'
  | 'current'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'error';
export interface UpdateState {
  phase: Phase;
  version: string;
  notes: string;
  downloaded: number;
  total?: number;
  error: string;
}

export function createUpdater(
  deps = {
    check: () => check({ timeout: 30_000 }),
    pause: () => invoke<void>('set_running', { value: false }),
  },
) {
  const state = writable<UpdateState>({
    phase: 'idle',
    version: '',
    notes: '',
    downloaded: 0,
    error: '',
  });
  let candidate: Update | null = null;
  let working = false;
  let disposed = false;
  const close = (update: Update | null) => update?.close().catch(() => {});

  async function checkForUpdate() {
    if (working || disposed) return;
    working = true;
    state.update((s) => ({ ...s, phase: 'checking', error: '' }));
    await close(candidate);
    candidate = null;
    try {
      const update = await deps.check();
      if (disposed) {
        await close(update);
        return;
      }
      candidate = update;
      state.set({
        phase: update ? 'available' : 'current',
        version: update?.version ?? '',
        notes: update?.body ?? '',
        downloaded: 0,
        error: '',
      });
    } catch {
      if (!disposed)
        state.update((s) => ({
          ...s,
          phase: 'error',
          error:
            'Could not check for updates. Check your connection and try again. A published release may not be available yet.',
        }));
    } finally {
      working = false;
    }
  }

  async function install() {
    if (working || disposed || !candidate) return;
    working = true;
    const update = candidate;
    state.update((s) => ({
      ...s,
      phase: 'downloading',
      downloaded: 0,
      total: undefined,
      error: '',
    }));
    let downloaded = 0;
    let step: 'download' | 'pause' | 'install' = 'download';
    try {
      // Download verifies the signature before farming is paused or an installer runs.
      await update.download(
        (event) => {
          if (disposed) return;
          if (event.event === 'Started') {
            state.update((s) => ({ ...s, total: event.data.contentLength }));
          } else if (event.event === 'Progress') {
            downloaded += event.data.chunkLength;
            state.update((s) => ({ ...s, downloaded }));
          }
        },
        { timeout: 300_000 },
      );
      if (disposed) return;
      step = 'pause';
      await deps.pause();
      if (disposed) return;
      step = 'install';
      state.update((s) => ({ ...s, phase: 'installing' }));
      await update.install(); // Windows exits and the installer reopens the app.
    } catch {
      if (!disposed)
        state.update((s) => ({
          ...s,
          phase: 'error',
          error:
            step === 'download'
              ? 'Could not download or verify the update. Check for updates to try again.'
              : step === 'pause'
                ? 'Could not pause farming. Stop farming and try the update again.'
                : 'Could not start the installer. Farming is paused. Check for updates to try again.',
        }));
    } finally {
      await close(update);
      candidate = null;
      working = false;
    }
  }

  return {
    subscribe: state.subscribe,
    check: checkForUpdate,
    install,
    dispose() {
      disposed = true;
      if (!working) {
        void close(candidate);
        candidate = null;
      }
    },
  };
}
