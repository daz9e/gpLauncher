<script lang="ts">
  import Icon from './Icon.svelte';

  // A button that opens a list of options below it.
  let {
    label,
    value,
    options,
    onselect,
    disabled = false,
  }: {
    /** Shown before the value, e.g. `Version`. */
    label: string;
    value: string;
    options: { label: string; selected: boolean }[];
    onselect: (index: number) => void;
    disabled?: boolean;
  } = $props();

  let open = $state(false);
  let root: HTMLDivElement;
  let list = $state<HTMLDivElement>();

  function outside(e: MouseEvent) {
    if (open && !root.contains(e.target as Node)) open = false;
  }

  $effect(() => {
    if (open && list) list.querySelector('.selected')?.scrollIntoView({ block: 'nearest' });
  });
</script>

<svelte:window onmousedown={outside} onkeydown={(e) => open && e.key === 'Escape' && ((open = false), e.stopPropagation())} />

<div class="dropdown" bind:this={root}>
  <button
    class="trigger"
    class:open
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label="{label}: {value}"
    onclick={() => (open = !open)}
  >
    <span class="subtle">{label}</span>
    <span class="value truncate">{value}</span>
    <Icon name="chevron-down" size={12} />
  </button>
  {#if open}
    <div class="menu" role="listbox" bind:this={list}>
      {#each options as option, i (i)}
        <button
          class="option"
          class:selected={option.selected}
          role="option"
          aria-selected={option.selected}
          onclick={() => {
            open = false;
            onselect(i);
          }}
        >
          <span class="truncate grow">{option.label}</span>
          {#if option.selected}<Icon name="check" size={13} />{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dropdown {
    position: relative;
    flex: none;
  }
  .trigger {
    height: 28px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px 0 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-size: 12px;
    cursor: pointer;
    max-width: 260px;
    color: var(--muted);
  }
  .trigger:not(:disabled):hover,
  .trigger.open {
    background: var(--hover);
  }
  .trigger:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .value {
    color: var(--text);
    font-weight: 500;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 50;
    min-width: 100%;
    max-width: 320px;
    max-height: 300px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  .option {
    width: 100%;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: 5px;
    font-size: 13px;
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
  }
  .option:hover {
    background: var(--hover);
  }
  .option.selected {
    color: var(--accent);
    font-weight: 500;
  }
</style>
