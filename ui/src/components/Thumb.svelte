<script lang="ts">
  import Icon from './Icon.svelte';
  import { fileUrl } from '../lib/dom';

  // A square picture: an instance, mod or world icon, or a placeholder icon.
  let {
    src = null,
    url = null,
    size,
    fallback = 'box',
    running = false,
    radius = 0.24,
    edge = false,
    dim = false,
  }: {
    /** Local file. */
    src?: string | null;
    /** Remote image. */
    url?: string | null;
    size: number;
    fallback?: string;
    running?: boolean;
    radius?: number;
    edge?: boolean;
    dim?: boolean;
  } = $props();

  let failed = $state(false);
  let image = $derived(src ? fileUrl(src) : url);
  $effect(() => {
    image;
    failed = false;
  });
</script>

<span
  class="thumb"
  class:edge
  class:running
  class:dim
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{size * radius}px"
>
  {#if image && !failed}
    <img src={image} alt="" loading="lazy" draggable="false" onerror={() => (failed = true)} />
  {:else}
    <Icon name={fallback} size={Math.round(size * 0.44)} />
  {/if}
</span>

<style>
  .thumb {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background: var(--tile);
    color: var(--subtle);
    box-sizing: border-box;
  }
  .thumb.edge {
    border: 1px solid var(--tile-edge);
  }
  .thumb.running {
    border: 2px solid var(--success);
    color: var(--success);
  }
  .thumb.dim {
    opacity: 0.45;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    image-rendering: pixelated;
  }
</style>
