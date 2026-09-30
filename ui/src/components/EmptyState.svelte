<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { icon, title, detail = '', children }: { icon: string; title: string; detail?: string; children?: Snippet } =
    $props();
</script>

<div class="empty">
  <div class="circle"><Icon name={icon} size={22} /></div>
  <div class="title medium">{title}</div>
  {#if detail}<div class="detail sm muted">{detail}</div>{/if}
  {@render children?.()}
</div>

<style>
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
  }
  .circle {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--tile);
    color: var(--subtle);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .title {
    margin-top: 4px;
  }
  .detail {
    max-width: 360px;
  }
</style>
