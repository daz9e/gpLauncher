<script lang="ts" generics="T">
  // A segmented control: one option of a few is active.
  let {
    options,
    value,
    onchange,
    disabled = false,
    small = false,
    label,
  }: {
    options: { value: T; label: string }[];
    value: T;
    onchange: (value: T) => void;
    disabled?: boolean;
    small?: boolean;
    label?: string;
  } = $props();
</script>

<div class="segments" class:small role="radiogroup" aria-label={label}>
  {#each options as option (option.label)}
    <button
      class="segment"
      class:active={option.value === value}
      role="radio"
      aria-checked={option.value === value}
      {disabled}
      onclick={() => option.value !== value && onchange(option.value)}>{option.label}</button
    >
  {/each}
</div>

<style>
  .segments {
    display: flex;
    flex: none;
    padding: 2px;
    gap: 2px;
    border-radius: 6px;
    background: var(--tile);
  }
  .segment {
    flex: 1;
    display: flex;
    justify-content: center;
    padding: 3px 10px;
    border-radius: 5px;
    border: 1px solid transparent;
    font-size: 13px;
    color: var(--muted);
    cursor: pointer;
    white-space: nowrap;
  }
  .small .segment {
    font-size: 12px;
    padding: 2px 10px;
  }
  .segment:not(.active):not(:disabled):hover {
    color: var(--text);
  }
  .segment.active {
    background: var(--bg);
    color: var(--text);
    font-weight: 500;
    border-color: var(--border);
  }
  .segment:disabled {
    cursor: default;
  }
</style>
