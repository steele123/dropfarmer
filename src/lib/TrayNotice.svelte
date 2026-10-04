<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { ChevronUp, Volume2, Wifi } from 'lucide-svelte';

  let { onerror }: { onerror: (message: string) => void } = $props();
  const preferenceKey = 'dropfarmer.tray-notice-dismissed';
  let dialog: HTMLDialogElement;
  let remember = $state(true);
  let busy = $state(false);

  async function hide() {
    busy = true;
    try {
      await invoke('hide_to_tray', { remember });
      try {
        if (remember) localStorage.setItem(preferenceKey, 'true');
        else localStorage.removeItem(preferenceKey);
      } catch {
        // The notice will appear again next launch if storage is unavailable.
      }
      dialog.close();
    } catch {
      onerror('Could not move Dropfarmer to the tray. Please try again.');
      dialog.close();
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await listen('tray-hide-requested', () => {
        if (!disposed && !dialog.open) dialog.showModal();
      });
      if (disposed) {
        unlisten();
        return;
      }
      let dismissed = false;
      try {
        dismissed = localStorage.getItem(preferenceKey) === 'true';
      } catch {
        // Keep the explanation available without browser storage.
      }
      await invoke('configure_tray_notice', { showNotice: !dismissed });
    })().catch(() => {
      if (!disposed) onerror('Could not prepare the tray notice.');
    });
    return () => {
      disposed = true;
      unlisten?.();
      void invoke('configure_tray_notice', { showNotice: false }).catch(
        () => {},
      );
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="tray-notice-title"
  aria-describedby="tray-notice-description"
>
  <span class="eyebrow">SYSTEM TRAY</span>
  <h2 id="tray-notice-title">Move Dropfarmer to the tray?</h2>
  <p id="tray-notice-description">
    Dropfarmer will move to the tray near your clock. Farming keeps going while
    the window is hidden.
  </p>
  <div class="tray-preview" aria-hidden="true">
    <div class="tray-overflow">
      <img src="/dropfarmer.svg" alt="" width="28" height="28" /><span
        >Dropfarmer</span
      >
    </div>
    <div class="taskbar">
      <span class="arrow"><ChevronUp size={18} /></span><Wifi
        size={17}
      /><Volume2 size={17} /><span class="clock">12:00</span>
    </div>
  </div>
  <p>
    Click the <strong>⌃ arrow</strong>, then the
    <strong>Dropfarmer icon</strong> to reopen it. The icon may already be next to
    the clock.
  </p>
  <p class="quit-hint">
    To exit completely, right-click the icon and choose <strong
      >Quit Dropfarmer</strong
    >.
  </p>
  <label
    ><input type="checkbox" bind:checked={remember} disabled={busy} /> Don’t show
    this again</label
  >
  <div class="actions">
    <button class="secondary" disabled={busy} onclick={() => dialog.close()}
      >Keep open</button
    >
    <button class="primary" disabled={busy} onclick={hide}>Move to tray</button>
  </div>
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 48px));
    box-sizing: border-box;
    margin: auto;
    padding: 28px;
    border: 1px solid #333;
    border-radius: 12px;
    background: #0a0a0a;
    color: #ededed;
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 72%);
  }
  .eyebrow {
    color: #888;
    font-size: 10px;
    letter-spacing: 0.12em;
  }
  h2 {
    margin: 12px 0;
    font-size: 21px;
    font-weight: 600;
    letter-spacing: -0.5px;
  }
  p {
    color: #a1a1a1;
    font-size: 13px;
    line-height: 1.6;
    margin: 12px 0;
  }
  strong {
    color: #ededed;
    font-weight: 500;
  }
  .tray-preview {
    margin: 22px 0;
    background: #111;
    border: 1px solid #262626;
    border-radius: 8px;
    overflow: hidden;
    padding-top: 18px;
  }
  .tray-overflow {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 18px 12px auto;
    padding: 12px 16px;
    width: fit-content;
    border: 1px solid #444;
    border-radius: 7px;
    background: #202020;
    font-size: 12px;
  }
  .taskbar {
    border-top: 1px solid #292929;
    background: #181818;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 18px;
    padding: 10px 20px;
    color: #bbb;
  }
  .arrow {
    display: flex;
    align-items: center;
    border: 1px solid #555;
    border-radius: 4px;
    padding: 3px;
    color: white;
  }
  .clock {
    font-size: 12px;
  }
  .quit-hint {
    font-size: 12px;
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: #a1a1a1;
    margin-top: 22px;
  }
  input {
    appearance: auto;
    width: 14px;
    height: 14px;
    margin: 0;
    accent-color: #ededed;
  }
  input::before {
    display: none;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 24px;
  }
</style>
