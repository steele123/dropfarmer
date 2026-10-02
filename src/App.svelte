<script lang="ts">
  import { onMount } from 'svelte';
  import TitleBar from './lib/TitleBar.svelte';
  import UpdatePanel from './lib/UpdatePanel.svelte';
  import { createUpdater } from './lib/updater';
  import { version } from '../package.json';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import {
    LayoutGrid,
    ListOrdered,
    Gift,
    Activity,
    Settings,
    ArrowUpRight,
    ArrowUp,
    ArrowDown,
    Plus,
    Check,
    Play,
    Pause,
    Search,
    RefreshCw,
    X,
    Link2,
    LogOut,
    ChevronRight,
    Clock,
    Radio,
    ShieldCheck,
    CircleHelp,
  } from 'lucide-svelte';
  import {
    initialState,
    previewState,
    progress,
    complete,
    canQueue,
    moveQueue,
    deadline,
    duration,
    campaignState,
    matchesCampaign,
    type Snapshot,
    type Campaign,
    type LoginCode,
  } from './lib/model';
  let farm = $state<Snapshot>(initialState());
  let page = $state('Campaigns');
  let filter = $state('All campaigns');
  let search = $state('');
  let busy = $state('');
  let refreshing = $state(false);
  let error = $state('');
  let preview = $state(false);
  let desktop = $state(false);
  let ready = $state(false);
  let selected = $state<Campaign | null>(null);
  let login = $state<LoginCode | null>(null);
  let loginOpen = $state(false);
  let loginMode = $state<'choice' | 'browser' | 'code'>('choice');
  let loginError = $state('');
  let loginAttempt = 0;
  let connecting = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  const updater = createUpdater();
  let updating = $derived(
    $updater.phase === 'downloading' || $updater.phase === 'installing',
  );
  const navigation = [
    { name: 'Campaigns', icon: LayoutGrid },
    { name: 'My queue', icon: ListOrdered },
    { name: 'Inventory', icon: Gift },
    { name: 'Activity', icon: Activity },
    { name: 'Settings', icon: Settings },
  ];
  let queued = $derived(
    farm.queue
      .map((id) => farm.campaigns.find((c) => c.id === id))
      .filter((c): c is Campaign => !!c),
  );
  let rewards = $derived(
    farm.campaigns.flatMap((c) =>
      c.drops.map((d) => ({ ...d, game: c.game, campaignId: c.id })),
    ),
  );
  let active = $derived(
    farm.campaigns.find((c) => c.id === farm.activeCampaign),
  );
  let visible = $derived(
    farm.campaigns.filter(
      (c) =>
        matchesCampaign(c, search) &&
        (filter === 'All campaigns' ||
          (filter === 'Linked accounts' && c.linked) ||
          (filter === 'Ending soon' &&
            new Date(c.endsAt).getTime() - Date.now() < 86400000 * 3 &&
            campaignState(c) !== 'Ended')),
    ),
  );
  let selectedLive = $derived(
    selected
      ? (farm.campaigns.find((c) => c.id === selected?.id) ?? selected)
      : null,
  );
  async function action(name: string, fn: () => Promise<void>) {
    if (busy || updating) return;
    busy = name;
    error = '';
    try {
      await fn();
    } catch (e) {
      error = String(e);
    } finally {
      busy = '';
    }
  }
  async function command(name: string, args: Record<string, unknown> = {}) {
    await invoke(name, args);
    farm = await invoke<Snapshot>('get_state');
  }
  async function refresh() {
    if (refreshing || preview) return;
    refreshing = true;
    error = '';
    try {
      if (!desktop) throw 'Open the desktop app to connect Twitch.';
      await invoke('refresh');
      if (!disposed && !preview) farm = await invoke<Snapshot>('get_state');
    } catch (e) {
      if (!disposed && !preview) error = String(e);
    } finally {
      refreshing = false;
    }
  }
  async function changeQueue(ids: string[]) {
    await action('Updating queue', async () => {
      if (preview) {
        farm = { ...farm, queue: ids };
        return;
      }
      await command('set_queue', { ids });
    });
  }
  function toggleQueue(c: Campaign) {
    void changeQueue(
      farm.queue.includes(c.id)
        ? farm.queue.filter((id) => id !== c.id)
        : [...farm.queue, c.id],
    );
  }
  function showPreview() {
    preview = true;
    farm = previewState();
    error = '';
  }
  async function leavePreview() {
    preview = false;
    selected = null;
    farm = desktop ? await invoke<Snapshot>('get_state') : initialState();
  }
  async function openLink(url: string) {
    try {
      if (desktop) await invoke('open_twitch', { url });
      else window.open(url, '_blank', 'noopener,noreferrer');
    } catch (e) {
      error = String(e);
    }
  }
  async function connect() {
    loginOpen = true;
    loginMode = 'choice';
    loginError = '';
  }
  async function startLogin(method: 'browser' | 'code') {
    await action('Connecting Twitch', async () => {
      const attempt = ++loginAttempt;
      loginError = '';
      try {
        if (!desktop) throw 'Open the desktop app to sign in.';
        if (preview) await leavePreview();
        loginMode = method;
        if (method === 'browser') await invoke('begin_browser_login');
        else login = await invoke<LoginCode>('begin_login');
        if (attempt !== loginAttempt) return;
        connecting = true;
        schedulePoll(method === 'browser' ? 2 : login!.interval, attempt);
      } catch (e) {
        if (attempt !== loginAttempt) return;
        loginError = String(e);
        loginMode = 'choice';
        connecting = false;
      }
    });
  }
  function schedulePoll(seconds: number, attempt: number) {
    clearTimeout(pollTimer);
    pollTimer = setTimeout(async () => {
      if (disposed || !connecting || attempt !== loginAttempt) return;
      try {
        const done = await invoke<boolean>('poll_login');
        if (disposed || !connecting || attempt !== loginAttempt) return;
        if (done) {
          connecting = false;
          login = null;
          loginOpen = false;
          farm = await invoke<Snapshot>('get_state');
          await refresh();
        } else
          schedulePoll(
            loginMode === 'browser' ? 2 : (login?.interval ?? 5),
            attempt,
          );
      } catch (e) {
        if (disposed || attempt !== loginAttempt) return;
        loginError = String(e);
        connecting = false;
        login = null;
        loginMode = 'choice';
      }
    }, seconds * 1000);
  }
  function openModal(node: HTMLDialogElement) {
    node.showModal();
    return { destroy: () => node.close() };
  }
  function cancelLogin() {
    loginAttempt++;
    connecting = false;
    login = null;
    loginOpen = false;
    clearTimeout(pollTimer);
    if (desktop)
      void invoke('cancel_login').catch((e) => {
        error = String(e);
      });
  }
  async function toggleRunning() {
    await action('Updating farmer', async () => {
      if (preview) throw 'Connect Twitch to start farming.';
      await command('set_running', { value: !farm.running });
    });
  }
  async function autoClaim(value: boolean) {
    await action('Saving settings', async () => {
      if (preview) {
        farm = { ...farm, autoClaim: value };
        return;
      }
      await command('set_auto_claim', { value });
    });
  }
  async function claim(campaignId: string, dropId: string) {
    await action('Claiming reward', async () => {
      if (preview) return;
      await command('claim_drop', { campaignId, dropId });
    });
  }
  onMount(() => {
    desktop = isTauri();
    if (desktop) void updater.check();
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        if (desktop) {
          unlisten = await listen<Snapshot>('farmer-state', (event) => {
            if (!preview) farm = event.payload;
          });
          if (disposed) {
            unlisten();
            return;
          }
          farm = await invoke<Snapshot>('initialize');
          ready = true;
          if (farm.account) void refresh();
        }
      } catch (e) {
        error = String(e);
      } finally {
        ready = true;
      }
    })();
    return () => {
      disposed = true;
      updater.dispose();
      if (desktop && loginOpen) void invoke('cancel_login').catch(() => {});
      clearTimeout(pollTimer);
      unlisten?.();
    };
  });
</script>

<svelte:head><title>Dropfarmer</title></svelte:head>

<div class="app-shell">
  <TitleBar onerror={(message) => (error = message)} />
  <header class="app-header">
    <div class="header-inner">
      <a href="#campaigns" class="brand" onclick={() => (page = 'Campaigns')}
        ><span class="brand-mark"
          ><img
            src="/dropfarmer.svg"
            alt=""
            width="27"
            height="27"
            draggable="false"
          /></span
        >Dropfarmer</a
      >
      <span class="header-divider">/</span><span class="workspace-name"
        >Twitch Drops</span
      >
      <span class="version">v{version}</span>
      {#if $updater.phase === 'available'}<button
          class="update-available"
          onclick={() => (page = 'Settings')}>Update available</button
        >{/if}
      <div class="header-actions">
        <div class="engine-note">
          <span class:live={farm.running} class="status-dot"></span><strong
            >{farm.running ? 'Running' : 'Stopped'}</strong
          >
        </div>
        <div class="account">
          <span class="avatar"
            >{farm.account?.login.slice(0, 2).toUpperCase() ?? 'DF'}</span
          >
          <div>
            <strong>{farm.account?.login ?? 'Not connected'}</strong><small
              >Twitch account</small
            >
          </div>
          {#if farm.account}<button
              class="icon-button"
              title="Disconnect Twitch"
              aria-label="Disconnect Twitch"
              disabled={!!busy}
              onclick={() => action('Disconnecting', () => command('logout'))}
              ><LogOut size={16} /></button
            >{/if}
        </div>
      </div>
    </div>
    <nav aria-label="Main navigation">
      {#each navigation as item}<button
          class:active={page === item.name}
          aria-current={page === item.name ? 'page' : undefined}
          onclick={() => (page = item.name)}
          ><item.icon size={16} /><span>{item.name}</span
          >{#if item.name === 'My queue' && farm.queue.length}<b
              >{farm.queue.length}</b
            >{/if}</button
        >{/each}
    </nav>
  </header>

  <main>
    <div class="main-content">
      {#if preview}<div class="preview-banner">
          <span
            ><CircleHelp size={16} /> Preview mode · sample campaigns, no Twitch activity</span
          ><button onclick={() => action('Leaving preview', leavePreview)}
            >Exit preview <X size={14} /></button
          >
        </div>{/if}
      {#if error || farm.error}<div class="error-banner" role="alert">
          <span>{error || farm.error}</span><button
            aria-label="Dismiss error"
            onclick={() => {
              error = '';
              farm = { ...farm, error: null };
            }}><X size={16} /></button
          >
        </div>{/if}
      <div class="page-heading">
        <div>
          <h1>{page}</h1>
        </div>
        {#if farm.account || preview}<button
            class="secondary"
            disabled={!!busy || refreshing}
            onclick={refresh}
            ><RefreshCw
              size={15}
              class={refreshing ? 'spin' : ''}
            />Refresh</button
          >{/if}
      </div>

      {#if page === 'Campaigns'}
        {#if farm.campaignNotice}
          <div class="preview-banner" role="status">
            <span><CircleHelp size={16} />{farm.campaignNotice}</span>
            <button
              onclick={() => openLink('https://www.twitch.tv/drops/campaigns')}
            >
              Twitch campaigns <ArrowUpRight size={14} />
            </button>
          </div>
        {/if}
        <section class="hero" class:connected={!!farm.account || preview}>
          <div class="hero-copy">
            <div class="hero-kicker">
              <span class="status-dot" class:live={farm.running}
              ></span>{farm.running
                ? 'RUNNING'
                : farm.account || preview
                  ? 'STOPPED'
                  : 'NOT CONNECTED'}
            </div>
            <h2>
              {farm.running
                ? (active?.game ?? 'Checking queue')
                : farm.account || preview
                  ? 'Ready to start'
                  : 'Connect Twitch'}
            </h2>
            <p>
              {farm.account || preview
                ? farm.running
                  ? farm.status
                  : 'Add campaigns to the queue, then start farming.'
                : 'Sign in to load your campaigns and drop progress.'}
            </p>
            <div class="hero-actions">
              {#if farm.account || preview}<button
                  class="primary"
                  disabled={!!busy || (!farm.running && !farm.queue.length)}
                  onclick={toggleRunning}
                  >{#if farm.running}<Pause size={16} />{:else}<Play
                      size={16}
                      fill="currentColor"
                    />{/if}{farm.running
                    ? 'Pause farming'
                    : 'Start farming'}</button
                ><span>{farm.queue.length} campaigns in queue</span
                >{:else}<button
                  class="primary"
                  disabled={!!busy || !ready}
                  onclick={connect}
                  ><Link2 size={16} />Connect Twitch<ArrowUpRight
                    size={15}
                  /></button
                ><button class="text-button" onclick={showPreview}
                  >Preview <ChevronRight size={14} /></button
                >{/if}
            </div>
          </div>
        </section>
        <div class="stats">
          <div>
            <span class="stat-icon"><LayoutGrid size={19} /></span>
            <div>
              <strong
                >{farm.campaigns
                  .filter((c) => campaignState(c) === 'Available')
                  .length.toString()
                  .padStart(2, '0')}</strong
              ><span>Available campaigns</span>
            </div>
          </div>
          <div>
            <span class="stat-icon"><ListOrdered size={19} /></span>
            <div>
              <strong>{farm.queue.length.toString().padStart(2, '0')}</strong
              ><span>In your queue</span>
            </div>
          </div>
          <div>
            <span class="stat-icon"><Gift size={19} /></span>
            <div>
              <strong
                >{rewards
                  .filter((d) => d.claimed)
                  .length.toString()
                  .padStart(2, '0')}</strong
              ><span>Drops claimed</span>
            </div>
          </div>
        </div>
        <div class="section-heading">
          <div>
            <h2>Campaigns <span>{visible.length}</span></h2>
          </div>
          <div class="search">
            <Search size={16} /><input
              aria-label="Search campaigns"
              placeholder="Game, reward, or streamer"
              bind:value={search}
            />
          </div>
        </div>
        <div class="filters">
          {#each ['All campaigns', 'Linked accounts', 'Ending soon'] as name}<button
              class:chosen={filter === name}
              onclick={() => (filter = name)}>{name}</button
            >{/each}
        </div>
        {#if visible.length}<div class="campaign-grid">
            {#each visible as c, i (c.id)}<article class="campaign-card">
                <button
                  class="poster tone-{i % 6}"
                  onclick={() => (selected = c)}
                  aria-label={`View ${c.game} rewards`}
                  >{#if c.image}<img
                      src={c.image}
                      alt=""
                      loading="lazy"
                      referrerpolicy="no-referrer"
                    />{:else}<div class="poster-lines"></div>
                    <div class="poster-symbol">
                      <Gift size={38} strokeWidth={1} />
                    </div>
                    <span class="poster-title">{c.game}</span>{/if}<span
                    class="deadline"
                    ><Clock size={11} />{deadline(c.endsAt)}</span
                  >{#if farm.activeCampaign === c.id}<span class="farming-badge"
                      ><Radio size={12} />Farming</span
                    >{/if}</button
                >
                <div class="card-body">
                  <div class="card-game">
                    <h3>{c.game}</h3>
                    <span
                      class="link-status"
                      class:unlinked={!c.linked}
                      title={c.linked
                        ? 'Account linked'
                        : 'Link your game account'}><Link2 size={13} /></span
                    >
                  </div>
                  <button class="campaign-name" onclick={() => (selected = c)}
                    >{c.name}</button
                  >
                  <div class="reward-meta">
                    <span><Gift size={13} />{c.drops.length} rewards</span><span
                      >{campaignState(c) === 'Available'
                        ? `${c.drops.filter((d) => d.claimed).length} claimed`
                        : campaignState(c)}</span
                    >
                  </div>
                  <div class="progress-track">
                    <span style:width={`${progress(c)}%`}></span>
                  </div>
                  <div class="card-footer">
                    <span>{progress(c)}% complete</span><button
                      class:queued={farm.queue.includes(c.id)}
                      class="queue-button"
                      disabled={!!busy ||
                        (!farm.queue.includes(c.id) && !canQueue(c))}
                      title={!c.linked
                        ? 'Link your game account before farming.'
                        : undefined}
                      onclick={() => toggleQueue(c)}
                      >{#if farm.queue.includes(c.id)}<Check
                          size={14}
                        />Queued{:else}<Plus size={14} />Add to queue{/if}</button
                    >
                  </div>
                </div>
              </article>{/each}
          </div>{:else}<div class="empty-state">
            <span class="empty-icon"><Gift size={30} /></span>
            <h3>
              {farm.account ? 'No campaigns found' : 'Twitch not connected'}
            </h3>
            <p>
              {farm.account
                ? 'Refresh your campaigns or try another search.'
                : 'Connect Twitch to load campaigns.'}
            </p>
            {#if !farm.account}<button
                class="secondary"
                disabled={!!busy || !ready}
                onclick={connect}
                >Connect Twitch <ArrowUpRight size={14} /></button
              >{:else}<button
                class="text-button"
                onclick={() => {
                  search = '';
                  filter = 'All campaigns';
                }}>Clear filters</button
              >{/if}
          </div>{/if}
      {:else if page === 'My queue'}
        <div class="queue-toolbar">
          <div>
            <span class="status-dot" class:live={farm.running}
            ></span>{farm.status}
          </div>
          <button
            class="primary"
            disabled={!!busy || !farm.queue.length}
            onclick={toggleRunning}
            >{#if farm.running}<Pause size={15} />{:else}<Play
                size={15}
              />{/if}{farm.running ? 'Pause farming' : 'Start farming'}</button
          >
        </div>
        {#each farm.queue as id, i (id)}{@const c = farm.campaigns.find(
            (c) => c.id === id,
          )}
          <div class="queue-row">
            <span class="queue-number">{String(i + 1).padStart(2, '0')}</span
            ><span class="queue-glyph tone-{i % 6}"><Gift size={24} /></span>
            <div class="queue-info">
              <h3>{c?.game ?? 'Campaign unavailable'}</h3>
              <p>{c?.name ?? 'Refresh campaigns or remove this entry.'}</p>
              {#if c && !c.linked}
                <p>
                  Waiting for account link · <button
                    class="text-button"
                    onclick={() =>
                      openLink('https://www.twitch.tv/drops/campaigns')}
                    >Link on Twitch <ArrowUpRight size={12} /></button
                  >
                </p>
              {/if}
              {#if c}<div class="progress-track">
                  <span style:width={`${progress(c)}%`}></span>
                </div>{/if}
            </div>
            <div class="queue-detail">
              {#if c}<strong>{progress(c)}%</strong><span
                  >{deadline(c.endsAt)}</span
                >{/if}
            </div>
            <div class="row-actions">
              <button
                class="icon-button"
                aria-label={`Move ${c?.game ?? 'campaign'} up`}
                disabled={!!busy || i === 0}
                onclick={() => changeQueue(moveQueue(farm.queue, id, -1))}
                ><ArrowUp size={16} /></button
              ><button
                class="icon-button"
                aria-label={`Move ${c?.game ?? 'campaign'} down`}
                disabled={!!busy || i === farm.queue.length - 1}
                onclick={() => changeQueue(moveQueue(farm.queue, id, 1))}
                ><ArrowDown size={16} /></button
              ><button
                class="icon-button"
                aria-label={`Remove ${c?.game ?? 'campaign'} from queue`}
                disabled={!!busy}
                onclick={() => changeQueue(farm.queue.filter((q) => q !== id))}
                ><X size={16} /></button
              >
            </div>
          </div>{/each}
        {#if !queued.length}<div class="empty-state">
            <ListOrdered size={32} />
            <h3>Queue empty</h3>
            <p>Add campaigns to start farming.</p>
            <button class="secondary" onclick={() => (page = 'Campaigns')}
              >Browse campaigns <ArrowUpRight size={14} /></button
            >
          </div>{/if}
        <div class="info-note">
          <CircleHelp size={16} /><span
            >Move campaigns to set priority. Offline campaigns are skipped and
            checked again every minute.</span
          >
        </div>
      {:else if page === 'Inventory'}
        <div class="section-heading">
          <h2>Rewards <span>{rewards.length}</span></h2>
          <span class="muted"
            >{rewards.filter((d) => d.claimed).length} claimed · {rewards.filter(
              (d) => !d.claimed && d.minutes >= d.required && d.required > 0,
            ).length} ready to claim</span
          >
        </div>
        <div class="reward-list">
          {#each rewards as d (`${d.campaignId}:${d.id}`)}<div
              class="reward-row"
            >
              <span class="reward-image"
                >{#if d.image}<img
                    src={d.image}
                    alt=""
                    loading="lazy"
                  />{:else}<Gift size={24} />{/if}</span
              >
              <div>
                <h3>{d.name}</h3>
                <p>{d.game}</p>
              </div>
              <div class="reward-progress">
                <span>{duration(d.minutes)} / {duration(d.required)}</span>
                <div class="progress-track">
                  <span
                    style:width={`${d.required ? Math.min(100, (d.minutes / d.required) * 100) : 0}%`}
                  ></span>
                </div>
              </div>
              {#if d.claimed}<span class="claimed"
                  ><Check size={14} />Claimed</span
                >{:else if d.required > 0 && d.minutes >= d.required && d.claimId}<button
                  class="secondary"
                  disabled={!!busy}
                  onclick={() => claim(d.campaignId, d.id)}>Claim reward</button
                >{:else}<span class="muted reward-status">In progress</span
                >{/if}
            </div>{/each}
        </div>
        {#if !rewards.length}<div class="empty-state">
            <Gift size={32} />
            <h3>No rewards</h3>
            <p>Refresh to load your Twitch drops.</p>
          </div>{/if}
      {:else if page === 'Activity'}
        <div class="activity-list">
          {#each farm.logs as log}<div class="activity-row">
              <span class="log-dot {log.level}"></span><time
                >{new Date(log.time).toLocaleTimeString([], {
                  hour: '2-digit',
                  minute: '2-digit',
                  second: '2-digit',
                })}</time
              >
              <p>{log.message}</p>
            </div>{/each}
        </div>
        {#if !farm.logs.length}<div class="empty-state">
            <Activity size={32} />
            <h3>No activity yet</h3>
          </div>{/if}
      {:else}
        <section class="settings-panel">
          <div class="setting-row">
            <div>
              <h3>Automatically claim rewards</h3>
              <p>Claim drops when their watch time is complete.</p>
            </div>
            <input
              type="checkbox"
              role="switch"
              aria-label="Automatically claim rewards"
              checked={farm.autoClaim}
              disabled={!!busy}
              onchange={(e) => autoClaim(e.currentTarget.checked)}
            />
          </div>
          <div class="setting-row">
            <div>
              <h3>Twitch connection</h3>
              <p>
                {farm.account
                  ? `Connected as ${farm.account.login} · ${farm.loginMethod === 'browser' ? 'Browser sign-in' : 'Sign-in code'}`
                  : 'Sign in with a browser or a one-time code.'}
              </p>
            </div>
            {#if !farm.account}<button
                class="secondary"
                onclick={connect}
                disabled={!!busy || !ready}
                >Connect Twitch<ArrowUpRight size={14} /></button
              >{:else}<div class="connection-actions">
                <button class="secondary" disabled={!!busy} onclick={connect}
                  >Change sign-in</button
                ><button
                  class="secondary"
                  disabled={!!busy}
                  onclick={() =>
                    action('Disconnecting', () => command('logout'))}
                  >Disconnect<LogOut size={14} /></button
                >
              </div>{/if}
          </div>
          <div class="setting-row">
            <div>
              <h3>Linked game accounts</h3>
              <p>Link game accounts on Twitch to receive drops.</p>
            </div>
            <button
              class="secondary"
              onclick={() => openLink('https://www.twitch.tv/drops/campaigns')}
              >Manage on Twitch<ArrowUpRight size={14} /></button
            >
          </div>
          <UpdatePanel {updater} {desktop} disabled={!!busy || connecting} />
        </section>
        <div class="settings-about">
          <ShieldCheck size={22} />
          <div>
            <h3>Storage and farming</h3>
            <p>
              Login credentials and the queue are saved on this computer.
              Closing the app stops farming.
            </p>
            <p>Progress comes from Twitch. Check Activity for errors.</p>
          </div>
        </div>
      {/if}
      <footer>
        <span
          >{busy
            ? busy + '…'
            : refreshing
              ? farm.campaignsCached
                ? 'Updating cached campaigns…'
                : 'Refreshing campaigns…'
              : farm.lastSync
                ? `${farm.campaignsCached ? 'Cached' : 'Last synced'} ${new Date(farm.lastSync).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
                : 'Not synced'}</span
        >
      </footer>
    </div>
  </main>
</div>

{#if loginOpen}<div class="modal-backdrop">
    <dialog
      use:openModal
      oncancel={cancelLogin}
      class="modal"
      aria-modal="true"
      aria-labelledby="login-title"
      tabindex="-1"
    >
      <button
        class="modal-close icon-button"
        aria-label="Close sign-in"
        onclick={cancelLogin}><X size={20} /></button
      ><span class="empty-icon"><Link2 size={28} /></span>
      <h2 id="login-title">Connect Twitch</h2>
      {#if loginError}<div class="error-banner" role="alert">
          {loginError}
        </div>{/if}
      {#if loginMode === 'choice'}
        <p>Choose a sign-in method.</p>
        <div class="login-options">
          <button
            class="login-option"
            disabled={!!busy}
            onclick={() => startLogin('browser')}
          >
            <ArrowUpRight size={20} /><span
              ><strong>Sign in with browser</strong><small
                >Chrome or Edge · verifies full campaign access</small
              ></span
            >
          </button>
          <button
            class="login-option"
            disabled={!!busy}
            onclick={() => startLogin('code')}
          >
            <Link2 size={20} /><span
              ><strong>Use a sign-in code</strong><small
                >May only show campaigns already in progress</small
              ></span
            >
          </button>
        </div>
      {:else if loginMode === 'browser'}
        <p>
          Sign in on Twitch in the window that opens. Complete any verification
          there and keep it open until this app confirms the connection.
        </p>
        <p class="modal-note">
          <RefreshCw class="spin" size={13} />Waiting for browser sign-in
        </p>
        <button class="secondary full-width login-cancel" onclick={cancelLogin}
          >Cancel sign-in</button
        >
      {:else if login}
        <p>
          Open Twitch, sign in, and enter this one-time code to connect your
          account.
        </p>
        <div class="device-code">{login.userCode}</div>
        <button
          class="primary full-width"
          onclick={() => openLink(login!.verificationUri)}
          >Open Twitch activation <ArrowUpRight size={16} /></button
        >
        <p class="modal-note">
          <RefreshCw class="spin" size={13} />Waiting for Twitch authorization ·
          code expires in {Math.ceil(login.expiresIn / 60)} minutes
        </p>
      {:else}<p class="modal-note">
          <RefreshCw class="spin" size={13} />Requesting sign-in code
        </p>{/if}
    </dialog>
  </div>{/if}
{#if selectedLive}{@const c = selectedLive}
  <div class="modal-backdrop">
    <dialog
      use:openModal
      oncancel={() => (selected = null)}
      class="modal details-modal"
      aria-modal="true"
      aria-labelledby="campaign-title"
      tabindex="-1"
    >
      <button
        class="modal-close icon-button"
        aria-label="Close campaign"
        onclick={() => (selected = null)}><X size={20} /></button
      >
      <div class="eyebrow">{c.game}</div>
      <h2 id="campaign-title">{c.name}</h2>
      <p>
        {deadline(c.endsAt)} · {c.channels.length
          ? `${c.channels.length} eligible channels`
          : 'Any eligible channel in this game'}
      </p>
      {#each c.drops as d (d.id)}<div class="detail-drop">
          <Gift size={20} />
          <div>
            <h3>{d.name}</h3>
            <p>
              {duration(d.minutes)} of {duration(d.required)} · {d.claimed
                ? 'Claimed'
                : d.prerequisites.length
                  ? 'Requires earlier drops'
                  : 'Watch reward'}
            </p>
            <div class="progress-track">
              <span
                style:width={`${d.required ? Math.min(100, (d.minutes / d.required) * 100) : 0}%`}
              ></span>
            </div>
          </div>
          {#if d.claimed}<Check size={18} />{/if}
        </div>{/each}
      <div class="modal-actions">
        <button
          class="primary"
          disabled={!!busy || (!farm.queue.includes(c.id) && !canQueue(c))}
          onclick={() => toggleQueue(c)}
          >{farm.queue.includes(c.id)
            ? 'Remove from queue'
            : 'Add to queue'}</button
        >{#if !c.linked}<button
            class="secondary"
            onclick={() => openLink('https://www.twitch.tv/drops/campaigns')}
            >Link game account<ArrowUpRight size={15} /></button
          >{/if}<button
          class="secondary"
          onclick={() => openLink('https://www.twitch.tv/drops/inventory')}
          >Twitch inventory<ArrowUpRight size={15} /></button
        >
      </div>
    </dialog>
  </div>{/if}
