<script lang="ts">
  import type { Campaign, Snapshot } from './model';
  let {
    config,
    campaigns,
    lastCheck,
    nextCheck,
    disabled,
    needsReconnect,
    loginMethod,
    onupdate,
  }: {
    config: Snapshot['autoFarm'];
    campaigns: Campaign[];
    lastCheck: string | null;
    nextCheck: string | null;
    disabled: boolean;
    needsReconnect: boolean;
    loginMethod: Snapshot['loginMethod'];
    onupdate: (enabled: boolean, gameIds: string[]) => Promise<void>;
  } = $props();
  let selectedGame = $state('');
  let search = $state('');
  const games = $derived(
    [
      ...new Map(
        campaigns
          .filter((c) => c.gameId)
          .map((c) => [c.gameId, { id: c.gameId, name: c.game }]),
      ).values(),
    ]
      .filter(
        (g) =>
          !config.games.some((f) => f.id === g.id) &&
          g.name.toLowerCase().includes(search.toLowerCase().trim()),
      )
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const time = (value: string) =>
    new Date(value).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
    });
</script>

<section class="auto-farm" aria-labelledby="auto-farm-title">
  <div class="setting-row">
    <div>
      <h3 id="auto-farm-title">Auto farm followed games</h3>
      <p>
        Check every 15 minutes and start farming new watch-time drops. Works
        while the app is open or in the tray.
      </p>
    </div>
    <input
      type="checkbox"
      role="switch"
      aria-label="Auto farm followed games"
      checked={config.enabled}
      disabled={disabled ||
        (!config.enabled && (needsReconnect || !config.games.length))}
      onchange={(e) => {
        const enabled = e.currentTarget.checked;
        e.currentTarget.checked = config.enabled;
        void onupdate(
          enabled,
          config.games.map((g) => g.id),
        );
      }}
    />
  </div>
  <div class="follow-controls">
    <input
      class="game-search"
      aria-label="Search games to follow"
      placeholder="Search loaded games…"
      bind:value={search}
      {disabled}
    />
    <select aria-label="Game to follow" bind:value={selectedGame} {disabled}>
      <option value="">Choose a game</option>
      {#each games as game (game.id)}<option value={game.id}>{game.name}</option
        >{/each}
    </select>
    <button
      class="secondary"
      disabled={disabled || !games.some((g) => g.id === selectedGame)}
      onclick={async () => {
        await onupdate(config.enabled, [
          ...config.games.map((g) => g.id),
          selectedGame,
        ]);
        selectedGame = '';
      }}>Follow game</button
    >
  </div>
  <div class="followed-games">
    {#each config.games as game (game.id)}
      <div class="followed-game">
        <span>{game.name}</span>
        <button
          class="text-button"
          {disabled}
          aria-label={`Unfollow ${game.name}`}
          onclick={() =>
            onupdate(
              config.enabled && config.games.length > 1,
              config.games.filter((g) => g.id !== game.id).map((g) => g.id),
            )}>Unfollow</button
        >
      </div>
    {:else}<p>
        No followed games. Refresh campaigns to load games you can follow.
      </p>{/each}
  </div>
  <p class="auto-farm-note" role="status">
    {#if needsReconnect}Reconnect Twitch to check for new drops.
    {:else if config.enabled}{lastCheck
        ? `Last checked ${time(lastCheck)}.`
        : 'Waiting for the first check.'}
      {nextCheck ? `Next check ${time(nextCheck)}.` : 'Checking soon.'}
    {:else}Auto farm is off.{/if}
  </p>
  <p class="auto-farm-note">
    New campaigns go to the end of your queue. Current unclaimed drops are
    included on the first check. Pausing farming turns this off. Sleep when
    finished is unavailable while this is on.
  </p>
  {#if loginMethod === 'code'}<p class="auto-farm-note">
      Browser sign-in is recommended here. Code sign-in may only show campaigns
      already in progress.
    </p>{/if}
</section>

<style>
  .auto-farm {
    border-bottom: 1px solid #242424;
    padding-bottom: 24px;
  }
  .auto-farm .setting-row {
    border-bottom: 0;
  }
  .follow-controls,
  .followed-games,
  .auto-farm-note {
    margin-left: 24px;
    margin-right: 24px;
  }
  .setting-row p {
    max-width: 620px;
  }
  .follow-controls {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .follow-controls input,
  .follow-controls select {
    min-width: 180px;
    flex: 1;
    padding: 10px 12px;
    color: #ededed;
    background: #111;
    border: 1px solid #333;
    border-radius: 6px;
  }
  .followed-games {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 16px;
  }
  .followed-game {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 12px;
    border: 1px solid #292929;
    border-radius: 6px;
    font-size: 13px;
  }
  .followed-games p,
  .auto-farm-note {
    color: #999;
    font-size: 12px;
    line-height: 1.6;
  }
  .auto-farm-note {
    margin-top: 12px;
    margin-bottom: 0;
  }
</style>
