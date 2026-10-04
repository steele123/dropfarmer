<script lang="ts">
  import { duration, type Snapshot } from './model';
  import type { queueEstimate } from './queueEstimate';
  let {
    estimate,
    running,
    autoFarm,
    autoClaim,
  }: {
    estimate: ReturnType<typeof queueEstimate>;
    running: boolean;
    autoFarm: boolean;
    autoClaim: Snapshot['autoClaim'];
  } = $props();
  const finish = $derived(
    estimate.finishAt === null
      ? null
      : new Date(estimate.finishAt).toLocaleString([], {
          month: 'short',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
        }),
  );
</script>

<section class="estimate" aria-label="Queue finish estimate">
  <div class="figures">
    <div>
      <span
        >{estimate.partial ? 'Known watch time left' : 'Watch time left'}</span
      ><strong
        >{estimate.partial && !estimate.minutes
          ? 'Unknown'
          : `${duration(estimate.minutes)}${estimate.partial ? '+' : ''}`}</strong
      >
    </div>
    <div>
      <span>{running ? 'Estimated finish' : 'If started now'}</span><strong
        >{finish ??
          (estimate.issue ? 'Unavailable' : 'Watch time complete')}</strong
      >
    </div>
  </div>
  {#if estimate.issue}<p>{estimate.issue}</p>{/if}
  {#if !estimate.minutes && !estimate.partial && !estimate.issue}<p>
      Waiting for rewards to be claimed or confirmed by Twitch.
    </p>{/if}
  <p>
    Based on the current queue order. Assumes eligible streams stay live and
    rewards are claimed promptly. Rewards that progress together share watch
    time.
  </p>
  {#if !autoClaim}<p>
      Auto-claim is off. Claim completed rewards to keep the queue moving.
    </p>{/if}
  {#if autoFarm}<p>
      New campaigns added by Auto farm can extend this estimate.
    </p>{/if}
</section>

<style>
  .estimate {
    margin-bottom: 24px;
    padding: 20px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
  }
  .figures {
    display: flex;
    flex-wrap: wrap;
    gap: 18px 56px;
    margin-bottom: 14px;
  }
  .figures div {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .figures span {
    font-size: 12px;
    color: var(--muted);
  }
  strong {
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.5px;
  }
  p {
    font-size: 12px;
    color: var(--muted);
    line-height: 1.6;
    margin-top: 6px;
  }
</style>
