<script lang="ts">
  let {
    enabled,
    disabled,
    supported,
    onchange,
  }: {
    enabled: boolean;
    disabled: boolean;
    supported: boolean;
    onchange: (value: boolean) => Promise<void>;
  } = $props();
</script>

<div class="setting-row">
  <div>
    <h3>Sleep PC when queue finishes</h3>
    <p>
      {supported
        ? 'This run only. After every reward is claimed, a 60-second countdown lets you cancel. Pausing or adding or removing campaigns turns this off.'
        : 'Available in the Windows desktop app.'}
    </p>
  </div>
  <input
    type="checkbox"
    role="switch"
    aria-label="Sleep PC when queue finishes"
    checked={enabled}
    disabled={!enabled && (disabled || !supported)}
    onchange={(e) => {
      const value = e.currentTarget.checked;
      e.currentTarget.checked = enabled;
      void onchange(value);
    }}
  />
</div>
