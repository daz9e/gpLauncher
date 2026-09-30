<script lang="ts">
  // Screenshots the game saved, newest first.
  import { untrack } from 'svelte';
  import { launcher } from '../../lib/store.svelte';
  import { api } from '../../lib/api';
  import { actions } from '../../lib/actions';
  import { fileStem } from '../../lib/format';
  import { fileUrl } from '../../lib/dom';
  import Button from '../../components/Button.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import PageToolbar from './PageToolbar.svelte';

  let { id, reload, active }: { id: string; reload: number; active: boolean } = $props();

  let shots = $state<string[] | null>(null);
  let inst = $derived(launcher.instance(id));
  const join = (dir: string, name: string) => `${dir}${dir.includes('\\') ? '\\' : '/'}${name}`;

  $effect(() => {
    reload;
    if (active) untrack(() => api.screenshots(id).then((list) => (shots = list)));
  });
</script>

<PageToolbar title="Screenshots" detail={shots?.length ? String(shots.length) : ''}>
  {#if inst}
    <Button
      icon="folder"
      label="Open folder"
      variant="ghost"
      onclick={() => actions.openFolder(join(inst!.game_dir, 'screenshots'))}
    />
  {/if}
</PageToolbar>
<div class="body">
  {#if shots && shots.length === 0}
    <EmptyState icon="image" title="No screenshots yet" detail="Press F2 in the game to take one." />
  {:else if shots}
    <div class="grid">
      {#each shots as path (path)}
        <button class="shot" title="Open" onclick={() => api.openFile(path)}>
          <span class="frame"><img src={fileUrl(path)} alt="" loading="lazy" draggable="false" /></span>
          <span class="xs muted truncate">{fileStem(path)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
  }
  .grid {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .shot {
    width: 200px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    text-align: left;
    cursor: pointer;
  }
  .frame {
    width: 200px;
    height: 112px;
    border-radius: 6px;
    overflow: hidden;
    border: 1px solid var(--border);
    background: var(--tile);
  }
  .shot:hover .frame {
    border-color: var(--accent);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
