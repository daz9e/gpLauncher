<script lang="ts">
  // Mods, resource packs or shader packs of one instance: turn them on and off, remove them,
  // add files, find updates, and browse Modrinth for more.
  import { onMount, untrack } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { ask, open } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { actions } from '../../lib/actions';
  import { KIND_ICONS, KIND_LABELS, count, humanSize } from '../../lib/format';
  import type { Item, Kind, Project, TaskProgress, Update, Version } from '../../lib/types';
  import Button from '../../components/Button.svelte';
  import IconButton from '../../components/IconButton.svelte';
  import Icon from '../../components/Icon.svelte';
  import Switch from '../../components/Switch.svelte';
  import TextField from '../../components/TextField.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import ProgressBar from '../../components/ProgressBar.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import AddonBrowser from './AddonBrowser.svelte';

  let {
    id,
    kind,
    reload,
    active,
    browse,
    onsettings,
  }: {
    id: string;
    kind: Kind;
    reload: number;
    active: boolean;
    /** Bumped to switch to browsing Modrinth. */
    browse: number;
    onsettings: () => void;
  } = $props();

  type Updates = { state: 'idle' } | { state: 'checking' } | { state: 'found'; list: Update[] } | { state: 'failed'; error: string };
  type Task = { kind: 'install'; project: Project; version: Version | null } | { kind: 'update'; list: Update[] };

  let items = $state<Item[]>([]);
  let loaded = $state(false);
  let filter = $state('');
  /** Modrinth versions of installed files, by path. */
  let identified = $state<Record<string, Version>>({});
  let updates = $state<Updates>({ state: 'idle' });
  let mode = $state<'installed' | 'browse'>('installed');
  let browserOpened = $state(false);
  let queue = $state<Task[]>([]);
  /** What the backend is doing, with its progress. */
  let working = $state<{ task: string; status: string; progress: [number, number] | null } | null>(null);
  /** Result of the last action, until the next one. */
  let notice = $state<{ text: string; error: boolean } | null>(null);
  let dragging = $state(false);
  let generation = 0;
  let taskCounter = 0;

  let inst = $derived(launcher.instance(id));
  let running = $derived(launcher.running(id));
  let label = $derived(KIND_LABELS[kind].toLowerCase());
  let noLoader = $derived(kind === 'mods' && inst?.loader === 'vanilla');
  let visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return items.filter(
      (i) =>
        !q || i.name.toLowerCase().includes(q) || i.file_name.toLowerCase().includes(q) || i.id.toLowerCase().includes(q),
    );
  });
  let installedProjects = $derived(new Set(Object.values(identified).map((v) => v.project_id)));
  let queuedProjects = $derived(
    new Set(queue.flatMap((t) => (t.kind === 'install' ? [t.project.id] : []))),
  );
  let disabledCount = $derived(items.filter((i) => !i.enabled).length);
  let totalSize = $derived(items.reduce((sum, i) => sum + i.size, 0));

  /** Reads the folder again, then looks the new files up on Modrinth. */
  async function load() {
    const gen = ++generation;
    let list: Item[];
    try {
      list = await api.contentList(id, kind);
    } catch (e) {
      notice = { text: message(e), error: true };
      return;
    }
    if (gen !== generation) return;
    items = list;
    loaded = true;
    const paths = new Set(list.map((i) => i.path));
    // Updates found before refer to files that may be gone.
    if (updates.state === 'found') updates = { state: 'found', list: updates.list.filter((u) => paths.has(u.item.path)) };
    // Only files not looked up yet: the rest keep what Modrinth said before.
    const unknown = list.filter((i) => !(i.path in identified));
    if (!unknown.length) return;
    try {
      const found = await api.contentIdentify(unknown);
      if (gen !== generation) return;
      const kept = Object.fromEntries(Object.entries(identified).filter(([p]) => paths.has(p)));
      identified = { ...kept, ...found };
    } catch {
      // Offline: the list still works without Modrinth.
    }
  }

  $effect(() => {
    reload;
    if (active) untrack(load);
  });

  $effect(() => {
    if (browse) untrack(() => setMode('browse'));
  });

  function setMode(m: 'installed' | 'browse') {
    mode = m;
    if (m === 'browse') browserOpened = true;
  }

  async function toggle(item: Item) {
    try {
      const path = await api.contentSetEnabled(id, kind, $state.snapshot(item), !item.enabled);
      const version = identified[item.path];
      if (version) {
        const { [item.path]: _, ...rest } = identified;
        identified = { ...rest, [path]: version };
      }
      item.path = path;
      item.enabled = !item.enabled;
    } catch (e) {
      notice = { text: message(e), error: true };
    }
  }

  async function setAll(enabled: boolean) {
    for (const item of visible) if (item.enabled !== enabled) await toggle(item);
  }

  async function remove(item: Item) {
    const yes = await ask(`${item.file_name} will be deleted from the instance.`, {
      title: `Remove ${item.name}?`,
      kind: 'warning',
      okLabel: 'Remove',
      cancelLabel: 'Cancel',
    });
    if (!yes) return;
    try {
      await api.contentDelete(id, kind, $state.snapshot(item));
      items = items.filter((i) => i.path !== item.path);
      const { [item.path]: _, ...rest } = identified;
      identified = rest;
      if (updates.state === 'found') updates = { state: 'found', list: updates.list.filter((u) => u.item.path !== item.path) };
      notice = { text: `Removed ${item.name}`, error: false };
    } catch (e) {
      notice = { text: message(e), error: true };
    }
  }

  async function addFiles(paths: string[]) {
    try {
      const n = await api.contentAddFiles(id, kind, paths);
      notice = { text: `Added ${count(n, kind)}`, error: false };
    } catch (e) {
      notice = { text: message(e), error: true };
    }
    load();
  }

  async function pickFiles() {
    const paths = await open({
      multiple: true,
      directory: false,
      filters: [{ name: KIND_LABELS[kind], extensions: kind === 'mods' ? ['jar', 'litemod'] : ['zip'] }],
    });
    if (paths?.length) addFiles(paths);
  }

  async function checkUpdates() {
    if (updates.state === 'checking') return;
    updates = { state: 'checking' };
    try {
      updates = { state: 'found', list: await api.contentCheckUpdates(id, kind, $state.snapshot(items)) };
    } catch (e) {
      updates = { state: 'failed', error: message(e) };
    }
  }

  function updateItems(list: Update[]) {
    // The new file may reuse the name; look it up again afterwards.
    const paths = new Set(list.map((u) => u.item.path));
    identified = Object.fromEntries(Object.entries(identified).filter(([p]) => !paths.has(p)));
    if (updates.state === 'found') updates = { state: 'found', list: updates.list.filter((u) => !paths.has(u.item.path)) };
    enqueue({ kind: 'update', list });
  }

  function enqueue(task: Task) {
    queue = [...queue, task];
    runNext();
  }

  async function runNext() {
    if (working || !queue.length) return;
    const [next, ...rest] = queue;
    queue = rest;
    const task = `${id}:${kind}:${++taskCounter}`;
    working = {
      task,
      status: next.kind === 'install' ? `Installing ${next.project.title}` : `Updating ${count(next.list.length, kind)}`,
      progress: null,
    };
    let result: { text: string; error: boolean };
    try {
      const text =
        next.kind === 'install'
          ? await api.contentInstall(id, kind, $state.snapshot(next.project), next.version ? $state.snapshot(next.version) : null, [...installedProjects], task)
          : await api.contentUpdate(id, kind, $state.snapshot(next.list), task);
      result = { text, error: false };
    } catch (e) {
      result = { text: message(e), error: true };
    }
    working = null;
    notice = result;
    await load();
    runNext();
  }

  onMount(() => {
    const unlisten = [
      listen<TaskProgress>('task-progress', (e) => {
        const p = e.payload;
        if (!working || p.task !== working.task) return;
        if (p.status !== null) working.status = p.status;
        else working.progress = p.progress;
      }),
      getCurrentWebview().onDragDropEvent((e) => {
        if (!active) return;
        const type = e.payload.type;
        if (type === 'enter' || type === 'over') dragging = true;
        else if (type === 'leave') dragging = false;
        else if (type === 'drop') {
          dragging = false;
          addFiles(e.payload.paths);
        }
      }),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });

  function updateFor(item: Item): Update | undefined {
    return updates.state === 'found' ? updates.list.find((u) => u.item.path === item.path) : undefined;
  }

  function meta(item: Item): string {
    const parts: string[] = [];
    if (item.version) parts.push(item.version);
    if (item.authors.length) parts.push(`by ${item.authors.slice(0, 2).join(', ')}`);
    return parts.join(' · ');
  }
</script>

<div class="content" class:dragging>
  <div class="main">
    {#if mode === 'installed'}
      <div class="toolbar">
        <div class="filter"><TextField bind:value={filter} placeholder="Filter {label}" icon="search" /></div>
        <span class="grow"></span>
        <Button
          icon="refresh-cw"
          label={updates.state === 'checking' ? 'Checking…' : 'Check for updates'}
          variant="ghost"
          disabled={!items.length || updates.state === 'checking'}
          onclick={checkUpdates}
        />
        <IconButton icon="plus" title="Add files…" onclick={pickFiles} />
        <IconButton icon="folder" title="Open folder" onclick={() => inst && actions.openFolder(`${inst.game_dir}/${kind}`)} />
        <Button icon="download" label="Get {label}" variant="primary" disabled={noLoader} onclick={() => setMode('browse')} />
      </div>
      {#if running}
        <div class="banner"><Icon name="info" size={15} class="accent" />The game is running: changes apply the next time it starts.</div>
      {/if}
      {#if updates.state === 'found' && updates.list.length}
        {@const list = updates.list}
        <div class="banner">
          <Icon name="circle-arrow-up" size={15} class="success" />
          <span class="grow">{list.length} update{list.length === 1 ? '' : 's'} available</span>
          <Button label="Update all" variant="primary" onclick={() => updateItems($state.snapshot(list))} />
        </div>
      {:else if updates.state === 'found'}
        <div class="banner"><Icon name="check" size={15} class="success" />Everything is up to date.</div>
      {:else if updates.state === 'failed'}
        <div class="banner"><Icon name="triangle-alert" size={15} class="danger" />Could not check for updates: {updates.error}</div>
      {/if}
      <div class="list">
        {#if noLoader}
          <EmptyState
            icon="puzzle"
            title="This instance has no mod loader"
            detail="Mods need Fabric, Quilt, Forge or NeoForge. Pick one in the instance settings."
          >
            <div style="margin-top: 8px">
              <Button icon="settings" label="Choose a mod loader" variant="primary" onclick={onsettings} />
            </div>
          </EmptyState>
        {:else if !loaded}
          <div></div>
        {:else if !items.length}
          <EmptyState
            icon={KIND_ICONS[kind]}
            title="No {label} yet"
            detail="Get them from Modrinth, or drop {kind === 'mods' ? '.jar' : '.zip'} files here."
          >
            <div style="margin-top: 8px">
              <Button icon="download" label="Get {label}" variant="primary" onclick={() => setMode('browse')} />
            </div>
          </EmptyState>
        {:else if !visible.length}
          <EmptyState icon="search" title="Nothing matches the filter" />
        {:else}
          {#each visible as item (item.path)}
            {@const update = updateFor(item)}
            {@const project = identified[item.path]?.project_id}
            <div class="item" class:off={!item.enabled} data-file={item.file_name}>
              <Switch on={item.enabled} title={item.enabled ? 'Turn off' : 'Turn on'} onchange={() => toggle(item)} />
              <Thumb src={item.icon} size={36} radius={0.17} fallback={KIND_ICONS[kind]} dim={!item.enabled} />
              <div class="grow col text">
                <div class="row names">
                  <span class="sm medium truncate name">{item.name}</span>
                  {#if meta(item)}<span class="xs subtle truncate">{meta(item)}</span>{/if}
                </div>
                <span class="xs muted truncate">{item.description || item.file_name}</span>
              </div>
              {#if !item.enabled}<span class="badge">Off</span>{/if}
              {#if update}
                <Button
                  icon="circle-arrow-up"
                  label="Update"
                  title="Update to {update.latest.number}"
                  onclick={() => updateItems([$state.snapshot(update)])}
                />
              {/if}
              <div class="hover-actions">
                {#if project}
                  <IconButton icon="external-link" title="Open on Modrinth" onclick={() => openUrl(`https://modrinth.com/project/${project}`)} />
                {/if}
                <IconButton icon="trash" title="Remove" onclick={() => remove(item)} />
              </div>
            </div>
          {/each}
        {/if}
      </div>
      <footer class="footer xs muted">
        <span>
          {count(items.length, kind)}{disabledCount ? `, ${disabledCount} off` : ''} · {humanSize(totalSize)}
        </span>
        <span class="grow"></span>
        {#if items.length}
          <button class="plain" onclick={() => setAll(true)}>Turn all on</button>
          <button class="plain" onclick={() => setAll(false)}>Turn all off</button>
        {/if}
      </footer>
    {/if}
    {#if browserOpened}
      <div class="browse" hidden={mode !== 'browse'}>
        <div class="toolbar tight">
          <Button icon="chevron-left" label="Installed {label}" variant="ghost" onclick={() => setMode('installed')} />
          <span class="grow"></span>
          <span class="xs subtle pad">{items.length} installed</span>
        </div>
        <div class="grow-v">
          <AddonBrowser
            {id}
            {kind}
            active={active && mode === 'browse'}
            installed={installedProjects}
            queued={queuedProjects}
            oninstall={(project, version) => enqueue({ kind: 'install', project, version })}
          />
        </div>
      </div>
    {/if}
  </div>
  {#if working || notice}
    {@const error = !working && !!notice?.error}
    <div class="status xs" class:error>
      <span class="dot" class:working={!!working} class:error></span>
      <span class="grow truncate">{working ? working.status : notice?.text}</span>
      {#if queue.length}<span>{queue.length} queued</span>{/if}
      {#if working?.progress}
        <div class="bar"><ProgressBar ratio={working.progress[0] / working.progress[1]} /></div>
      {/if}
      {#if !working}<IconButton icon="x" title="Dismiss" onclick={() => (notice = null)} />{/if}
    </div>
  {/if}
</div>

<style>
  .content {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .content.dragging {
    background: var(--accent-soft);
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  .toolbar.tight {
    padding: 6px 8px;
  }
  .pad {
    padding-right: 8px;
  }
  .filter {
    width: 240px;
  }
  .banner {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .banner :global(.accent) {
    color: var(--accent);
  }
  .banner :global(.success) {
    color: var(--success);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .item {
    height: 56px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
  }
  .item:hover {
    background: var(--hover);
  }
  .text {
    gap: 2px;
  }
  .off .text {
    opacity: 0.55;
  }
  .names {
    min-width: 0;
  }
  .name {
    flex-shrink: 1;
  }
  .badge {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 12px;
    font-weight: 500;
    color: var(--subtle);
    background: color-mix(in srgb, var(--subtle) 14%, transparent);
  }
  .hover-actions {
    display: flex;
    flex: none;
    gap: 2px;
    visibility: hidden;
  }
  .item:hover .hover-actions {
    visibility: visible;
  }
  .footer {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 34px;
    padding: 0 16px;
    border-top: 1px solid var(--border);
  }
  .plain {
    cursor: pointer;
    color: var(--muted);
    font-size: 12px;
  }
  .plain:hover {
    color: var(--text);
  }
  .browse {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .browse[hidden] {
    display: none;
  }
  .grow-v {
    flex: 1;
    min-height: 0;
  }
  .status {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 30px;
    padding: 0 16px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    color: var(--muted);
  }
  .status.error {
    color: var(--danger);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
    background: var(--success);
  }
  .dot.working {
    background: var(--accent);
  }
  .dot.error {
    background: var(--danger);
  }
  .bar {
    width: 140px;
  }
</style>
