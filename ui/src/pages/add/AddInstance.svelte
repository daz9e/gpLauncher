<script lang="ts">
  // "Add Instance" dialog. Pages on the left: a custom instance (name, loader, Minecraft version),
  // importing a modpack file, and browsing modpacks on Modrinth and CurseForge.
  import { onMount, tick } from 'svelte';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { LOADER_LABELS, versionKindLabel } from '../../lib/format';
  import type { Instance, Loader, LoaderVersion, Pack, PackVersion, Platform, VersionEntry } from '../../lib/types';
  import Modal from '../../components/Modal.svelte';
  import Button from '../../components/Button.svelte';
  import Icon from '../../components/Icon.svelte';
  import TextField from '../../components/TextField.svelte';
  import Segments from '../../components/Segments.svelte';
  import Dropdown from '../../components/Dropdown.svelte';
  import ModpackBrowser from './ModpackBrowser.svelte';

  let {
    loader: initialLoader,
    dragging = false,
    onclose,
    onimport,
    oncreated,
    oninstall,
  }: {
    loader?: Loader;
    dragging?: boolean;
    onclose: () => void;
    onimport: (paths: string[]) => void;
    oncreated: (inst: Instance) => void;
    oninstall: () => void;
  } = $props();

  type Page = 'custom' | 'import' | Platform;
  const PAGES: [Page, string, string][] = [
    ['custom', 'Custom', 'box'],
    ['import', 'Import', 'folder'],
    ['modrinth', 'Modrinth', 'modrinth'],
    ['curseforge', 'CurseForge', 'globe'],
  ];
  const LOADERS: Loader[] = ['vanilla', 'fabric', 'quilt', 'forge', 'neoforge'];
  const PLATFORMS: Platform[] = ['modrinth', 'curseforge'];
  const CURSEFORGE_CONSOLE = 'https://console.curseforge.com/';

  /** Result of a background lookup; `undefined` while it is still running. */
  type Lookup<T> = { ok: true; value: T } | { ok: false; error: string } | undefined;

  let page = $state<Page>('custom');
  let name = $state('');
  let nameInput = $state<HTMLInputElement>();
  let search = $state('');
  let loader = $state<Loader>('vanilla');
  let snapshots = $state(false);
  let old = $state(false);
  let versions = $state<Lookup<VersionEntry[]>>();
  /** Game versions each loader supports (`null` = no restriction). */
  let supported = $state<Record<string, Lookup<Set<string> | null>>>({ vanilla: { ok: true, value: null } });
  let selected = $state<string | null>(null);
  let error = $state<string | null>(null);
  /** Loader builds by `loader:minecraft`. */
  let loaderVersions = $state<Record<string, Lookup<LoaderVersion[]>>>({});
  /** A build picked for (loader, Minecraft version); anything else means the latest stable. */
  let loaderPick = $state<{ loader: Loader; mc: string; version: string } | null>(null);
  let keyInput = $state('');
  let listEl = $state<HTMLDivElement>();
  let browsers = $state<Record<string, ModpackBrowser | undefined>>({});
  let selection = $state<Record<string, [Pack, PackVersion] | null>>({});

  let snap = $derived(launcher.snap!);
  let curseforgeKey = $derived(
    launcher.boot?.env_curseforge_key || snap.settings.curseforge_api_key.trim() !== '',
  );
  let releases = $derived(versions?.ok ? versions.value.filter((v) => v.kind === 'release').map((v) => v.id) : []);

  let visible = $derived.by(() => {
    if (!versions?.ok) return [];
    const q = search.trim().toLowerCase();
    // While the loader list is loading (or failed to load) show everything.
    const lookup = supported[loader];
    const allowed = lookup?.ok ? lookup.value : null;
    return versions.value.filter((v) => {
      const kindOk =
        v.kind === 'release' ||
        (v.kind === 'snapshot' && snapshots) ||
        ((v.kind === 'old_beta' || v.kind === 'old_alpha') && old);
      // Local profiles (Fabric, Forge, ...) are what loaders produce, not something to pick here.
      return kindOk && (!allowed || allowed.has(v.id)) && (!q || v.id.toLowerCase().includes(q));
    });
  });

  // Keep a visible version selected.
  $effect(() => {
    if (!visible.some((v) => v.id === selected)) selected = visible[0]?.id ?? null;
  });

  let defaultName = $derived(selected ? (loader === 'vanilla' ? selected : `${selected} ${LOADER_LABELS[loader]}`) : '');
  let chosenBuild = $derived(
    loaderPick && loaderPick.loader === loader && loaderPick.mc === selected ? loaderPick.version : null,
  );
  let buildKey = $derived(selected && loader !== 'vanilla' ? `${loader}:${selected}` : null);

  // Loads the builds of the current loader for the selected version, once.
  $effect(() => {
    const key = buildKey;
    if (!key || key in loaderVersions) return;
    loaderVersions[key] = undefined;
    const [l, mc] = [loader, selected!];
    api.loaderVersions(l, mc).then(
      (value) => {
        loaderVersions[key] = { ok: true, value };
      },
      (e) => {
        loaderVersions[key] = { ok: false, error: message(e) };
      },
    );
  });

  function setLoader(l: Loader) {
    loader = l;
    if (!(l in supported)) {
      supported[l] = undefined;
      api.supportedVersions(l).then(
        (list) => {
        supported[l] = { ok: true, value: list ? new Set(list) : null };
      },
        (e) => {
        supported[l] = { ok: false, error: message(e) };
      },
      );
    }
  }

  onMount(() => {
    api.listVersions().then(
      (value) => (versions = { ok: true, value }),
      (e) => (versions = { ok: false, error: message(e) }),
    );
    if (initialLoader) setLoader(initialLoader);
  });

  async function scrollToSelected() {
    await tick();
    listEl?.querySelector('.version.selected')?.scrollIntoView({ block: 'nearest' });
  }

  function move(delta: number) {
    if (page === 'custom') {
      if (!visible.length) return;
      const ix = visible.findIndex((v) => v.id === selected);
      const next = ix < 0 ? 0 : Math.min(Math.max(ix + delta, 0), visible.length - 1);
      selected = visible[next].id;
      scrollToSelected();
    } else if (page === 'modrinth' || page === 'curseforge') {
      browsers[page]?.move(delta);
    }
  }

  async function create() {
    if (!selected) return;
    try {
      const inst = await api.createInstance(name.trim() || defaultName, selected, loader, chosenBuild ?? '');
      oncreated(inst);
    } catch (e) {
      error = message(e);
    }
  }

  async function pickModpack() {
    const paths = await openDialog({
      multiple: true,
      directory: false,
      filters: [{ name: 'Modpacks', extensions: ['mrpack', 'zip'] }],
    });
    if (paths && paths.length) onimport(paths);
  }

  async function saveKey() {
    const key = keyInput.trim();
    if (!key) return;
    await api.setCurseforgeKey(key);
  }

  function install() {
    if (page !== 'modrinth' && page !== 'curseforge') return;
    const pick = selection[page];
    if (!pick || !pick[1].url) return;
    api.installModpack(pick[0], pick[1]).then(oninstall, (e) => api.notice(message(e)));
  }

  function confirm() {
    if (page === 'custom') create();
    else if (page === 'import') pickModpack();
    else if (page === 'curseforge' && !curseforgeKey) saveKey();
    else install();
  }

  function keydown(e: KeyboardEvent) {
    if (e.defaultPrevented) return;
    // Buttons and open menus handle their own keys.
    const target = e.target as HTMLElement;
    if (target.closest?.('.dropdown')) return;
    if (e.key === 'Enter' && target.tagName === 'BUTTON' && !target.classList.contains('version')) return;
    if (e.key === 'Escape') onclose();
    else if (e.key === 'Enter' && !e.metaKey && !e.ctrlKey) confirm();
    else if (e.key === 'ArrowUp') move(-1);
    else if (e.key === 'ArrowDown') move(1);
    else return;
    e.preventDefault();
  }

  let footer = $derived.by((): { label: string; enabled: boolean; note?: string } => {
    if (page === 'custom') return { label: 'Create', enabled: selected !== null };
    if (page === 'import') return { label: 'Choose files…', enabled: true };
    if (page === 'curseforge' && !curseforgeKey) return { label: 'Save key', enabled: keyInput.trim() !== '' };
    const pick = selection[page];
    if (pick && !pick[1].url)
      return { label: 'Install', enabled: false, note: 'The author does not allow launchers to download this version' };
    return { label: 'Install', enabled: !!pick };
  });

  $effect(() => {
    if (page === 'custom') nameInput?.focus();
  });
</script>

<svelte:window onkeydown={keydown} />

<Modal {onclose}>
  <div class="dialog" role="dialog" aria-label="Add Instance">
    <nav class="nav">
      <div class="title semibold">Add Instance</div>
      {#each PAGES as [p, label, icon] (p)}
        <button class="nav-item" class:active={page === p} onclick={() => (page = p)}>
          <Icon name={icon} size={14} class="icon" />{label}
        </button>
      {/each}
    </nav>
    <div class="content">
      <div class="page">
        {#if page === 'custom'}
          <div class="custom">
            <label class="field-block">
              <span class="caption">Name</span>
              <TextField bind:value={name} bind:input={nameInput} placeholder={defaultName || 'Instance name'} label="Name" />
            </label>
            <div class="field-block">
              <span class="caption">Mod loader</span>
              <Segments
                label="Mod loader"
                options={LOADERS.map((l) => ({ value: l, label: LOADER_LABELS[l] }))}
                value={loader}
                onchange={setLoader}
              />
              {#if loader !== 'vanilla'}
                {@const label = LOADER_LABELS[loader]}
                {@const builds = buildKey ? loaderVersions[buildKey] : undefined}
                {#if !selected}
                  <div class="xs muted">Pick a Minecraft version to see the {label} builds.</div>
                {:else if builds === undefined}
                  <div class="xs muted">Loading {label} builds for {selected}…</div>
                {:else if !builds.ok}
                  <div class="xs muted">Could not load {label} builds: {builds.error}</div>
                {:else if builds.value.length === 0}
                  <div class="xs danger">{label} has no builds for Minecraft {selected}</div>
                {:else}
                  {@const list = builds.value.slice(0, 80)}
                  <div class="row loader-row">
                    <Dropdown
                      label="Build"
                      value={chosenBuild ?? 'Latest stable'}
                      options={[
                        { label: 'Latest stable', selected: chosenBuild === null },
                        ...list.map((b) => ({
                          label: b.stable ? b.version : `${b.version} (beta)`,
                          selected: chosenBuild === b.version,
                        })),
                      ]}
                      onselect={(i) =>
                        (loaderPick = i === 0 ? null : { loader, mc: selected!, version: list[i - 1].version })}
                    />
                    <span class="xs muted">
                      {#if supported[loader] && !supported[loader]?.ok}
                        Could not check which versions {label} supports; showing all.
                      {:else if chosenBuild}
                        This {label} build is installed on first launch.
                      {:else}
                        The newest stable {label} build is installed on first launch.
                      {/if}
                    </span>
                  </div>
                {/if}
              {/if}
            </div>
            <div class="field-block versions-block">
              <span class="caption">Minecraft version</span>
              <div class="row">
                <div class="grow"><TextField bind:value={search} placeholder="Search versions" /></div>
                <button class="toggle" class:on={snapshots} aria-pressed={snapshots} onclick={() => (snapshots = !snapshots)}>Snapshots</button>
                <button class="toggle" class:on={old} aria-pressed={old} onclick={() => (old = !old)}>Old</button>
              </div>
              <div class="version-list" bind:this={listEl} role="listbox" aria-label="Minecraft versions">
                {#if versions === undefined}
                  <div class="placeholder">Loading versions…</div>
                {:else if !versions.ok}
                  <div class="placeholder">Could not load the version list: {versions.error}</div>
                {:else if loader in supported && supported[loader] === undefined}
                  <div class="placeholder">Checking {LOADER_LABELS[loader]} support…</div>
                {:else if visible.length === 0}
                  <div class="placeholder">No matching versions</div>
                {:else}
                  {#each visible as v (v.id)}
                    <button
                      class="version"
                      class:selected={v.id === selected}
                      role="option"
                      aria-selected={v.id === selected}
                      onclick={() => (selected = v.id)}
                    >
                      <span class="grow truncate">{v.id}</span>
                      {#if v.installed}<span class="xs subtle">installed</span>{/if}
                      <span class="kind xs muted">{versionKindLabel(v.kind)}</span>
                    </button>
                  {/each}
                {/if}
              </div>
            </div>
            {#if error}<div class="xs danger">{error}</div>{/if}
          </div>
        {:else if page === 'import'}
          <div class="import">
            <button class="drop" class:over={dragging} onclick={pickModpack}>
              <Icon name="folder" size={28} class="subtle" />
              <span class="medium">Drop modpack files here</span>
              <span class="sm muted">or click to choose them</span>
            </button>
            <div class="field-block">
              <span class="caption">Supported formats</span>
              <div class="format sm"><span>Modrinth</span><span class="muted">.mrpack</span></div>
              <div class="format sm">
                <span>CurseForge</span>
                <span class="muted">.zip{curseforgeKey ? '' : ' (needs an API key, see the CurseForge page)'}</span>
              </div>
              <div class="format sm"><span>MultiMC / Prism</span><span class="muted">.zip export</span></div>
            </div>
          </div>
        {:else if page === 'curseforge' && !curseforgeKey}
          <div class="key-page">
            <div class="semibold">CurseForge API key needed</div>
            <div class="sm muted">
              CurseForge only answers apps that send an API key. You can get one for free in the CurseForge
              console; it is stored in the launcher settings. The CURSEFORGE_API_KEY environment variable
              works too.
            </div>
            <TextField bind:value={keyInput} placeholder="Paste your API key" autofocus />
            <button class="link sm" onclick={() => openUrl(CURSEFORGE_CONSOLE)}>Open console.curseforge.com</button>
          </div>
        {/if}
        {#each PLATFORMS as platform (platform)}
          {#if (page === platform && (platform === 'modrinth' || curseforgeKey)) || browsers[platform]}
            <div class="browser" hidden={page !== platform}>
              <ModpackBrowser
                bind:this={browsers[platform]}
                {platform}
                {releases}
                active={page === platform}
                onselection={(pick) => {
                  selection[platform] = pick;
                }}
              />
            </div>
          {/if}
        {/each}
      </div>
      <footer class="footer">
        <span class="grow truncate xs muted">{footer.note ?? ''}</span>
        <Button label="Cancel" onclick={onclose} />
        <Button label={footer.label} variant="primary" disabled={!footer.enabled} onclick={confirm} />
      </footer>
    </div>
  </div>
</Modal>

<style>
  .dialog {
    width: 860px;
    height: 600px;
    max-width: 100%;
    max-height: 100%;
    display: flex;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--bg);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .nav {
    width: 176px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
    background: var(--panel);
    border-right: 1px solid var(--border);
  }
  .nav .title {
    padding: 8px 8px 12px;
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .page {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .browser {
    position: absolute;
    inset: 0;
  }
  .browser[hidden] {
    display: none;
  }
  .custom {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px;
  }
  .field-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .caption {
    font-size: 12px;
    font-weight: 500;
    color: var(--muted);
  }
  .loader-row {
    gap: 12px;
  }
  .versions-block {
    flex: 1;
    min-height: 0;
  }
  .toggle {
    flex: none;
    height: 30px;
    padding: 0 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    font-size: 13px;
    color: var(--muted);
    cursor: pointer;
  }
  .toggle:hover {
    background: var(--hover);
  }
  .toggle.on {
    background: var(--accent-soft);
    border-color: var(--accent-edge);
    color: var(--accent);
  }
  .version-list {
    flex: 1;
    min-height: 160px;
    overflow-y: auto;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
  }
  .placeholder {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 16px;
    font-size: 13px;
    color: var(--muted);
    text-align: center;
  }
  .version {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .version:hover {
    background: var(--hover);
  }
  .version.selected {
    background: var(--accent-soft);
    font-weight: 500;
  }
  .kind {
    width: 64px;
    text-align: right;
    font-weight: 400;
  }
  .import {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 20px;
  }
  .drop {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border-radius: 8px;
    border: 2px dashed var(--border);
    background: var(--bg);
    cursor: pointer;
  }
  .drop:hover {
    border-color: var(--accent-edge);
  }
  .drop.over {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .format {
    display: flex;
    gap: 8px;
  }
  .format > :first-child {
    width: 150px;
    flex: none;
  }
  .key-page {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 12px;
    padding: 0 40px;
  }
  .key-page .link {
    align-self: flex-start;
  }
  .footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border);
  }
</style>
