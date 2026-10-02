<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { Copy, Minus, Square, X } from 'lucide-svelte';

  let {
    onerror,
    trayEnabled = true,
  }: { onerror: (message: string) => void; trayEnabled?: boolean } = $props();
  let desktop = $state(false);
  let maximized = $state(false);
  let focused = $state(true);

  async function control(action: 'minimize' | 'toggleMaximize' | 'close') {
    if (!desktop) return;
    try {
      const window = getCurrentWindow();
      if (action === 'minimize') await invoke('minimize_window');
      else await window[action]();
      if (action === 'toggleMaximize') maximized = await window.isMaximized();
    } catch {
      onerror('Could not update the window. Please try again.');
    }
  }

  onMount(() => {
    desktop = isTauri();
    if (!desktop) return;
    const window = getCurrentWindow();
    const cleanups: (() => void)[] = [];
    let disposed = false;
    const retain = (unlisten: () => void) => {
      if (disposed) unlisten();
      else cleanups.push(unlisten);
    };
    const syncMaximized = async () => {
      const value = await window.isMaximized();
      if (!disposed) maximized = value;
    };
    void Promise.all([
      syncMaximized(),
      window.isFocused().then((value) => {
        if (!disposed) focused = value;
      }),
      window
        .onResized(() => {
          void syncMaximized().catch(() => {});
        })
        .then(retain),
      window
        .onFocusChanged(({ payload }) => {
          if (!disposed) focused = payload;
        })
        .then(retain),
    ]).catch(() => {
      if (!disposed) onerror('Could not read the window state.');
    });
    return () => {
      disposed = true;
      cleanups.forEach((unlisten) => unlisten());
    };
  });
</script>

<div class="window-titlebar" class:unfocused={!focused}>
  <div class="window-drag-region" data-tauri-drag-region>
    <span class="window-brand" aria-hidden="true"
      ><img
        src="/dropfarmer.svg"
        alt=""
        width="18"
        height="18"
        draggable="false"
      /><span>Dropfarmer</span></span
    >
  </div>
  <div class="window-controls" role="group" aria-label="Window controls">
    <button
      aria-label={trayEnabled ? 'Minimize to tray' : 'Minimize window'}
      title={trayEnabled ? 'Minimize to tray' : 'Minimize'}
      disabled={!desktop}
      onclick={() => control('minimize')}
      ><Minus size={15} strokeWidth={1.4} /></button
    >
    <button
      aria-label={maximized ? 'Restore window' : 'Maximize window'}
      title={maximized ? 'Restore' : 'Maximize'}
      disabled={!desktop}
      onclick={() => control('toggleMaximize')}
    >
      {#if maximized}<Copy size={12} strokeWidth={1.4} />{:else}<Square
          size={12}
          strokeWidth={1.4}
        />{/if}
    </button>
    <button
      class="window-close"
      aria-label={trayEnabled ? 'Hide to tray' : 'Close window'}
      title={trayEnabled ? 'Hide to tray — quit from the tray menu' : 'Close'}
      disabled={!desktop}
      onclick={() => control('close')}><X size={16} strokeWidth={1.4} /></button
    >
  </div>
</div>

<style>
  .window-titlebar {
    height: 36px;
    display: flex;
    position: sticky;
    top: 0;
    z-index: 30;
    background: #0a0a0a;
    border-bottom: 1px solid #202020;
    user-select: none;
  }
  .window-drag-region {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    padding-left: 16px;
  }
  .window-brand {
    display: flex;
    align-items: center;
    gap: 9px;
    color: #a1a1a1;
    font-size: 11px;
    font-weight: 500;
    pointer-events: none;
  }
  .window-brand :global(*) {
    pointer-events: none;
  }
  .unfocused .window-brand {
    color: #666;
  }
  .window-controls {
    display: flex;
    height: 100%;
  }
  .window-controls button {
    width: 46px;
    height: 100%;
    padding: 0;
    border-radius: 0;
    color: #a1a1a1;
    cursor: default;
  }
  .window-controls button:hover:not(:disabled) {
    background: #242424;
    color: #ededed;
  }
  .window-controls .window-close:hover:not(:disabled) {
    background: #c42b1c;
    color: white;
  }
  .window-controls button:focus-visible {
    outline-offset: -3px;
  }
  .window-controls button:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
