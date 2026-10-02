<script lang="ts">
  import { Download, RefreshCw } from 'lucide-svelte';
  import { version } from '../../package.json';
  import type { createUpdater } from './updater';
  let {
    updater,
    desktop,
    disabled = false,
  }: {
    updater: ReturnType<typeof createUpdater>;
    desktop: boolean;
    disabled?: boolean;
  } = $props();
  let updating = $derived(
    $updater.phase === 'downloading' || $updater.phase === 'installing',
  );
  let percent = $derived(
    $updater.total
      ? Math.min(100, Math.floor(($updater.downloaded / $updater.total) * 100))
      : undefined,
  );
</script>

<div class="setting-row">
  <div>
    <h3>App updates</h3>
    <p aria-live="polite">
      {#if !desktop}Version {version} · Updates are available in the desktop app.
      {:else if $updater.phase === 'checking'}Checking for updates…
      {:else if $updater.phase === 'available'}Version {$updater.version} is available.
        Installing will pause farming and restart the app.
      {:else if $updater.phase === 'current'}Version {version} · You’re up to date.
      {:else if $updater.phase === 'downloading'}Downloading{percent ===
        undefined
          ? '…'
          : `… ${percent}%`}
      {:else if $updater.phase === 'installing'}Installing update…
      {:else}Version {version} · Checks when the app opens.{/if}
    </p>
    {#if $updater.phase === 'error'}<p class="update-error" role="alert">
        {$updater.error}
      </p>{/if}
    {#if $updater.phase === 'available' && $updater.notes}
      <details>
        <summary>Release notes</summary>
        <pre>{$updater.notes}</pre>
      </details>
    {/if}
    {#if $updater.phase === 'downloading'}<progress
        max="100"
        value={percent}
        aria-label="Update download"
      ></progress>{/if}
  </div>
  {#if $updater.phase === 'available'}
    <button
      class="secondary"
      disabled={disabled || !desktop}
      onclick={() => updater.install()}
      ><Download size={14} /> Download and install</button
    >
  {:else}
    <button
      class="secondary"
      disabled={!desktop ||
        disabled ||
        updating ||
        $updater.phase === 'checking'}
      onclick={() => updater.check()}
      ><RefreshCw size={14} /> Check for updates</button
    >
  {/if}
</div>

<style>
  .setting-row > div {
    min-width: 0;
  }
  button {
    flex-shrink: 0;
  }
  details {
    margin-top: 14px;
    color: #a1a1a1;
    font-size: 12px;
  }
  summary {
    cursor: pointer;
    width: fit-content;
  }
  pre {
    font: inherit;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 180px;
    overflow-y: auto;
  }
  .update-error {
    color: #fca5a5;
  }
  progress {
    width: 240px;
    max-width: 100%;
    height: 5px;
    accent-color: #ededed;
    margin-top: 14px;
  }
</style>
