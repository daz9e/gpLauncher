<script lang="ts">
  // Details and actions of the selected instance.
  import { launcher } from '../../lib/store.svelte';
  import { actions } from '../../lib/actions';
  import { api } from '../../lib/api';
  import { ago, duration, isError, loaderText } from '../../lib/format';
  import type { Instance, InstancePage } from '../../lib/types';
  import Button from '../../components/Button.svelte';
  import Icon from '../../components/Icon.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import ProgressBar from '../../components/ProgressBar.svelte';

  let { inst }: { inst: Instance } = $props();

  let snap = $derived(launcher.snap!);
  let session = $derived(snap.sessions[inst.id]);
  let phase = $derived(session?.phase);
  let running = $derived(phase === 'preparing' || phase === 'running');
  let manual = $derived(snap.manual_downloads.includes(inst.id));
  let memory = $derived(inst.memory_mb ?? snap.settings.memory_mb);
  const sep = (p: string) => (p.includes('\\') ? '\\' : '/');

  const links: [string, string, InstancePage][] = [
    ['puzzle', 'Mods', 'mods'],
    ['image', 'Resource packs', 'resourcepacks'],
    ['earth', 'Worlds', 'worlds'],
    ['terminal', 'Console & logs', 'console'],
    ['settings', 'Settings', 'settings'],
  ];
</script>

<aside class="sidebar" aria-label="Selected instance">
  <div class="head">
    <Thumb src={inst.icon} size={76} {running} edge />
    <div class="titles">
      <div class="semibold title">{inst.name}</div>
      <div class="xs muted">{inst.description}</div>
    </div>
  </div>
  <div class="buttons">
    {#if phase === 'running'}
      <Button icon="stop" label="Stop" variant="stop" class="big" onclick={() => actions.kill(inst.id)} />
    {:else if phase === 'preparing'}
      <Button label="Starting…" disabled class="big" />
    {:else}
      <Button icon="play" label="Play" variant="play" class="big" onclick={() => actions.launch(inst.id)} />
    {/if}
    <Button
      label="Open"
      title="Mods, worlds, console and settings (⌘O)"
      class="tall"
      onclick={() => actions.open(inst.id, 'mods')}
    />
  </div>
  {#if session}
    {@const error = isError(session.status)}
    <div class="session" class:error>
      <div class="xs status">{session.status}</div>
      {#if session.progress}<ProgressBar ratio={session.progress[0] / session.progress[1]} />{/if}
      <button class="link" onclick={() => actions.open(inst.id, 'console')}>
        {phase === 'finished' ? 'Show the log' : 'Show the console'}
      </button>
    </div>
  {/if}
  <nav class="list">
    {#each links as [icon, label, page] (page)}
      <button class="nav-item" onclick={() => actions.open(inst.id, page)}>
        <Icon name={icon} size={15} class="icon" />{label}
      </button>
    {/each}
  </nav>
  <nav class="list">
    <button class="nav-item" onclick={() => actions.openFolder(inst.game_dir)}>
      <Icon name="folder" size={15} class="icon" />Open folder
    </button>
    <button class="nav-item" onclick={() => actions.export(inst.id)}>
      <Icon name="share" size={15} class="icon" />Export…
    </button>
    <button class="nav-item" onclick={() => actions.duplicate(inst.id)}>
      <Icon name="copy" size={15} class="icon" />Duplicate
    </button>
    <button class="nav-item" onclick={() => actions.shortcut(inst.id)}>
      <Icon name="shortcut" size={15} class="icon" />Desktop shortcut
    </button>
    <button class="nav-item delete" disabled={running} onclick={() => actions.delete(inst)}>
      <Icon name="trash" size={15} class="icon" />Delete
    </button>
  </nav>
  <dl class="details xs">
    <div><dt>Minecraft</dt><dd class="truncate">{inst.minecraft}</dd></div>
    <div><dt>Loader</dt><dd class="truncate">{loaderText(inst)}</dd></div>
    <div><dt>Memory</dt><dd>{memory} MB</dd></div>
    <div><dt>Last played</dt><dd>{ago(inst.last_played, launcher.now)}</dd></div>
    {#if inst.play_time > 0}
      <div><dt>Time played</dt><dd>{duration(inst.play_time)}</dd></div>
    {/if}
  </dl>
  {#if manual}
    <div class="manual xs">
      <div>Some mods must be downloaded by hand</div>
      <button class="link" onclick={() => api.openFile(`${inst.dir}${sep(inst.dir)}MANUAL_DOWNLOADS.txt`)}>
        Show the list
      </button>
    </div>
  {/if}
</aside>

<style>
  .sidebar {
    width: 264px;
    flex: none;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    background: var(--panel);
    border-left: 1px solid var(--border);
  }
  .head {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 20px 16px 16px;
  }
  .titles {
    width: 100%;
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    overflow-wrap: anywhere;
  }
  .buttons {
    display: flex;
    gap: 8px;
    padding: 0 16px;
  }
  .buttons :global(.big) {
    flex: 1;
    height: 36px;
  }
  .buttons :global(.tall) {
    height: 36px;
  }
  .session {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 12px 16px 0;
    padding: 10px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .session .status {
    color: var(--muted);
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .session.error {
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
  }
  .session.error .status {
    color: var(--danger);
  }
  .session .link {
    align-self: flex-start;
  }
  .list {
    display: flex;
    flex-direction: column;
    margin: 12px 8px 0;
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }
  .list + .list {
    margin-top: 0;
  }
  .delete:not(:disabled) {
    color: var(--danger);
  }
  .delete:not(:disabled) :global(.icon) {
    color: var(--danger);
  }
  .nav-item:disabled {
    color: var(--subtle);
    cursor: default;
    background: none;
  }
  .details {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0 16px;
    padding: 12px 0 16px;
    border-top: 1px solid var(--border);
  }
  .details div {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
  }
  .manual {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0 16px 16px;
    padding: 12px;
    border-radius: 6px;
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
    background: color-mix(in srgb, var(--warning) 8%, transparent);
  }
  .manual .link {
    align-self: flex-start;
  }
</style>
