<script lang="ts">
  let {
    on,
    title,
    onchange,
    size = 'sm',
  }: { on: boolean; title?: string; onchange: (on: boolean) => void; size?: 'sm' | 'md' } = $props();
</script>

<button
  class="switch {size}"
  class:on
  role="switch"
  aria-checked={on}
  aria-label={title}
  {title}
  onclick={(e) => {
    e.stopPropagation();
    onchange(!on);
  }}
>
  <span class="knob"></span>
</button>

<style>
  .switch {
    flex: none;
    width: 32px;
    height: 18px;
    padding: 2px;
    border-radius: 999px;
    display: flex;
    align-items: center;
    background: color-mix(in srgb, var(--subtle) 45%, transparent);
    cursor: pointer;
  }
  .switch.md {
    width: 36px;
    height: 20px;
  }
  .switch.on {
    justify-content: flex-end;
    background: var(--accent);
  }
  .knob {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.25);
  }
  .md .knob {
    width: 16px;
    height: 16px;
  }
</style>
