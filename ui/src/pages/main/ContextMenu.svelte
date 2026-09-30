<script lang="ts">
  // Right-click menu of an instance tile.
  import { launcher } from '../../lib/store.svelte';
  import { actions } from '../../lib/actions';
  import type { Instance } from '../../lib/types';
  import Icon from '../../components/Icon.svelte';

  let { inst, x, y, onclose }: { inst: Instance; x: number; y: number; onclose: () => void } = $props();

  let running = $derived(launcher.running(inst.id));
  let el = $state<HTMLDivElement>();
  let pos = $state({ left: 0, top: 0 });

  // Keep the menu inside the window.
  $effect(() => {
    if (!el) return;
    const margin = 8;
    const { width, height } = el.getBoundingClientRect();
    pos = {
      left: Math.max(margin, Math.min(x, window.innerWidth - width - margin)),
      top: Math.max(margin, Math.min(y, window.innerHeight - height - margin)),
    };
  });

  /** Closes the menu, then acts on its instance (the prop is gone once the menu closes). */
  function run(f: (inst: Instance) => unknown) {
    const target = inst;
    onclose();
    f(target);
  }
</script>

<svelte:window
  onmousedown={(e) => !el?.contains(e.target as Node) && onclose()}
  onkeydown={(e) => e.key === 'Escape' && onclose()}
  onblur={onclose}
/>

<div class="menu" role="menu" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px">
  {#if running}
    <button class="item danger" role="menuitem" onclick={() => run((i) => actions.kill(i.id))}>
      <Icon name="stop" size={15} />Stop
    </button>
  {:else}
    <button class="item" role="menuitem" onclick={() => run((i) => actions.launch(i.id))}>
      <Icon name="play" size={15} />Play
    </button>
  {/if}
  <button class="item" role="menuitem" onclick={() => run((i) => actions.open(i.id, 'mods'))}>
    <Icon name="puzzle" size={15} />Mods
  </button>
  <button class="item" role="menuitem" onclick={() => run((i) => actions.open(i.id, 'console'))}>
    <Icon name="terminal" size={15} />Console
  </button>
  <button class="item" role="menuitem" onclick={() => run((i) => actions.open(i.id, 'settings'))}>
    <Icon name="settings" size={15} />Settings
  </button>
  <div class="sep"></div>
  <button class="item" role="menuitem" onclick={() => run((i) => actions.openFolder(i.game_dir))}>
    <Icon name="folder" size={15} />Open folder
  </button>
  <button class="item" role="menuitem" onclick={() => run((i) => actions.duplicate(i.id))}>
    <Icon name="copy" size={15} />Duplicate
  </button>
  <button class="item" role="menuitem" onclick={() => run((i) => actions.export(i.id))}>
    <Icon name="share" size={15} />Export…
  </button>
  <div class="sep"></div>
  <button class="item danger" role="menuitem" disabled={running} onclick={() => run((i) => actions.delete(i))}>
    <Icon name="trash" size={15} />Delete
  </button>
</div>

<style>
  .menu {
    position: fixed;
    z-index: 90;
    width: 200px;
    padding: 4px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  .item {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px;
    border-radius: 6px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    text-align: left;
  }
  .item :global(.icon) {
    color: var(--muted);
  }
  .item:not(:disabled):hover {
    background: var(--hover);
  }
  .item.danger:not(:disabled),
  .item.danger:not(:disabled) :global(.icon) {
    color: var(--danger);
  }
  .item:disabled {
    color: var(--subtle);
    cursor: default;
  }
  .item:disabled :global(.icon) {
    color: var(--subtle);
  }
  .sep {
    height: 1px;
    margin: 4px 0;
    background: var(--border);
  }
</style>
