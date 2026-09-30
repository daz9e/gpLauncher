<script lang="ts">
  // Main window: toolbar, instance grid, sidebar for the selected instance, status bar.
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { launcher } from '../lib/store.svelte';
  import { api, message } from '../lib/api';
  import { actions } from '../lib/actions';
  import type { Instance, Loader } from '../lib/types';
  import Button from '../components/Button.svelte';
  import TextField from '../components/TextField.svelte';
  import Avatar from '../components/Avatar.svelte';
  import Icon from '../components/Icon.svelte';
  import InstanceGrid from './main/InstanceGrid.svelte';
  import Sidebar from './main/Sidebar.svelte';
  import StatusBar from './main/StatusBar.svelte';
  import ContextMenu from './main/ContextMenu.svelte';
  import AddInstance from './add/AddInstance.svelte';
  import Accounts from './Accounts.svelte';
  import SettingsPage from './settings/SettingsPage.svelte';

  let snap = $derived(launcher.snap!);
  let selected = $state<string | null>(launcher.snap?.instances[0]?.id ?? null);
  let query = $state('');
  let searchInput = $state<HTMLInputElement>();
  let dialog = $state<'add' | 'accounts' | null>(null);
  let addLoader = $state<Loader | undefined>();
  let settingsOpen = $state(false);
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  let dragging = $state(false);

  let account = $derived(snap.accounts[snap.selected_account]);
  let offline = $derived(!account || account.kind === 'Offline');

  // The selection follows deleted instances to a neighbor.
  let lastIndex = 0;
  $effect(() => {
    const list = snap.instances;
    const index = list.findIndex((i) => i.id === selected);
    if (index >= 0) lastIndex = index;
    else selected = (list[lastIndex] ?? list[list.length - 1] ?? list[0])?.id ?? null;
  });

  function select(id: string) {
    selected = id;
    menu = null;
    // A finished job's message has been seen once the user moves on.
    if (snap.job && !snap.job.active) api.clearNotice();
  }

  function openAdd(loader?: Loader) {
    if (dialog) return;
    settingsOpen = false;
    addLoader = loader;
    dialog = 'add';
  }

  function openSettings() {
    if (!dialog) settingsOpen = true;
  }

  function openSelected() {
    if (selected) actions.open(selected, 'mods');
  }

  function launchSelected() {
    if (selected) actions.launch(selected);
  }

  async function importPaths(paths: string[]) {
    try {
      await api.importFiles(paths);
      dialog = null;
    } catch (e) {
      // Shown in the status bar by the backend when nothing was importable.
      console.warn(message(e));
    }
  }

  function keydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod || dialog) return;
    const key = e.key.toLowerCase();
    if (key === 'n') openAdd();
    else if (key === 'f' && !settingsOpen) {
      searchInput?.focus();
      searchInput?.select();
    } else if (key === ',') openSettings();
    else if (key === 'enter') launchSelected();
    else if (key === 'o') openSelected();
    else return;
    e.preventDefault();
  }

  async function debugOpen(spec: string) {
    const [what, arg, extra] = spec.split(':');
    if (what === 'add') openAdd((['fabric', 'forge', 'neoforge', 'quilt'] as Loader[]).find((l) => l === arg));
    else if (what === 'accounts') dialog = 'accounts';
    else if (what === 'settings') settingsOpen = true;
    else if (launcher.instance(what)) {
      select(what);
      await actions.open(what, ((arg || 'console') + (extra ? `:${extra}` : '')) as never);
    }
  }

  onMount(() => {
    const unlisten = [
      listen<string>('reveal', (e) => select(e.payload)),
      listen<string>('menu', (e) => {
        if (e.payload === 'add-instance') openAdd();
        else if (e.payload === 'settings') openSettings();
        else if (e.payload === 'play-selected') launchSelected();
        else if (e.payload === 'open-selected') openSelected();
      }),
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === 'enter' || e.payload.type === 'over') dragging = true;
        else if (e.payload.type === 'leave') dragging = false;
        else if (e.payload.type === 'drop') {
          dragging = false;
          if (!settingsOpen) importPaths(e.payload.paths);
        }
      }),
    ];
    const boot = launcher.boot;
    if (boot?.launch) {
      const id = boot.launch;
      if (launcher.instance(id)) select(id);
      actions.launch(id);
    }
    if (boot?.open) tick().then(() => debugOpen(boot.open!));
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });

  let selectedInstance = $derived(launcher.instance(selected));
  let menuInstance = $derived(menu ? launcher.instance(menu.id) : undefined);

  function contextMenu(inst: Instance, e: MouseEvent) {
    select(inst.id);
    menu = { id: inst.id, x: e.clientX, y: e.clientY };
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="window" class:dragging={dragging && !settingsOpen && dialog !== 'accounts'}>
  {#if settingsOpen}
    <SettingsPage onclose={() => (settingsOpen = false)} />
  {:else}
    <div class="main">
      <header class="toolbar">
        <Button icon="plus" label="Add Instance" variant="primary" onclick={() => openAdd()} />
        <div style="width: 6px"></div>
        <Button
          icon="folder"
          label="Folder"
          variant="ghost"
          title="Open the launcher folder"
          onclick={() => actions.openFolder(snap.settings.data_dir)}
        />
        <Button icon="settings" label="Settings" variant="ghost" onclick={openSettings} />
        <div class="grow"></div>
        <div class="search">
          <TextField bind:value={query} bind:input={searchInput} placeholder="Search instances" icon="search" />
        </div>
        <button
          class="account"
          title={offline ? 'Offline account · switch or sign in' : 'Microsoft account · switch'}
          onclick={() => (dialog = 'accounts')}
        >
          <Avatar name={account?.name ?? 'Player'} />
          <span class="sm">{account?.name ?? 'Player'}</span>
          <Icon name="chevron-down" size={12} class="muted" />
        </button>
      </header>
      <div class="body">
        <InstanceGrid
          {query}
          {selected}
          onselect={select}
          oncontextmenu={contextMenu}
          onadd={() => openAdd()}
        />
        {#if selectedInstance}
          <Sidebar inst={selectedInstance} />
        {:else}
          <aside class="empty-sidebar"></aside>
        {/if}
      </div>
    </div>
  {/if}
  <StatusBar {selected} />
</div>

{#if menu && menuInstance}
  <ContextMenu inst={menuInstance} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

{#if dialog === 'add'}
  <AddInstance
    loader={addLoader}
    onclose={() => (dialog = null)}
    onimport={importPaths}
    oncreated={(inst) => {
      select(inst.id);
      dialog = null;
    }}
    oninstall={() => (dialog = null)}
    {dragging}
  />
{:else if dialog === 'accounts'}
  <Accounts onclose={() => (dialog = null)} />
{/if}

<style>
  .window {
    height: 100%;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    flex: none;
    height: 46px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  .search {
    width: 240px;
    min-width: 120px;
    flex-shrink: 1;
  }
  .account {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: 8px;
    padding: 4px 8px 4px 4px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg);
    cursor: pointer;
    white-space: nowrap;
  }
  .account:hover {
    background: var(--hover);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .empty-sidebar {
    width: 264px;
    flex: none;
    background: var(--panel);
    border-left: 1px solid var(--border);
  }
  .dragging .main {
    outline: 2px dashed var(--accent);
    outline-offset: -4px;
  }
</style>
