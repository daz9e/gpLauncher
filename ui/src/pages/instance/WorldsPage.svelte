<script lang="ts">
  // Worlds in the instance's `saves` folder.
  import { untrack } from 'svelte';
  import { ask } from '@tauri-apps/plugin-dialog';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { actions } from '../../lib/actions';
  import { ago, humanSize } from '../../lib/format';
  import type { World } from '../../lib/types';
  import Button from '../../components/Button.svelte';
  import IconButton from '../../components/IconButton.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import PageToolbar from './PageToolbar.svelte';

  let { id, reload, active }: { id: string; reload: number; active: boolean } = $props();

  let worlds = $state<World[] | null>(null);
  let sizes = $state<Record<string, number>>({});
  let error = $state<string | null>(null);
  let inst = $derived(launcher.instance(id));
  let running = $derived(launcher.running(id));
  const join = (dir: string, name: string) => `${dir}${dir.includes('\\') ? '\\' : '/'}${name}`;

  async function load() {
    try {
      const list = await api.worlds(id);
      worlds = list;
      error = null;
      for (const w of list) api.dirSize(w.path).then((s) => (sizes[w.path] = s));
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    reload;
    if (active) untrack(load);
  });

  async function remove(w: World) {
    const yes = await ask('The world folder is deleted permanently. This can not be undone.', {
      title: `Delete the world "${w.name}"?`,
      kind: 'warning',
      okLabel: 'Delete',
      cancelLabel: 'Cancel',
    });
    if (!yes) return;
    try {
      await api.deleteWorld(id, w.path);
    } catch (e) {
      error = message(e);
    }
    load();
  }

  function detail(w: World): string {
    const size = sizes[w.path];
    return [w.game_mode, `Played ${ago(w.last_played, launcher.now).toLowerCase()}`, size !== undefined ? humanSize(size) : null]
      .filter(Boolean)
      .join(' · ');
  }
</script>

<PageToolbar title="Worlds" detail={worlds?.length ? String(worlds.length) : ''}>
  {#if inst}
    <Button icon="folder" label="Open folder" variant="ghost" onclick={() => actions.openFolder(join(inst!.game_dir, 'saves'))} />
  {/if}
</PageToolbar>
{#if error}<div class="error pad">{error}</div>{/if}
<div class="body">
  {#if worlds && worlds.length === 0}
    <EmptyState
      icon="earth"
      title="No worlds yet"
      detail="Worlds you create in the game appear here. Drop a world folder into the saves folder to add one."
    />
  {:else if worlds}
    {#each worlds as w (w.path)}
      <div class="world">
        <Thumb src={w.icon} size={44} radius={0.14} fallback="earth" />
        <div class="grow col">
          <span class="sm medium truncate">{w.name}</span>
          <span class="xs muted truncate">{detail(w)}</span>
          {#if w.name !== w.folder}<span class="xs subtle truncate">{w.folder}</span>{/if}
        </div>
        <div class="actions">
          <IconButton icon="folder" title="Open folder" onclick={() => api.openFolder(w.path)} />
          <IconButton
            icon="trash"
            title={running ? 'Close the game to delete worlds' : 'Delete'}
            disabled={running}
            onclick={() => remove(w)}
          />
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .pad {
    padding: 8px 16px;
  }
  .world {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
  }
  .world:hover {
    background: var(--hover);
  }
  .actions {
    display: flex;
    gap: 2px;
    visibility: hidden;
  }
  .world:hover .actions {
    visibility: visible;
  }
</style>
