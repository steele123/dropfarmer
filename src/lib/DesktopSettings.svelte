<script lang="ts">
  import type { NotificationSettings } from './model';
  let {
    trayEnabled,
    notifications,
    disabled,
    onupdate,
    ontest,
  }: {
    trayEnabled: boolean;
    notifications: NotificationSettings;
    disabled: boolean;
    onupdate: (
      tray: boolean,
      notifications: NotificationSettings,
    ) => Promise<void>;
    ontest: () => Promise<void>;
  } = $props();
  const options = [
    { key: 'rewards', label: 'Drops claimed' },
    { key: 'queue', label: 'Queue finished' },
    { key: 'reconnect', label: 'Twitch needs reconnecting' },
  ] as const;
  let testSent = $state(false);
</script>

<div class="setting-row">
  <div>
    <h3>Minimize to tray</h3>
    <p>
      Closing or minimizing the window keeps farming. Use Quit Dropfarmer in the
      tray menu to exit.
    </p>
  </div>
  <input
    type="checkbox"
    role="switch"
    aria-label="Minimize to tray"
    checked={trayEnabled}
    {disabled}
    onchange={(e) => {
      const checked = e.currentTarget.checked;
      e.currentTarget.checked = trayEnabled;
      void onupdate(checked, notifications);
    }}
  />
</div>
<div class="setting-row notification-setting">
  <div>
    <h3>Desktop notifications</h3>
    <p>Choose what you want to hear about, even while the window is hidden.</p>
    <div class="notification-options">
      {#each options as option}<label
          ><input
            type="checkbox"
            checked={notifications[option.key]}
            {disabled}
            onchange={(e) => {
              const checked = e.currentTarget.checked;
              e.currentTarget.checked = notifications[option.key];
              void onupdate(trayEnabled, {
                ...notifications,
                [option.key]: checked,
              });
            }}
          />{option.label}</label
        >{/each}
    </div>
    {#if testSent}<p role="status">Test notification sent.</p>{/if}
  </div>
  <button
    class="secondary"
    disabled={disabled || !Object.values(notifications).some(Boolean)}
    onclick={async () => {
      testSent = false;
      try {
        await ontest();
        testSent = true;
      } catch {
        /* Parent displays the error. */
      }
    }}>Test notification</button
  >
</div>

<style>
  .setting-row > div {
    min-width: 0;
  }
  .setting-row p {
    max-width: 620px;
  }
  .notification-setting {
    align-items: flex-start;
  }
  .notification-options {
    display: flex;
    flex-wrap: wrap;
    gap: 14px 20px;
    margin-top: 18px;
  }
  .notification-options label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: #ccc;
    cursor: pointer;
  }
  .notification-options input {
    appearance: auto;
    width: 14px;
    height: 14px;
    margin: 0;
    border-radius: 3px;
    accent-color: #ededed;
    background: initial;
  }
  .notification-options input::before {
    display: none;
  }
  button {
    flex-shrink: 0;
  }
</style>
