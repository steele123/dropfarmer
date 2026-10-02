<script lang="ts">
  import { ChevronDown, X } from 'lucide-svelte';
  import {
    defaultCampaignFilters,
    campaignViewChanged,
    type CampaignFilters,
  } from './campaignFilters';
  let {
    filters = $bindable(),
    search = $bindable(),
    games,
    count,
    total,
  }: {
    filters: CampaignFilters;
    search: string;
    games: string[];
    count: number;
    total: number;
  } = $props();
  const fields = [
    {
      key: 'status',
      label: 'Status',
      options: [
        ['all', 'Any status'],
        ['active', 'Active'],
        ['upcoming', 'Upcoming'],
        ['ended', 'Ended'],
      ],
    },
    {
      key: 'progress',
      label: 'Rewards',
      options: [
        ['all', 'Any progress'],
        ['not-started', 'Not started'],
        ['in-progress', 'In progress'],
        ['claimable', 'Ready to claim'],
        ['completed', 'All claimed'],
      ],
    },
    {
      key: 'account',
      label: 'Account',
      options: [
        ['all', 'Any account'],
        ['linked', 'Linked'],
        ['unlinked', 'Link required'],
      ],
    },
    {
      key: 'queue',
      label: 'Queue',
      options: [
        ['all', 'All campaigns'],
        ['queued', 'In queue'],
        ['not-queued', 'Not in queue'],
      ],
    },
    {
      key: 'sort',
      label: 'Sort by',
      options: [
        ['default', 'Default order'],
        ['ending', 'Ending soonest'],
        ['starting', 'Newest start date'],
        ['game', 'Game A–Z'],
        ['progress', 'Most progress'],
      ],
    },
  ] as const;
</script>

<div class="campaign-filters" role="group" aria-label="Campaign filters">
  <div class="filter-fields">
    <label
      >Game
      <span class="select-wrap" class:filtered={!!filters.game}>
        <select bind:value={filters.game}>
          <option value="">All games</option>
          {#each games as game}<option value={game}>{game}</option>{/each}
        </select><ChevronDown size={13} />
      </span>
    </label>
    {#each fields as field}
      <label
        >{field.label}
        <span
          class="select-wrap"
          class:filtered={filters[field.key] !==
            (field.key === 'sort' ? 'default' : 'all')}
        >
          <select bind:value={filters[field.key]}>
            {#each field.options as [value, label]}<option {value}
                >{label}</option
              >{/each}
          </select><ChevronDown size={13} />
        </span>
      </label>
    {/each}
  </div>
  <div class="filter-summary">
    <label class="ending-toggle"
      ><input type="checkbox" bind:checked={filters.endingSoon} /> Ending within 72
      hours</label
    >
    <span class="result-count" role="status">{count} of {total} campaigns</span>
    <button
      class="clear-filters"
      disabled={!campaignViewChanged(filters, search)}
      onclick={() => {
        filters = defaultCampaignFilters();
        search = '';
      }}><X size={13} />Clear all</button
    >
  </div>
</div>

<style>
  .campaign-filters {
    margin-bottom: 20px;
    padding: 16px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
  }
  .filter-fields {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    color: var(--muted);
    font-size: 11px;
  }
  .select-wrap {
    position: relative;
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: #111;
    color: #ededed;
  }
  .select-wrap.filtered {
    border-color: #777;
  }
  .select-wrap :global(svg) {
    position: absolute;
    right: 9px;
    pointer-events: none;
    color: var(--muted);
  }
  select {
    appearance: none;
    width: 100%;
    min-width: 0;
    padding: 9px 26px 9px 10px;
    border: 0;
    border-radius: inherit;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: 12px;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  option {
    color: #ededed;
    background: #111;
  }
  select:focus-visible {
    outline: 2px solid #888;
    outline-offset: 2px;
  }
  .filter-summary {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
    margin-top: 16px;
    font-size: 11px;
    color: var(--muted);
  }
  .ending-toggle {
    flex-direction: row;
    align-items: center;
    cursor: pointer;
  }
  input {
    margin: 0;
    accent-color: #ededed;
  }
  .result-count {
    margin-left: auto;
    font-family: var(--mono);
  }
  .clear-filters {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: #ededed;
    font-size: 11px;
  }
  .clear-filters:disabled {
    opacity: 0.35;
    cursor: default;
  }
  @media (max-width: 1100px) {
    .filter-fields {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  @media (max-width: 560px) {
    .filter-fields {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .ending-toggle {
      width: 100%;
    }
    .result-count {
      margin-left: 0;
    }
    .clear-filters {
      margin-left: auto;
    }
  }
</style>
