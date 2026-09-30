<script lang="ts">
  import Icon from './Icon.svelte';

  let {
    icon,
    title,
    disabled = false,
    onclick,
  }: { icon: string; title: string; disabled?: boolean; onclick?: (e: MouseEvent) => void } = $props();
</script>

<button class="icon-btn" {title} aria-label={title} {disabled} onclick={onclick}>
  <Icon name={icon} size={15} />
</button>

<style>
  .icon-btn {
    flex: none;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    color: var(--muted);
    cursor: pointer;
  }
  .icon-btn:not(:disabled):hover {
    background: var(--hover);
  }
  .icon-btn:disabled {
    cursor: default;
    color: color-mix(in srgb, var(--subtle) 60%, transparent);
  }
</style>
