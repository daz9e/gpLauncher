<script lang="ts">
  // Logs and crash reports the game wrote, with the selected one in a console.
  import { untrack } from 'svelte';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { ago } from '../../lib/format';
  import type { LogFile, LogLine } from '../../lib/types';
  import Icon from '../../components/Icon.svelte';
  import IconButton from '../../components/IconButton.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import LogView from './LogView.svelte';

  let { id, reload, active }: { id: string; reload: number; active: boolean } = $props();

  let files = $state<LogFile[] | null>(null);
  let selected = $state<string | null>(null);
  let lines = $state<LogLine[]>([]);
  let error = $state<string | null>(null);

  async function load() {
    const list = await api.logFiles(id);
    files = list;
    if (!selected && list.length) open(list[0].path);
  }

  async function open(path: string) {
    selected = path;
    error = null;
    try {
      lines = await api.readLog(id, path);
    } catch (e) {
      error = message(e);
      lines = [];
    }
  }

  $effect(() => {
    reload;
    if (active) untrack(load);
  });
</script>

{#if files && files.length === 0}
  <EmptyState icon="file-text" title="No log files" detail="The game writes its logs and crash reports here once it has run." />
{:else}
  <div class="split">
    <div class="files">
      {#each files ?? [] as f (f.path)}
        <button class="file" class:selected={selected === f.path} onclick={() => open(f.path)}>
          <span class="row name">
            <Icon name={f.crash ? 'triangle-alert' : 'file-text'} size={13} class={f.crash ? 'danger' : 'subtle'} />
            <span class="sm truncate">{f.name}</span>
          </span>
          <span class="xs subtle when">{ago(f.modified, launcher.now)}</span>
        </button>
      {/each}
    </div>
    <div class="view">
      {#if selected}
        <div class="path">
          <span class="grow truncate xs muted selectable">{selected}</span>
          <IconButton icon="external-link" title="Open in another app" onclick={() => api.openFile(selected!)} />
        </div>
      {/if}
      {#if error}<div class="error pad">{error}</div>{/if}
      <div class="grow-v"><LogView {lines} empty={['Pick a file', 'Choose a log or crash report on the left.']} /></div>
    </div>
  </div>
{/if}

<style>
  .split {
    height: 100%;
    display: flex;
  }
  .files {
    width: 230px;
    flex: none;
    overflow-y: auto;
    padding: 4px 0;
    border-right: 1px solid var(--border);
  }
  .file {
    width: calc(100% - 8px);
    margin: 0 4px;
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
    border-radius: 6px;
    text-align: left;
    cursor: pointer;
  }
  .file:hover {
    background: var(--hover);
  }
  .file.selected {
    background: var(--accent-soft);
  }
  .name {
    gap: 6px;
    width: 100%;
  }
  .when {
    padding-left: 19px;
  }
  .view {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .path {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 16px;
    border-bottom: 1px solid var(--border);
  }
  .pad {
    padding: 16px;
  }
  .grow-v {
    flex: 1;
    min-height: 0;
  }
</style>
