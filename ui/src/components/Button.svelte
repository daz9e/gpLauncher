<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  type Style = 'primary' | 'play' | 'stop' | 'secondary' | 'ghost';
  let {
    label = '',
    icon,
    variant = 'secondary',
    disabled = false,
    title,
    onclick,
    class: className = '',
    children,
  }: {
    label?: string;
    icon?: string;
    variant?: Style;
    disabled?: boolean;
    title?: string;
    onclick?: (e: MouseEvent) => void;
    class?: string;
    children?: Snippet;
  } = $props();
</script>

<button class="btn {variant} {className}" {disabled} {title} aria-label={label || title} onclick={onclick}>
  {#if icon}<Icon name={icon} size={14} />{/if}
  {#if label}<span>{label}</span>{/if}
  {@render children?.()}
</button>

<style>
  .btn {
    flex: none;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 0 12px;
    border-radius: 6px;
    border: 1px solid transparent;
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
  }
  .btn:disabled {
    cursor: default;
    color: var(--subtle);
    background: transparent;
    border-color: var(--border);
  }
  .btn.ghost:disabled {
    border-color: transparent;
  }
  .primary:not(:disabled) {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .play:not(:disabled) {
    background: var(--success);
    border-color: var(--success);
    color: var(--on-accent);
  }
  .stop:not(:disabled) {
    background: var(--danger);
    border-color: var(--danger);
    color: var(--on-accent);
  }
  .primary:not(:disabled):hover,
  .play:not(:disabled):hover,
  .stop:not(:disabled):hover {
    opacity: 0.9;
  }
  .secondary:not(:disabled) {
    background: var(--bg);
    border-color: var(--border);
    color: var(--text);
  }
  .ghost:not(:disabled) {
    color: var(--text);
  }
  .secondary:not(:disabled):hover,
  .ghost:not(:disabled):hover {
    background: var(--hover);
  }
</style>
