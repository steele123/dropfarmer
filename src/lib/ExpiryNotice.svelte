<script lang="ts">
  import { TriangleAlert } from 'lucide-svelte';
  import type { ExpiryWarning } from './expiry';
  let {
    warnings,
    expanded = false,
  }: { warnings: ExpiryWarning[]; expanded?: boolean } = $props();
</script>

{#if warnings.length}
  <div class="expiry-notices" aria-label="Reward expiry warnings">
    {#each expanded ? warnings : warnings.slice(0, 1) as warning (warning.dropId)}
      <div
        class="expiry-notice"
        class:urgent={warning.level === 'insufficient'}
        title={`${warning.detail} Deadline: ${new Date(warning.deadline).toLocaleString()}`}
      >
        <TriangleAlert size={13} />
        <div>
          <span>{warning.message}</span>{#if expanded}<p>{warning.detail}</p>
            <time datetime={new Date(warning.deadline).toISOString()}
              >Ends {new Date(warning.deadline).toLocaleString()}</time
            >{/if}
        </div>
      </div>
    {/each}
    {#if !expanded && warnings.length > 1}<span class="more"
        >+{warnings.length - 1} other {warnings.length === 2
          ? 'reward warning'
          : 'reward warnings'}</span
      >{/if}
  </div>
{/if}

<style>
  .expiry-notices {
    display: grid;
    gap: 8px;
    margin: 12px 0;
  }
  .expiry-notice {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    color: #e9bd74;
    font-size: 11px;
    line-height: 1.5;
  }
  .expiry-notice :global(svg) {
    flex-shrink: 0;
    margin-top: 2px;
  }
  .expiry-notice > div {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .urgent {
    color: #ff9c91;
  }
  .expiry-notice p {
    margin: 5px 0;
    font-size: 11px;
    color: var(--muted);
  }
  time,
  .more {
    font-size: 10px;
    color: var(--muted);
  }
  .more {
    padding-left: 20px;
  }
</style>
