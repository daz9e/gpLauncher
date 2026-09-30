<script lang="ts">
  // A window for one instance: its console, mods and packs, worlds, screenshots, log files and
  // settings. One window per instance; opening it again brings the existing one to the front.
  import { onMount, untrack } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { launcher } from '../lib/store.svelte';
  import { actions } from '../lib/actions';
  import { KIND_ICONS, KIND_LABELS, duration, isError } from '../lib/format';
  import type { InstancePage, Kind } from '../lib/types';
  import Button from '../components/Button.svelte';
  import Icon from '../components/Icon.svelte';
  import Thumb from '../components/Thumb.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import LogView from './instance/LogView.svelte';
  import ContentPage from './instance/ContentPage.svelte';
  import WorldsPage from './instance/WorldsPage.svelte';
  import ScreenshotsPage from './instance/ScreenshotsPage.svelte';
  import LogFilesPage from './instance/LogFilesPage.svelte';
  import InstanceSettings from './instance/InstanceSettings.svelte';

  let { id, initialPage }: { id: string; initialPage: string } = $props();

  const KINDS: Kind[] = ['mods', 'resourcepacks', 'shaderpacks'];
  const NAV: InstancePage[] = ['console', ...KINDS, 'worlds', 'screenshots', 'logs', 'settings'];
  const LABELS: Record<string, string> = {
    console: 'Console',
    ...KIND_LABELS,
    worlds: 'Worlds',
    screenshots: 'Screenshots',
    logs: 'Log files',
    settings: 'Settings',
  };
  const ICONS: Record<string, string> = {
    console: 'terminal',
    ...KIND_ICONS,
    worlds: 'earth',
    screenshots: 'image',
    logs: 'file-text',
    settings: 'settings',
  };

  /** `<page>[:browse]`, as the backend and development specs send it. */
  function parse(spec: string): [InstancePage, boolean] {
    const [page, extra] = spec.split(':');
    return [(NAV.includes(page as InstancePage) ? page : 'console') as InstancePage, extra === 'browse'];
  }

  const [startPage, startBrowse] = untrack(() => parse(initialPage));
  let page = $state<InstancePage>(startPage);
  /** Pages opened so far stay mounted, so they keep their state. */
  let opened = $state<Record<string, boolean>>({ [startPage]: true });
  let browse = $state<Record<string, number>>(startBrowse ? { [startPage]: 1 } : {});
  /** Bumped when the window is focused: pages read their files again. */
  let reload = $state(0);

  let inst = $derived(launcher.instance(id));
  let session = $derived(launcher.snap!.sessions[id]);
  let phase = $derived(session?.phase);

  function show(p: InstancePage, browsing = false) {
    page = p;
    opened[p] = true;
    if (browsing) browse[p] = (browse[p] ?? 0) + 1;
    reload++;
  }

  // The instance was deleted or the launcher folder changed.
  $effect(() => {
    if (launcher.snap && !inst) getCurrentWindow().close();
  });

  $effect(() => {
    if (inst) getCurrentWindow().setTitle(inst.name);
  });

  onMount(() => {
    const window = getCurrentWindow();
    const unlisten = [
      listen<string>('show-page', (e) => show(...parse(e.payload))),
      // Files change while the game runs or in other apps: look again when the window is used.
      window.onFocusChanged(({ payload: focused }) => focused && reload++),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });

  function keydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'w') {
      e.preventDefault();
      getCurrentWindow().close();
    }
  }

  function play() {
    actions.launch(id);
    show('console');
  }

  let statusText = $derived.by(() => {
    if (!session) return '';
    if (session.phase === 'running')
      return `Playing for ${duration(launcher.now - session.started_at / 1000)}`;
    if (session.phase === 'finished' && session.played !== null && !session.status.startsWith('Error'))
      return `${session.status} · played ${duration(session.played)}`;
    return session.status;
  });
</script>

<svelte:window onkeydown={keydown} />

{#if inst}
  <div class="window">
    <nav class="nav">
      <div class="head">
        <div class="row">
          <Thumb src={inst.icon} size={44} radius={0.18} edge running={phase === 'preparing' || phase === 'running'} />
          <div class="grow col">
            <span class="sm semibold clamp">{inst.name}</span>
            <span class="xs muted truncate">{inst.description}</span>
          </div>
        </div>
        {#if phase === 'running'}
          <Button icon="stop" label="Stop" variant="stop" class="wide" onclick={() => actions.kill(id)} />
        {:else if phase === 'preparing'}
          <Button label="Starting…" disabled class="wide" />
        {:else}
          <Button icon="play" label="Play" variant="play" class="wide" onclick={play} />
        {/if}
        {#if session}
          <div class="col status">
            <span class="xs clamp" class:danger={isError(session.status)} class:muted={!isError(session.status)}>
              {statusText}
            </span>
            {#if session.progress}<ProgressBar ratio={session.progress[0] / session.progress[1]} />{/if}
          </div>
        {/if}
      </div>
      <div class="items">
        {#each NAV as p (p)}
          <button class="nav-item" class:active={page === p} onclick={() => show(p)}>
            <Icon name={ICONS[p]} size={15} class="icon" />
            <span class="grow">{LABELS[p]}</span>
            {#if p === 'console' && phase === 'running'}<span class="live" title="Running"></span>{/if}
          </button>
        {/each}
      </div>
      <div class="foot">
        <Button icon="folder" label="Open game folder" variant="ghost" class="folder" onclick={() => actions.openFolder(inst!.game_dir)} />
      </div>
    </nav>
    <main class="body">
      {#each NAV as p (p)}
        {#if opened[p]}
          <div class="page" hidden={page !== p}>
            {#if p === 'console'}
              <LogView session={id} empty={['No output yet', "Press Play: the game's output appears here while it runs."]} />
            {:else if p === 'mods' || p === 'resourcepacks' || p === 'shaderpacks'}
              <ContentPage
                {id}
                kind={p}
                {reload} active={page === p}
                browse={browse[p] ?? 0}
                onsettings={() => show('settings')}
              />
            {:else if p === 'worlds'}
              <WorldsPage {id} {reload} active={page === p} />
            {:else if p === 'screenshots'}
              <ScreenshotsPage {id} {reload} active={page === p} />
            {:else if p === 'logs'}
              <LogFilesPage {id} {reload} active={page === p} />
            {:else}
              <InstanceSettings {id} />
            {/if}
          </div>
        {/if}
      {/each}
    </main>
  </div>
{/if}

<style>
  .window {
    height: 100%;
    display: flex;
  }
  .nav {
    width: 220px;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-right: 1px solid var(--border);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .head .row {
    gap: 10px;
  }
  .head :global(.wide) {
    width: 100%;
    height: 34px;
  }
  .clamp {
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .status {
    gap: 4px;
  }
  .items {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
  }
  .live {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
  }
  .foot {
    padding: 8px;
    border-top: 1px solid var(--border);
  }
  .foot :global(.folder) {
    width: 100%;
    justify-content: flex-start;
    color: var(--muted) !important;
  }
  .body {
    flex: 1;
    min-width: 0;
    height: 100%;
    position: relative;
  }
  .page {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
  }
  .page[hidden] {
    display: none;
  }
</style>
