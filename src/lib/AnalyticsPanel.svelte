<script lang="ts">
  import {
    Gift,
    Clock,
    Check,
    Gamepad2,
    Search,
    X,
    Download,
  } from 'lucide-svelte';
  import { exportHistory } from './historyExport';
  import {
    farmingTime,
    rewardHistory,
    summarizeAnalytics,
    type Analytics,
  } from './analytics';
  let { data, connected }: { data: Analytics; connected: boolean } = $props();
  let query = $state('');
  let game = $state('');
  let limit = $state(50);
  let exporting = $state(false);
  let exportMessage = $state('');
  let exportError = $state('');
  async function downloadHistory() {
    if (exporting || !history.length) return;
    exporting = true;
    exportMessage = '';
    exportError = '';
    const rewards = [...history];
    try {
      const path = await exportHistory(rewards);
      exportMessage = path
        ? `Saved ${rewards.length} rewards to ${path}`
        : `Download started for ${rewards.length} rewards.`;
    } catch (error) {
      exportError = String(error);
    } finally {
      exporting = false;
    }
  }
  let summary = $derived(summarizeAnalytics(data));
  let games = $derived([...new Set(data.rewards.map((r) => r.game))].sort());
  let history = $derived(rewardHistory(data.rewards, query, game));
  let largest = $derived(Math.max(1, ...summary.games.map((g) => g.farmingMs)));
  const date = (value: string) =>
    new Date(value).toLocaleDateString(undefined, {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
    });
  $effect(() => {
    query;
    game;
    data.startedAt;
    limit = 50;
  });
</script>

<div class="analytics">
  {#if data.error}<p class="analytics-error" role="alert">{data.error}</p>{/if}
  <p class="intro">
    {connected
      ? 'All time · Saved on this device for your Twitch account.'
      : 'Connect Twitch to load your analytics and received rewards.'}
  </p>
  <div class="metrics">
    <section>
      <span><Gift size={16} />Drops claimed</span><strong
        >{summary.drops.toLocaleString()}</strong
      ><small>Confirmed by Twitch</small>
    </section>
    <section>
      <span><Clock size={16} />Farming time</span><strong
        >{farmingTime(summary.farmingMs)}</strong
      ><small>Active time tracked here</small>
    </section>
    <section>
      <span><Check size={16} />Campaigns completed</span><strong
        >{summary.completed.toLocaleString()}</strong
      ><small>Every reward claimed</small>
    </section>
    <section>
      <span><Gamepad2 size={16} />Games with rewards</span><strong
        >{summary.gamesRewarded.toLocaleString()}</strong
      ><small>Across your saved history</small>
    </section>
  </div>

  <section class="breakdown">
    <div class="section-heading">
      <h2>By game</h2>
      <span class="muted">Farming time / drops</span>
    </div>
    {#if summary.games.length}
      <div class="game-list">
        {#each summary.games as row (row.game)}
          <div class="game-row">
            <div class="game-name">
              {row.game || 'Unknown game'}
              <div class="time-track">
                <span style:width={`${(row.farmingMs / largest) * 100}%`}
                ></span>
              </div>
            </div>
            <span class="time">{farmingTime(row.farmingMs)}</span><span
              class="drop-count"
              >{row.drops} {row.drops === 1 ? 'drop' : 'drops'}</span
            >
          </div>
        {/each}
      </div>
    {:else}<p class="quiet-empty">
        Your games will appear here as you farm or sync claimed rewards.
      </p>{/if}
  </section>

  <section>
    <div class="section-heading">
      <h2>Received rewards <span>{data.rewards.length}</span></h2>
      <button
        class="secondary"
        disabled={exporting || !history.length}
        onclick={downloadHistory}
        title="Export all matching rewards as CSV. Desktop exports are saved in Downloads."
      >
        <Download size={14} />{exporting ? 'Exporting…' : 'Export CSV'}
      </button>
    </div>
    {#if exportMessage}<p class="export-message" role="status">
        {exportMessage}
      </p>{/if}
    {#if exportError}<p class="analytics-error" role="alert">
        {exportError}
      </p>{/if}
    <div class="history-filters">
      <label class="history-search"
        ><Search size={16} /><input
          aria-label="Search received rewards"
          placeholder="Search rewards or campaigns…"
          bind:value={query}
        /></label
      >
      <select aria-label="Filter received rewards by game" bind:value={game}
        ><option value="">All games</option>{#each games as name}<option
            value={name}>{name}</option
          >{/each}</select
      >
      {#if query || game}<button
          class="secondary"
          onclick={() => {
            query = '';
            game = '';
          }}><X size={14} />Clear</button
        >{/if}
    </div>
    <p class="history-note">
      Dates show when Dropfarmer first recorded each claim. Earlier claims are
      included when Twitch returns them.
    </p>
    {#if history.length}
      <div class="history-list">
        {#each history.slice(0, limit) as reward (`${reward.campaignId}:${reward.id}`)}
          <article class="history-row">
            <span class="reward-image"
              >{#if reward.image}<img
                  src={reward.image}
                  alt=""
                  loading="lazy"
                />{:else}<Gift size={22} />{/if}</span
            >
            <div class="reward-copy">
              <h3>{reward.name}</h3>
              <p>{reward.game} <span>· {reward.campaign}</span></p>
            </div>
            <div class="recorded">
              <span class="claimed"><Check size={13} />Claimed</span><time
                datetime={reward.recordedAt}
                title={new Date(reward.recordedAt).toLocaleString()}
                >{date(reward.recordedAt)}</time
              >
            </div>
          </article>
        {/each}
      </div>
      <div class="history-footer">
        <span
          >{Math.min(limit, history.length)} of {history.length} rewards</span
        >{#if history.length > limit}<button
            class="secondary"
            onclick={() => (limit += 50)}>Show more</button
          >{/if}
      </div>
    {:else}<div class="quiet-empty">
        <Gift size={24} />
        <h3>
          {data.rewards.length
            ? 'No matching rewards'
            : 'No recorded claims yet'}
        </h3>
        <p>
          {data.rewards.length
            ? 'Try another game or search.'
            : 'Refresh campaigns to record claimed drops. They stay here after campaigns end.'}
        </p>
      </div>{/if}
  </section>
  <p class="footnote">
    Farming time is an estimate while Dropfarmer is active on a live channel; it
    is separate from Twitch watch progress. Paused, offline, and suspended time
    is excluded.{#if data.startedAt}{' '}Tracking since {date(
        data.startedAt,
      )}.{/if}
  </p>
</div>

<style>
  .export-message {
    font-size: 12px;
    color: #80d4a8;
    margin-bottom: 12px;
    overflow-wrap: anywhere;
  }
  .analytics {
    display: grid;
    gap: 28px;
  }
  .intro,
  .history-note,
  .footnote {
    color: var(--muted);
    font-size: 12px;
  }
  .metrics {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
  }
  .metrics section {
    padding: 21px;
    min-width: 0;
    display: grid;
    gap: 14px;
  }
  .metrics section + section {
    border-left: 1px solid var(--border);
  }
  .metrics section > span {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 12px;
  }
  .metrics strong {
    font: 500 28px var(--mono);
    letter-spacing: -1.3px;
  }
  .metrics small {
    color: var(--muted);
    font-size: 11px;
  }
  .game-list,
  .history-list {
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
  }
  .game-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 90px 65px;
    gap: 20px;
    align-items: center;
    padding: 18px 20px;
    font-size: 13px;
  }
  .game-row + .game-row,
  .history-row + .history-row {
    border-top: 1px solid var(--border);
  }
  .game-name {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .time-track {
    height: 4px;
    background: #222;
    border-radius: 3px;
    margin-top: 10px;
  }
  .time-track span {
    display: block;
    height: 100%;
    background: #ededed;
    border-radius: inherit;
  }
  .time,
  .drop-count {
    text-align: right;
    font-family: var(--mono);
    font-size: 12px;
  }
  .drop-count {
    color: var(--muted);
  }
  .history-filters {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-bottom: 12px;
  }
  .history-search {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 180px;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 12px;
    color: var(--muted);
    background: var(--panel);
  }
  .history-search input {
    width: 100%;
    background: transparent;
    border: 0;
    padding: 11px 0;
    color: inherit;
    font-size: 13px;
  }
  select {
    color: #ededed;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 9px 30px 9px 12px;
    max-width: 100%;
    font: inherit;
    font-size: 13px;
  }
  select:focus-visible {
    outline: 2px solid #52a8ff;
    outline-offset: 3px;
  }
  .history-note {
    margin-bottom: 16px;
  }
  .history-row {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 16px 20px;
  }
  .reward-copy {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .reward-copy h3 {
    font-size: 13px;
  }
  .reward-copy p {
    font-size: 12px;
    color: var(--muted);
    margin-top: 4px;
  }
  .recorded {
    display: grid;
    gap: 7px;
    justify-items: end;
    flex-shrink: 0;
  }
  time {
    color: var(--muted);
    font-size: 11px;
  }
  .history-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 16px;
    color: var(--muted);
    font-size: 12px;
  }
  .quiet-empty {
    display: grid;
    justify-items: center;
    gap: 12px;
    padding: 30px 20px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
    border: 1px dashed var(--border);
    border-radius: 8px;
  }
  .quiet-empty h3 {
    color: #ededed;
  }
  .analytics-error {
    color: #ffb4b4;
    font-size: 13px;
    border: 1px solid #663333;
    border-radius: 6px;
    padding: 12px;
  }
  @media (max-width: 1050px) {
    .metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .metrics section:nth-child(3) {
      border-left: 0;
    }
    .metrics section:nth-child(n + 3) {
      border-top: 1px solid var(--border);
    }
  }
  @media (max-width: 600px) {
    .metrics section {
      padding: 16px 12px;
    }
    .metrics strong {
      font-size: 24px;
    }
    .game-row {
      gap: 10px;
      padding: 15px 12px;
      grid-template-columns: minmax(0, 1fr) 65px 50px;
    }
    .history-row {
      gap: 10px;
      padding: 15px 12px;
      flex-wrap: wrap;
    }
    .recorded {
      width: 100%;
      display: flex;
      justify-content: space-between;
    }
  }
</style>
