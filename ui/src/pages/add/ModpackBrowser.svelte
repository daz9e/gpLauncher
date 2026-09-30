<script lang="ts">
  // Searchable list of modpacks on one platform, with details and versions of the selected one.
  import { onMount, tick } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api, message } from '../../lib/api';
  import { whenVisible } from '../../lib/visible';
  import { LOADER_LABELS, oneLine, shortNumber } from '../../lib/format';
  import type { Filters, Loader, Pack, PackVersion, Platform, Sort } from '../../lib/types';
  import TextField from '../../components/TextField.svelte';
  import Dropdown from '../../components/Dropdown.svelte';
  import Thumb from '../../components/Thumb.svelte';

  let {
    platform,
    releases,
    active,
    onselection,
  }: {
    platform: Platform;
    /** Minecraft releases offered by the version filter, newest first. */
    releases: string[];
    active: boolean;
    /** What the Install button would install. */
    onselection: (pick: [Pack, PackVersion] | null) => void;
  } = $props();

  const PAGE_SIZE = 25;
  const SEARCH_DELAY = 350;
  const LOADERS: Loader[] = ['fabric', 'forge', 'neoforge', 'quilt'];
  const SORTS: [Sort, string][] = [
    ['relevance', 'Relevance'],
    ['downloads', 'Downloads'],
    ['updated', 'Recently updated'],
    ['newest', 'Newest'],
  ];
  type Lookup<T> = { ok: true; value: T } | { ok: false; error: string } | undefined;

  let query = $state('');
  let packs = $state<Pack[]>([]);
  let total = $state(0);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let filters = $state<Filters>({ game_version: null, loader: null, sort: 'relevance' });
  let selected = $state<number | null>(null);
  let versions = $state<Record<string, Lookup<PackVersion[]>>>({});
  let selectedVersion = $state(0);
  let versionQuery = $state('');
  let searchInput = $state<HTMLInputElement>();
  let versionInput = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();
  let versionList = $state<HTMLDivElement>();
  /** Bumped by every new query so results of older ones are dropped. */
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  let pack = $derived(selected !== null ? packs[selected] : undefined);
  let packVersions = $derived(pack ? versions[pack.id] : undefined);
  let matching = $derived.by(() => {
    if (!packVersions?.ok) return null;
    const q = versionQuery.trim().toLowerCase();
    return packVersions.value.filter(
      (v) =>
        (!filters.game_version || v.game_versions.includes(filters.game_version)) &&
        (!filters.loader || v.loaders.some((l) => l.toLowerCase() === LOADER_LABELS[filters.loader!].toLowerCase())) &&
        (!q || v.name.toLowerCase().includes(q) || v.game_versions.some((g) => g.toLowerCase().includes(q))),
    );
  });

  $effect(() => {
    const v = matching?.[selectedVersion];
    onselection(pack && v ? [$state.snapshot(pack), $state.snapshot(v)] : null);
  });

  $effect(() => {
    if (active) searchInput?.focus();
  });

  function runSearch() {
    generation++;
    packs = [];
    total = 0;
    selected = null;
    fetchPage(0);
  }

  function loadMore() {
    if (!loading && !error && packs.length < total) fetchPage(packs.length);
  }

  async function fetchPage(offset: number) {
    loading = true;
    error = null;
    const gen = generation;
    try {
      const page = await api.searchModpacks(platform, query, $state.snapshot(filters), offset, PAGE_SIZE);
      if (gen !== generation) return;
      total = page.total;
      packs = [...packs, ...page.packs];
      if (selected === null && packs.length) select(0);
    } catch (e) {
      if (gen === generation) error = message(e);
    } finally {
      if (gen === generation) loading = false;
    }
  }

  async function select(ix: number, reveal = false) {
    const p = packs[ix];
    if (!p) return;
    selected = ix;
    selectedVersion = 0;
    versionQuery = '';
    await tick();
    if (reveal) listEl?.querySelector(`[data-ix="${ix}"]`)?.scrollIntoView({ block: 'nearest' });
    versionList?.scrollTo({ top: 0 });
    if (p.id in versions) return;
    versions[p.id] = undefined;
    api.modpackVersions(platform, p.id).then(
      (value) => {
        versions[p.id] = { ok: true, value };
      },
      (e) => {
        versions[p.id] = { ok: false, error: message(e) };
      },
    );
  }

  /** Moves through the versions while their filter is focused, through the packs otherwise. */
  export function move(delta: number) {
    if (document.activeElement === versionInput) {
      const count = matching?.length ?? 0;
      if (count) {
        selectedVersion = Math.min(Math.max(selectedVersion + delta, 0), count - 1);
        tick().then(() => versionList?.querySelector('.selected')?.scrollIntoView({ block: 'nearest' }));
      }
      return;
    }
    if (!packs.length) return;
    select(selected === null ? 0 : Math.min(Math.max(selected + delta, 0), packs.length - 1), true);
  }

  function updateFilters(change: Partial<Filters>) {
    const next = { ...filters, ...change };
    if (JSON.stringify(next) === JSON.stringify(filters)) return;
    filters = next;
    selectedVersion = 0;
    runSearch();
  }


  onMount(() => {
    runSearch();
    return () => clearTimeout(timer);
  });

  function typed() {
    // Debounce: a new keystroke restarts the timer.
    clearTimeout(timer);
    timer = setTimeout(runSearch, SEARCH_DELAY);
  }

  function meta(v: PackVersion): string {
    const game = v.game_versions[0] ?? '';
    const loader = v.loaders[0];
    return loader ? (game ? `${game} · ${loader}` : loader) : game;
  }

  let count = $derived(total === 0 && loading ? '' : `${shortNumber(total)} results`);
</script>

<div class="browser">
  <div class="top">
    <div class="row">
      <div class="grow">
        <TextField bind:value={query} bind:input={searchInput} placeholder="Search modpacks" oninput={typed} />
      </div>
      <span class="xs subtle">{count}</span>
    </div>
    <div class="row filters">
      <Dropdown
        label="Minecraft"
        value={filters.game_version ?? 'Any'}
        options={[
          { label: 'Any', selected: !filters.game_version },
          ...releases.map((v) => ({ label: v, selected: filters.game_version === v })),
        ]}
        onselect={(i) => updateFilters({ game_version: i === 0 ? null : releases[i - 1] })}
      />
      <Dropdown
        label="Loader"
        value={filters.loader ? LOADER_LABELS[filters.loader] : 'Any'}
        options={[
          { label: 'Any', selected: !filters.loader },
          ...LOADERS.map((l) => ({ label: LOADER_LABELS[l], selected: filters.loader === l })),
        ]}
        onselect={(i) => updateFilters({ loader: i === 0 ? null : LOADERS[i - 1] })}
      />
      <Dropdown
        label="Sort"
        value={SORTS.find(([s]) => s === filters.sort)![1]}
        options={SORTS.map(([s, label]) => ({ label, selected: s === filters.sort }))}
        onselect={(i) => updateFilters({ sort: SORTS[i][0] })}
      />
      {#if filters.game_version || filters.loader}
        <button class="link" onclick={() => updateFilters({ game_version: null, loader: null })}>Reset</button>
      {/if}
    </div>
  </div>
  <div class="split">
    <div class="list" bind:this={listEl} role="listbox" aria-label="Modpacks">
      {#if packs.length === 0}
        <div class="message">
          {#if error}Could not load modpacks: {error}{:else if loading}Loading…{:else}Nothing found{/if}
        </div>
      {:else}
        {#each packs as p, ix (p.id + ix)}
          <button
            class="pack"
            class:selected={selected === ix}
            data-ix={ix}
            role="option"
            aria-selected={selected === ix}
            onclick={() => select(ix)}
          >
            <Thumb url={p.icon_url} size={40} radius={0.2} />
            <span class="grow col">
              <span class="row title-line">
                <span class="truncate sm medium">{p.title}</span>
                <span class="xs subtle nowrap">{shortNumber(p.downloads)} downloads</span>
              </span>
              <span class="truncate xs muted">{oneLine(p.summary)}</span>
            </span>
          </button>
        {/each}
        {#if packs.length < total || error}
          <div class="more xs muted" use:whenVisible={loadMore}>{error ? `Could not load more: ${error}` : 'Loading more…'}</div>
        {/if}
      {/if}
    </div>
    <aside class="details">
      {#if pack}
        <div class="about">
          <div class="row head">
            <Thumb url={pack.icon_url} size={48} radius={0.2} />
            <div class="grow col">
              <span class="semibold clamp">{pack.title}</span>
              {#if pack.author}<span class="truncate xs muted">by {pack.author}</span>{/if}
            </div>
          </div>
          <div class="xs muted summary selectable">{pack.summary}</div>
          {#if pack.website}
            <button class="link" onclick={() => openUrl(pack!.website!)}>
              Open on {platform === 'modrinth' ? 'Modrinth' : 'CurseForge'}
            </button>
          {/if}
        </div>
        <div class="versions-head xs medium muted">Version</div>
        {#if packVersions?.ok && packVersions.value.length > 1}
          <div class="version-filter">
            <TextField
              bind:value={versionQuery}
              bind:input={versionInput}
              placeholder="Filter versions"
              icon="search"
              oninput={() => (selectedVersion = 0)}
            />
          </div>
        {/if}
        {#if packVersions === undefined}
          <div class="note">Loading…</div>
        {:else if !packVersions.ok}
          <div class="note">Could not load versions: {packVersions.error}</div>
        {:else if packVersions.value.length === 0}
          <div class="note">No versions available</div>
        {:else if !matching?.length}
          <div class="note">No versions match the filters</div>
        {:else}
          <div class="versions" bind:this={versionList} role="listbox" aria-label="Versions">
            {#each matching as v, i (v.id)}
              <button
                class="version"
                class:selected={i === selectedVersion}
                role="option"
                aria-selected={i === selectedVersion}
                onclick={() => (selectedVersion = i)}
              >
                <span class="grow truncate">{v.name}</span>
                <span class="muted nowrap">{meta(v)}</span>
              </button>
            {/each}
          </div>
        {/if}
      {/if}
    </aside>
  </div>
</div>

<style>
  .browser {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    gap: 12px;
  }
  .filters {
    gap: 8px;
  }
  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .message {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 24px;
    font-size: 13px;
    color: var(--muted);
    text-align: center;
  }
  .pack {
    width: 100%;
    height: 60px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    text-align: left;
    cursor: pointer;
    overflow: hidden;
  }
  .pack:hover {
    background: var(--hover);
  }
  .pack.selected {
    background: var(--accent-soft);
  }
  .title-line {
    gap: 6px;
    align-items: baseline;
  }
  .nowrap {
    flex: none;
    white-space: nowrap;
  }
  .more {
    height: 60px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .details {
    width: 250px;
    flex: none;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
  }
  .about {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px;
  }
  .head {
    gap: 12px;
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .summary {
    max-height: 120px;
    overflow-y: auto;
  }
  .about .link {
    align-self: flex-start;
  }
  .versions-head {
    padding: 12px 16px 6px;
    border-top: 1px solid var(--border);
  }
  .version-filter {
    padding: 0 16px 8px;
  }
  .note {
    padding: 8px 16px;
    font-size: 12px;
    color: var(--muted);
  }
  .versions {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .version {
    width: 100%;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    font-size: 12px;
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
</style>
