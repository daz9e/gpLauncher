<script lang="ts">
  // The instances as tiles, grouped by their group, sorted by last play or name.
  import { launcher } from '../../lib/store.svelte';
  import { actions } from '../../lib/actions';
  import { shortDescription } from '../../lib/format';
  import type { Instance } from '../../lib/types';
  import Dropdown from '../../components/Dropdown.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Button from '../../components/Button.svelte';
  import Icon from '../../components/Icon.svelte';
  import Thumb from '../../components/Thumb.svelte';

  let {
    query,
    selected,
    onselect,
    oncontextmenu,
    onadd,
  }: {
    query: string;
    selected: string | null;
    onselect: (id: string) => void;
    oncontextmenu: (inst: Instance, e: MouseEvent) => void;
    onadd: () => void;
  } = $props();

  let sort = $state<'recent' | 'name'>('recent');
  let collapsed = $state<Record<string, boolean>>({});
  let instances = $derived(launcher.snap!.instances);

  let visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = instances.filter(
      (i) =>
        !q ||
        i.name.toLowerCase().includes(q) ||
        i.minecraft.toLowerCase().includes(q) ||
        i.group.toLowerCase().includes(q),
    );
    if (sort === 'name') list.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    return list;
  });

  // Ungrouped instances first, then groups by name.
  let groups = $derived.by(() => {
    const map = new Map<string, Instance[]>();
    for (const inst of visible) map.set(inst.group, [...(map.get(inst.group) ?? []), inst]);
    return [...map.entries()].sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a < b ? -1 : a > b ? 1 : 0));
  });
  let grouped = $derived(groups.some(([g]) => g !== ''));

  function click(inst: Instance, e: MouseEvent) {
    onselect(inst.id);
    if (e.detail === 2) actions.launch(inst.id);
  }
</script>

{#snippet tile(inst: Instance)}
  {@const phase = launcher.snap!.sessions[inst.id]?.phase}
  {@const active = phase === 'preparing' || phase === 'running'}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="tile"
    class:selected={selected === inst.id}
    role="button"
    tabindex="-1"
    aria-label={inst.name}
    data-id={inst.id}
    onclick={(e) => click(inst, e)}
    oncontextmenu={(e) => {
      e.preventDefault();
      oncontextmenu(inst, e);
    }}
  >
    <div class="picture">
      <Thumb src={inst.icon} size={68} running={active} edge />
      {#if !active}
        <button
          class="play"
          title="Play"
          aria-label="Play {inst.name}"
          onclick={(e) => {
            e.stopPropagation();
            onselect(inst.id);
            actions.launch(inst.id);
          }}><Icon name="play" size={13} /></button
        >
      {/if}
    </div>
    <div class="name sm medium">{inst.name}</div>
    {#if phase === 'running'}
      <div class="xs playing medium">Playing</div>
    {:else if active}
      <div class="xs starting">Starting…</div>
    {:else}
      <div class="xs muted">{shortDescription(inst)}</div>
    {/if}
  </div>
{/snippet}

{#if instances.length === 0}
  <div class="grid-area">
    <EmptyState
      icon="box"
      title="No instances yet"
      detail="An instance is a separate game folder with its own version, mods and worlds. Create one, or drop a modpack (.mrpack, .zip) here."
    >
      <div style="margin-top: 8px"><Button icon="plus" label="Add Instance" variant="primary" onclick={onadd} /></div>
    </EmptyState>
  </div>
{:else if visible.length === 0}
  <div class="grid-area">
    <EmptyState icon="search" title="No instances match" detail="Try another name or version." />
  </div>
{:else}
  <div class="grid-area scroll">
    <div class="header">
      <span class="semibold">Instances</span>
      <span class="sm subtle">{visible.length}</span>
      <span class="grow"></span>
      <Dropdown
        label="Sort"
        value={sort === 'recent' ? 'Last played' : 'Name'}
        options={[
          { label: 'Last played', selected: sort === 'recent' },
          { label: 'Name', selected: sort === 'name' },
        ]}
        onselect={(i) => (sort = i === 0 ? 'recent' : 'name')}
      />
    </div>
    <div class="sections">
      {#each groups as [group, list] (group)}
        {#if grouped}
          <section>
            <button
              class="group-head"
              aria-expanded={!collapsed[group]}
              onclick={() => (collapsed[group] = !collapsed[group])}
            >
              <Icon name={collapsed[group] ? 'chevron-right' : 'chevron-down'} size={14} class="muted" />
              <span class="sm semibold">{group || 'Ungrouped'}</span>
              <span class="xs subtle">{list.length}</span>
              <span class="rule"></span>
            </button>
            {#if !collapsed[group]}
              <div class="tiles">
                {#each list as inst (inst.id)}{@render tile(inst)}{/each}
              </div>
            {/if}
          </section>
        {:else}
          <div class="tiles">
            {#each list as inst (inst.id)}{@render tile(inst)}{/each}
          </div>
        {/if}
      {/each}
    </div>
  </div>
{/if}

<style>
  .grid-area {
    flex: 1;
    min-width: 0;
    padding: 16px 20px;
  }
  .scroll {
    overflow-y: auto;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }
  .sections {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .group-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
    cursor: pointer;
  }
  .rule {
    flex: 1;
    height: 1px;
    margin-left: 4px;
    background: var(--border);
  }
  .tiles {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 8px;
  }
  .tile {
    width: 132px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 12px 8px 10px;
    border-radius: 8px;
    border: 1px solid transparent;
    cursor: pointer;
    outline: none;
  }
  .tile:hover {
    background: var(--hover);
  }
  .tile.selected {
    background: var(--accent-soft);
    border-color: var(--accent-edge);
  }
  .picture {
    position: relative;
    margin-bottom: 6px;
  }
  .play {
    position: absolute;
    right: -6px;
    bottom: -6px;
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: var(--success);
    border: 2px solid var(--bg);
    color: var(--on-accent);
    box-shadow: 0 2px 6px rgb(0 0 0 / 0.2);
    visibility: hidden;
    cursor: pointer;
  }
  .tile:hover .play {
    visibility: visible;
  }
  .play:hover {
    opacity: 0.9;
  }
  .name {
    width: 100%;
    text-align: center;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  /* The selected tile shows the whole name. */
  .selected .name {
    display: block;
  }
  .playing {
    color: var(--success);
  }
  .starting {
    color: var(--accent);
  }
</style>
