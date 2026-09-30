<script lang="ts">
  // Settings of one instance: name, group and icon, game and loader versions, Java and window.
  // Every valid change is saved right away; fields left empty follow the launcher settings.
  import { onMount, untrack } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { actions } from '../../lib/actions';
  import { LOADER_LABELS, MEMORY_PRESETS, memoryLabel, parseMemory, parseResolution } from '../../lib/format';
  import type { InstanceData, Loader, LoaderVersion } from '../../lib/types';
  import Button from '../../components/Button.svelte';
  import Dropdown from '../../components/Dropdown.svelte';
  import Segments from '../../components/Segments.svelte';
  import TextField from '../../components/TextField.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import JavaField from '../settings/JavaField.svelte';

  let { id }: { id: string } = $props();

  const LOADERS: Loader[] = ['vanilla', 'fabric', 'quilt', 'forge', 'neoforge'];
  type Lookup<T> = { ok: true; value: T } | { ok: false; error: string } | undefined;

  let inst = $derived(launcher.instance(id)!);
  let settings = $derived(launcher.snap!.settings);
  let running = $derived(launcher.running(id));
  const start = untrack(() => launcher.instance(id)!);

  let name = $state(start.name);
  let group = $state(start.group);
  let java = $state(start.java_path);
  let memory = $state(start.memory_mb?.toString() ?? '');
  let jvmArgs = $state(start.jvm_args);
  let width = $state(start.window_width?.toString() ?? '');
  let height = $state(start.window_height?.toString() ?? '');
  let releases = $state<string[]>([]);
  let builds = $state<{ key: string; list: Lookup<LoaderVersion[]> } | null>(null);
  let memoryError = $state<string | null>(null);
  let javaError = $state<string | null>(null);
  let sizeError = $state<string | null>(null);
  let saveError = $state<string | null>(null);

  onMount(() => {
    api.listVersions().then(
      (list) => (releases = list.filter((v) => v.kind === 'release').map((v) => v.id)),
      () => {},
    );
  });

  // Loads the loader versions for the current loader and game version, once.
  $effect(() => {
    if (!inst || inst.loader === 'vanilla') return;
    const key = `${inst.loader}:${inst.minecraft}`;
    if (builds?.key === key) return;
    builds = { key, list: undefined };
    api.loaderVersions(inst.loader, inst.minecraft).then(
      (value) => builds?.key === key && (builds = { key, list: { ok: true, value } }),
      (e) => builds?.key === key && (builds = { key, list: { ok: false, error: message(e) } }),
    );
  });

  function data(): InstanceData {
    const i = launcher.instance(id)!;
    return {
      name: i.name,
      minecraft: i.minecraft,
      loader: i.loader,
      loader_version: i.loader_version,
      memory_mb: i.memory_mb,
      jvm_args: i.jvm_args,
      java_path: i.java_path,
      window_width: i.window_width,
      window_height: i.window_height,
      fullscreen: i.fullscreen,
      last_played: i.last_played,
      play_time: i.play_time,
      group: i.group,
    };
  }

  async function save(d: InstanceData) {
    try {
      await api.saveInstance(id, d);
      saveError = null;
    } catch (e) {
      saveError = message(e);
    }
  }

  let chain = Promise.resolve();
  /** Takes every valid field into the instance and saves it, one change at a time. */
  function commit(): Promise<void> {
    chain = chain.then(apply);
    return chain;
  }

  async function apply() {
    const d = data();
    d.name = name.trim() || d.minecraft;
    d.group = group.trim();
    const m = parseMemory(memory);
    memoryError = m.ok ? null : m.error;
    if (m.ok) d.memory_mb = m.mb;
    const path = java.trim();
    javaError = null;
    if (!path || (await api.isFile(path))) d.java_path = path;
    else javaError = `No Java at ${path}`;
    d.jvm_args = jvmArgs.trim();
    const r = parseResolution(width, height);
    sizeError = r.ok ? null : r.error;
    if (r.ok) [d.window_width, d.window_height] = [r.width, r.height];
    await save(d);
  }

  function change(f: (d: InstanceData) => void) {
    chain = chain.then(() => {
      const d = data();
      f(d);
      return save(d);
    });
  }

  function setLoader(loader: Loader) {
    change((d) => {
      if (d.loader !== loader) {
        d.loader = loader;
        d.loader_version = '';
      }
    });
  }

  function setMinecraft(mc: string) {
    change((d) => {
      if (d.minecraft !== mc) {
        d.minecraft = mc;
        d.loader_version = '';
      }
    });
  }

  async function pickIcon() {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'] }],
    });
    if (typeof path !== 'string') return;
    try {
      await api.setInstanceIcon(id, path);
      saveError = null;
    } catch (e) {
      saveError = message(e);
    }
  }

  async function clearIcon() {
    try {
      await api.clearInstanceIcon(id);
    } catch (e) {
      saveError = message(e);
    }
  }

  let globalJava = $derived(settings.java_path.trim() || 'Automatic');
  let [defaultWidth, defaultHeight] = $derived(
    settings.window_width && settings.window_height ? [settings.window_width, settings.window_height] : [854, 480],
  );
  let buildList = $derived(builds?.list?.ok ? builds.list.value.slice(0, 80) : []);
</script>

{#if inst}
  <div class="scroll">
    <div class="column">
      {#if saveError}<div class="error">{saveError}</div>{/if}
      <div class="heading">GENERAL</div>
      <div class="group">
        <div class="srow">
          <div class="label"><span class="sm">Icon</span><span class="hint">PNG or JPEG; shown in the launcher.</span></div>
          <div class="row">
            {#if inst.icon}<Button label="Remove" variant="ghost" onclick={clearIcon} />{/if}
            <Button label="Choose…" onclick={pickIcon} />
            <Thumb src={inst.icon} size={56} radius={0.14} edge />
          </div>
        </div>
        <div class="stacked">
          <span class="sm">Name</span>
          <TextField bind:value={name} placeholder={inst.minecraft} label="Name" oninput={commit} />
        </div>
        <div class="stacked">
          <span class="sm">Group</span>
          <TextField bind:value={group} placeholder="Ungrouped" label="Group" oninput={commit} />
          <span class="hint">Instances with the same group are shown together.</span>
        </div>
        <div class="srow">
          <div class="label">
            <span class="sm">Game folder</span>
            <span class="hint truncate selectable" title={inst.game_dir}>{inst.game_dir}</span>
          </div>
          <Button icon="folder" label="Open" onclick={() => actions.openFolder(inst.game_dir)} />
        </div>
      </div>

      <div class="heading top">VERSION</div>
      <div class="col gap">
        <div class="group">
          <div class="srow">
            <div class="label sm">Minecraft version</div>
            <Dropdown
              label="Minecraft"
              value={inst.minecraft}
              disabled={running}
              options={releases.map((v) => ({ label: v, selected: v === inst.minecraft }))}
              onselect={(i) => setMinecraft(releases[i])}
            />
          </div>
          <div class="stacked">
            <span class="sm">Mod loader</span>
            <Segments
              label="Mod loader"
              disabled={running}
              options={LOADERS.map((l) => ({ value: l, label: l === 'vanilla' ? 'None' : LOADER_LABELS[l] }))}
              value={inst.loader}
              onchange={setLoader}
            />
          </div>
          {#if inst.loader !== 'vanilla'}
            <div class="srow">
              <div class="label">
                <span class="sm">Loader version</span>
                {#if builds?.list === undefined}
                  <span class="hint">Loading versions…</span>
                {:else if !builds.list.ok}
                  <span class="error">Could not load versions: {builds.list.error}</span>
                {:else if builds.list.value.length === 0}
                  <span class="error">{LOADER_LABELS[inst.loader]} has no builds for Minecraft {inst.minecraft}</span>
                {:else}
                  <span class="hint">
                    {inst.loader_version
                      ? 'Installed on the next launch if needed.'
                      : 'The newest stable build is installed on the next launch.'}
                  </span>
                {/if}
              </div>
              <Dropdown
                label="Version"
                value={inst.loader_version || 'Latest stable'}
                disabled={running}
                options={[
                  { label: 'Latest stable', selected: !inst.loader_version },
                  ...buildList.map((b) => ({
                    label: b.stable ? b.version : `${b.version} (beta)`,
                    selected: b.version === inst.loader_version,
                  })),
                ]}
                onselect={(i) => change((d) => (d.loader_version = i === 0 ? '' : buildList[i - 1].version))}
              />
            </div>
          {/if}
        </div>
        <div class="hint pad">
          {running
            ? 'Close the game to change versions.'
            : 'Changing versions can break worlds and mods: make a copy of the instance first if unsure.'}
        </div>
      </div>

      <div class="heading top">JAVA</div>
      <div class="group">
        <div class="stacked">
          <span class="sm">Java executable</span>
          <JavaField
            bind:value={java}
            placeholder="Default ({globalJava})"
            emptyHint="Uses the Java from the launcher settings. Choose a binary only if this instance needs a different one."
            resetLabel="Use the default Java"
            onchange={commit}
          />
          {#if javaError}<div class="error">{javaError}</div>{/if}
        </div>
        <div class="srow">
          <div class="label">
            <span class="sm">Memory</span>
            {#if memoryError}<span class="error">{memoryError}</span>{:else}<span class="hint">Empty = the launcher setting.</span>{/if}
          </div>
          <div class="row memory">
            {#each MEMORY_PRESETS as mb (mb)}
              <button
                class="chip"
                class:active={memory.trim() === String(mb)}
                onclick={() => {
                  memory = String(mb);
                  commit();
                }}>{memoryLabel(mb)}</button
              >
            {/each}
            <div class="small-input">
              <TextField bind:value={memory} placeholder={String(settings.memory_mb)} label="Memory in MB" oninput={commit} />
            </div>
            <span class="sm muted">MB</span>
          </div>
        </div>
        <div class="stacked">
          <span class="sm">JVM arguments</span>
          <TextField bind:value={jvmArgs} placeholder="None" label="JVM arguments" oninput={commit} />
          <span class="hint">
            {settings.jvm_args.trim()
              ? `Added after the launcher's arguments: ${settings.jvm_args.trim()}`
              : 'Added to the JVM arguments from the launcher settings.'}
          </span>
        </div>
      </div>

      <div class="heading top">GAME WINDOW</div>
      <div class="group">
        <div class="srow">
          <div class="label">
            <span class="sm">Window size</span>
            {#if sizeError}<span class="error">{sizeError}</span>{:else}<span class="hint">Empty = the launcher setting.</span>{/if}
          </div>
          <div class="row">
            <div class="small-input"><TextField bind:value={width} placeholder={String(defaultWidth)} label="Width" oninput={commit} /></div>
            <span class="sm muted">×</span>
            <div class="small-input"><TextField bind:value={height} placeholder={String(defaultHeight)} label="Height" oninput={commit} /></div>
          </div>
        </div>
        <div class="srow">
          <div class="label">
            <span class="sm">Display</span>
            {#if inst.fullscreen === null}
              <span class="hint">Follows the launcher: {settings.fullscreen ? 'fullscreen' : 'windowed'}.</span>
            {/if}
          </div>
          <Segments
            label="Display"
            options={[
              { value: null as boolean | null, label: 'Default' },
              { value: false as boolean | null, label: 'Windowed' },
              { value: true as boolean | null, label: 'Fullscreen' },
            ]}
            value={inst.fullscreen}
            onchange={(mode) => change((d) => (d.fullscreen = mode))}
          />
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .scroll {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
  }
  .column {
    width: 100%;
    max-width: 680px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px 24px;
  }
  .top {
    margin-top: 12px;
  }
  .gap {
    gap: 8px;
  }
  .pad {
    padding: 0 4px;
  }
  .memory {
    gap: 6px;
  }
</style>
