<script lang="ts">
  // Modrinth search for mods, resource packs or shaders that fit one instance, with the versions
  // of the selected project. Installing is left to the owner through `oninstall`.
  import { onMount, tick } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api, message } from '../../lib/api';
  import { whenVisible } from '../../lib/visible';
  import { KIND_LABELS, MODRINTH_TYPES, capitalize, oneLine, shortNumber } from '../../lib/format';
  import type { AddonScope, Kind, Project, Sort, Version } from '../../lib/types';
  import Button from '../../components/Button.svelte';
  import IconButton from '../../components/IconButton.svelte';
  import Icon from '../../components/Icon.svelte';
  import Dropdown from '../../components/Dropdown.svelte';
  import TextField from '../../components/TextField.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Thumb from '../../components/Thumb.svelte';

  let {
    id,
    kind,
    active,
    installed,
    queued,
    oninstall,
  }: {
    id: string;
    kind: Kind;
    active: boolean;
    installed: Set<string>;
    queued: Set<string>;
    /** `version` = null installs the newest fitting version. */
    oninstall: (project: Project, version: Version | null) => void;
  } = $props();

  const PAGE_SIZE = 30;
  const SEARCH_DELAY = 300;
  const SORTS: [Sort, string][] = [
    ['relevance', 'Relevance'],
    ['downloads', 'Downloads'],
    ['updated', 'Recently updated'],
    ['newest', 'Newest'],
  ];
  type Lookup<T> = { ok: true; value: T } | { ok: false; error: string } | undefined;

  let scope = $state<AddonScope | null>(null);
  let query = $state('');
  let sort = $state<Sort>('relevance');
  let projects = $state<Project[]>([]);
  let total = $state(0);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selected = $state<number | null>(null);
  let versions = $state<Record<string, Lookup<Version[]>>>({});
  let searchInput = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  let label = $derived(KIND_LABELS[kind].toLowerCase());
  let project = $derived(selected !== null ? projects[selected] : undefined);

  function stateOf(projectId: string): 'installed' | 'queued' | 'available' {
    return installed.has(projectId) ? 'installed' : queued.has(projectId) ? 'queued' : 'available';
  }

  $effect(() => {
    if (active) searchInput?.focus();
  });

  function runSearch() {
    generation++;
    projects = [];
    total = 0;
    selected = null;
    fetchPage(0);
  }

  async function fetchPage(offset: number) {
    if (!scope) return;
    loading = true;
    error = null;
    const gen = generation;
    try {
      const page = await api.searchProjects(kind, query, scope.game_version, scope.loaders, sort, offset, PAGE_SIZE);
      if (gen !== generation) return;
      total = page.total;
      projects = [...projects, ...page.projects];
      if (selected === null && projects.length) select(0);
    } catch (e) {
      if (gen === generation) error = message(e);
    } finally {
      if (gen === generation) loading = false;
    }
  }

  function loadMore() {
    if (!loading && !error && projects.length < total) fetchPage(projects.length);
  }

  async function select(ix: number, reveal = false) {
    const p = projects[ix];
    if (!p || !scope) return;
    selected = ix;
    await tick();
    if (reveal) listEl?.querySelector(`[data-ix="${ix}"]`)?.scrollIntoView({ block: 'nearest' });
    if (p.id in versions) return;
    versions[p.id] = undefined;
    api.projectVersions(p.id, scope.loaders, scope.game_version).then(
      (value) => {
        versions[p.id] = { ok: true, value };
      },
      (e) => {
        versions[p.id] = { ok: false, error: message(e) };
      },
    );
  }

  onMount(() => {
    api.addonScope(id, kind).then((s) => {
      scope = s;
      runSearch();
    });
    return () => clearTimeout(timer);
  });

  function typed() {
    clearTimeout(timer);
    timer = setTimeout(runSearch, SEARCH_DELAY);
  }


  let scopeText = $derived.by(() => {
    if (!scope) return '';
    const loader = scope.loaders[0];
    if (scope.game_version && loader) return `Showing ${label} for ${scope.game_version} · ${capitalize(loader)}`;
    if (scope.game_version) return `Showing ${label} for ${scope.game_version}`;
    return `Showing all ${label}`;
  });
  let countText = $derived(total === 0 && loading ? '' : `${shortNumber(total)} results`);
  let projectVersions = $derived(project ? versions[project.id] : undefined);
</script>

<div class="browser">
  <div class="top">
    <div class="grow">
      <TextField
        bind:value={query}
        bind:input={searchInput}
        placeholder="Search {label} on Modrinth"
        icon="search"
        oninput={typed}
      />
    </div>
    <Dropdown
      label="Sort"
      value={SORTS.find(([s]) => s === sort)![1]}
      options={SORTS.map(([s, l]) => ({ label: l, selected: s === sort }))}
      onselect={(i) => {
        if (SORTS[i][0] !== sort) {
          sort = SORTS[i][0];
          runSearch();
        }
      }}
    />
  </div>
  <div class="scope xs subtle"><span>{scopeText}</span><span>{countText}</span></div>
  <div class="split">
    <div class="list" bind:this={listEl}>
      {#if !projects.length}
        {#if error}
          <EmptyState icon="triangle-alert" title="Could not search Modrinth" detail={error} />
        {:else if loading || !scope}
          <EmptyState icon="search" title="Searching…" />
        {:else}
          <EmptyState icon="search" title="Nothing found" detail="Try other words." />
        {/if}
      {:else}
        {#each projects as p, ix (p.id + ix)}
          {@const state = stateOf(p.id)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="project"
            class:selected={selected === ix}
            data-ix={ix}
            role="option"
            aria-selected={selected === ix}
            tabindex="-1"
            onclick={() => select(ix)}
          >
            <Thumb url={p.icon_url} size={40} radius={0.22} fallback="puzzle" />
            <div class="grow col">
              <div class="row line">
                <span class="sm medium truncate">{p.title}</span>
                {#if p.author}<span class="xs subtle nowrap">by {p.author}</span>{/if}
              </div>
              <span class="xs muted truncate">{oneLine(p.summary)}</span>
              <span class="xs subtle">{shortNumber(p.downloads)} downloads</span>
            </div>
            {#if state === 'installed'}
              <Button icon="check" label="Installed" variant="ghost" disabled />
            {:else if state === 'queued'}
              <Button label="Queued…" variant="ghost" disabled />
            {:else}
              <Button
                icon="download"
                label="Install"
                onclick={(e) => {
                  e.stopPropagation();
                  oninstall($state.snapshot(p), null);
                }}
              />
            {/if}
          </div>
        {/each}
        {#if projects.length < total || error}
          <div class="more xs muted" use:whenVisible={loadMore}>{error ? `Could not load more: ${error}` : 'Loading more…'}</div>
        {/if}
      {/if}
    </div>
    <aside class="details">
      {#if project}
        {@const state = stateOf(project.id)}
        <div class="about">
          <div class="row head">
            <Thumb url={project.icon_url} size={48} radius={0.22} fallback="puzzle" />
            <span class="semibold clamp">{project.title}</span>
          </div>
          <div class="xs muted summary selectable">{project.summary}</div>
          <button
            class="link row open"
            onclick={() => openUrl(`https://modrinth.com/${MODRINTH_TYPES[kind]}/${project!.slug}`)}
          >
            Open on Modrinth <Icon name="external-link" size={11} />
          </button>
        </div>
        <div class="versions-head xs medium muted">Versions for this instance</div>
        {#if projectVersions === undefined}
          <div class="note">Loading…</div>
        {:else if !projectVersions.ok}
          <div class="note">Could not load versions: {projectVersions.error}</div>
        {:else if !projectVersions.value.length}
          <div class="note">No version fits this instance's Minecraft version and loader.</div>
        {:else}
          <div class="versions">
            {#each projectVersions.value as v (v.id)}
              <div class="version">
                <div class="grow col">
                  <span class="xs truncate">{v.number}</span>
                  <span class="xs subtle truncate">
                    {[v.game_versions[v.game_versions.length - 1], v.loaders[0]].filter(Boolean).join(' · ')}
                  </span>
                </div>
                <IconButton
                  icon="download"
                  title="Install this version"
                  disabled={state !== 'available'}
                  onclick={() => oninstall($state.snapshot(project!), $state.snapshot(v))}
                />
              </div>
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
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  .scope {
    flex: none;
    display: flex;
    justify-content: space-between;
    padding: 6px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
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
  .project {
    height: 64px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    overflow: hidden;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    cursor: pointer;
    outline: none;
  }
  .project:hover {
    background: var(--hover);
  }
  .project.selected {
    background: var(--accent-soft);
  }
  .line {
    gap: 6px;
    align-items: baseline;
  }
  .nowrap {
    flex: none;
    white-space: nowrap;
  }
  .more {
    height: 64px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .details {
    width: 280px;
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
  .open {
    gap: 4px;
    align-self: flex-start;
  }
  .versions-head {
    padding: 12px 16px 6px;
    border-top: 1px solid var(--border);
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
    height: 40px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
  }
  .version:hover {
    background: var(--hover);
  }
</style>
