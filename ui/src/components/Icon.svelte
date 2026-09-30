<script lang="ts">
  import { icons } from '../lib/icons';

  let { name, size = 14, class: className = '' }: { name: string; size?: number; class?: string } = $props();
</script>

<span class="icon {className}" style:width="{size}px" style:height="{size}px" aria-hidden="true">
  {@html icons[name] ?? ''}
</span>

<style>
  .icon {
    display: inline-flex;
    flex: none;
  }
  .icon :global(svg) {
    width: 100%;
    height: 100%;
  }
</style>
