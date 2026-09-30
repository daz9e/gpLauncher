<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children, onclose }: { children: Snippet; onclose: () => void } = $props();
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="overlay"
  role="presentation"
  onmousedown={(e) => e.target === e.currentTarget && onclose()}
>
  {@render children()}
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgb(0 0 0 / 0.35);
  }
</style>
