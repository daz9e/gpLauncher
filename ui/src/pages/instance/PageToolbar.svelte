<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, detail = '', children }: { title: string; detail?: string; children?: Snippet } = $props();
</script>

<div class="toolbar">
  <span class="sm medium">{title}</span>
  <span class="xs subtle">{detail}</span>
  <span class="grow"></span>
  {@render children?.()}
</div>

<style>
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 46px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
  }
</style>
