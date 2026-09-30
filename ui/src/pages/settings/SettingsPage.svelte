<script lang="ts">
  // "Settings" page: launcher folder and behavior, Java, game window, service keys.
  // It fills the launcher window and applies every valid change right away.
  import { open } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { actions } from '../../lib/actions';
  import { MEMORY_PRESETS, memoryLabel, parseMemory, parseResolution } from '../../lib/format';
  import type { Appearance, Settings } from '../../lib/types';
  import Icon from '../../components/Icon.svelte';
  import Button from '../../components/Button.svelte';
  import Switch from '../../components/Switch.svelte';
  import Segments from '../../components/Segments.svelte';
  import TextField from '../../components/TextField.svelte';
  import JavaField from './JavaField.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const CURSEFORGE_CONSOLE = 'https://console.curseforge.com/';
  const DEFAULT_MEMORY = 4096;
  type Section = 'general' | 'java' | 'game' | 'services';
  const SECTIONS: [Section, string, string][] = [
    ['general', 'General', 'settings'],
    ['java', 'Java', 'coffee'],
    ['game', 'Game', 'monitor'],
    ['services', 'Services', 'globe'],
  ];

  const initial = $state.snapshot(launcher.snap!.settings) as Settings;
  /** The settings as last applied; fields not shown here are kept. */
  let settings = $state<Settings>(initial);
  /** The launcher folder when the page was opened, to offer going back to it. */
  const originalDir = initial.data_dir;
  let section = $state<Section>('general');
  let busy = $derived(launcher.snap!.busy);

  let java = $state(initial.java_path);
  let memory = $state(String(initial.memory_mb));
  let jvmArgs = $state(initial.jvm_args);
  let width = $state(initial.window_width?.toString() ?? '');
  let height = $state(initial.window_height?.toString() ?? '');
  let curseforgeKey = $state(initial.curseforge_api_key);
  let clientId = $state(initial.ms_client_id);
  // Fields holding a value that is not applied, and why.
  let memoryError = $state<string | null>(null);
  let javaError = $state<string | null>(null);
  let sizeError = $state<string | null>(null);
  let saveError = $state<string | null>(null);

  /** Takes every valid field into the settings and hands them to the launcher, one change at a time. */
  let chain = Promise.resolve();
  function commit(): Promise<void> {
    chain = chain.then(apply);
    return chain;
  }

  async function apply() {
    const m = parseMemory(memory);
    memoryError = m.ok ? null : m.error;
    if (m.ok) settings.memory_mb = m.mb ?? DEFAULT_MEMORY;

    const path = java.trim();
    javaError = null;
    if (!path || (await api.isFile(path))) settings.java_path = path;
    else javaError = `No Java at ${path}`;
    settings.jvm_args = jvmArgs.trim();

    const r = parseResolution(width, height);
    sizeError = r.ok ? null : r.error;
    if (r.ok) [settings.window_width, settings.window_height] = [r.width, r.height];

    settings.curseforge_api_key = curseforgeKey.trim();
    settings.ms_client_id = clientId.trim();
    try {
      await api.setSettings($state.snapshot(settings));
      saveError = null;
    } catch (e) {
      saveError = message(e);
      settings.data_dir = launcher.snap!.settings.data_dir;
    }
  }

  async function setDataDir(dir: string) {
    if (!busy && dir !== settings.data_dir) {
      settings.data_dir = dir;
      await commit();
    }
  }

  async function pickDataDir() {
    const dir = await open({ directory: true, multiple: false, defaultPath: settings.data_dir });
    if (typeof dir === 'string') setDataDir(dir);
  }

  let moved = $derived(settings.data_dir !== originalDir);
  let title = $derived(SECTIONS.find(([s]) => s === section)![1]);
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && !e.defaultPrevented && onclose()} />

<div class="settings">
  <nav class="nav">
    <button class="back nav-item" onclick={onclose}>
      <Icon name="chevron-left" size={15} class="icon" />Instances
    </button>
    {#each SECTIONS as [s, label, icon] (s)}
      <button class="nav-item" class:active={section === s} onclick={() => (section = s)}>
        <Icon name={icon} size={15} class="icon" />{label}
      </button>
    {/each}
  </nav>
  <div class="body">
    <div class="column">
      <h1>{title}</h1>
      {#if saveError}<div class="error">{saveError}</div>{/if}
      {#if section === 'general'}
        <div class="group">
          <div class="srow">
            <div class="label sm">Appearance</div>
            <div style="width: 240px">
              <Segments
                label="Appearance"
                options={[
                  { value: 'system' as Appearance, label: 'System' },
                  { value: 'light' as Appearance, label: 'Light' },
                  { value: 'dark' as Appearance, label: 'Dark' },
                ]}
                value={settings.appearance}
                onchange={(a) => {
                  settings.appearance = a;
                  commit();
                }}
              />
            </div>
          </div>
          <div class="srow">
            <div class="label">
              <span class="sm">Minimize while playing</span>
              <span class="hint">The launcher comes back when the game closes.</span>
            </div>
            <Switch
              size="md"
              title="Minimize while playing"
              on={settings.on_launch === 'minimize'}
              onchange={(on) => {
                settings.on_launch = on ? 'minimize' : 'keep_open';
                commit();
              }}
            />
          </div>
        </div>
        <div class="col gap">
          <div class="group">
            <div class="srow">
              <div class="label">
                <span class="sm">Launcher folder</span>
                <span class="hint truncate selectable" title={settings.data_dir}>{settings.data_dir}</span>
              </div>
              <div class="row">
                <Button label="Change…" disabled={busy} onclick={pickDataDir} />
                <Button label="Open" onclick={() => actions.openFolder(settings.data_dir)} />
              </div>
            </div>
          </div>
          <div class="row note">
            <span class="hint">
              {#if busy}
                Can not be changed while a game or a job is running.
              {:else if moved}
                Instances, versions and Java runtimes were not moved: the launcher starts over in the new folder.
              {:else}
                Holds instances, versions, libraries, assets and Java runtimes.
              {/if}
            </span>
            {#if moved && !busy}
              <button class="link nowrap" onclick={() => setDataDir(originalDir)}>Use the previous folder</button>
            {/if}
          </div>
        </div>
      {:else if section === 'java'}
        <div class="group">
          <div class="stacked">
            <span class="sm">Java executable</span>
            <JavaField
              bind:value={java}
              placeholder="Automatic"
              emptyHint="Each Minecraft version gets the Java it needs, downloaded from Mojang. Choose a binary only to use your own Java for every instance."
              resetLabel="Use automatic Java"
              onchange={commit}
            />
            {#if javaError}<div class="error">{javaError}</div>{/if}
          </div>
        </div>
        <div class="group">
          <div class="srow">
            <div class="label">
              <span class="sm">Memory</span>
              {#if memoryError}<span class="error">{memoryError}</span>
              {:else}<span class="hint">For instances without their own setting.</span>{/if}
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
                <TextField bind:value={memory} placeholder={String(DEFAULT_MEMORY)} label="Memory in MB" oninput={commit} />
              </div>
              <span class="sm muted">MB</span>
            </div>
          </div>
          <div class="stacked">
            <span class="sm">JVM arguments</span>
            <TextField bind:value={jvmArgs} placeholder="None" label="JVM arguments" oninput={commit} />
            <span class="hint">Passed to every instance, before the instance's own arguments.</span>
          </div>
        </div>
      {:else if section === 'game'}
        <div class="group">
          <div class="srow">
            <div class="label">
              <span class="sm">Window size</span>
              {#if sizeError}<span class="error">{sizeError}</span>
              {:else}<span class="hint">Leave empty for the game's default.</span>{/if}
            </div>
            <div class="row">
              <div class="small-input"><TextField bind:value={width} placeholder="854" label="Width" oninput={commit} /></div>
              <span class="sm muted">×</span>
              <div class="small-input"><TextField bind:value={height} placeholder="480" label="Height" oninput={commit} /></div>
            </div>
          </div>
          <div class="srow">
            <div class="label sm">Start in fullscreen</div>
            <Switch
              size="md"
              title="Start in fullscreen"
              on={settings.fullscreen}
              onchange={(on) => {
                settings.fullscreen = on;
                commit();
              }}
            />
          </div>
        </div>
      {:else}
        <div class="group">
          <div class="stacked">
            <span class="sm">CurseForge API key</span>
            <TextField bind:value={curseforgeKey} placeholder="API key" label="CurseForge API key" oninput={commit} />
            <div class="row">
              <span class="hint">
                {launcher.boot?.env_curseforge_key
                  ? 'CURSEFORGE_API_KEY is set in the environment and is used instead.'
                  : 'Needed to browse and install CurseForge modpacks.'}
              </span>
              <button class="link nowrap" onclick={() => openUrl(CURSEFORGE_CONSOLE)}>Get a key</button>
            </div>
          </div>
          <div class="stacked">
            <span class="sm">Microsoft client ID</span>
            <TextField bind:value={clientId} placeholder="Application (client) ID" label="Microsoft client ID" oninput={commit} />
            <span class="hint">
              The Azure application used to sign in with Microsoft accounts. Mojang has to allow it to use the
              Minecraft API.
            </span>
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .settings {
    flex: 1;
    min-height: 0;
    display: flex;
    background: var(--bg);
  }
  .nav {
    width: 200px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 8px 8px;
    background: var(--panel);
    border-right: 1px solid var(--border);
  }
  .back {
    color: var(--muted);
    margin-bottom: 8px;
  }
  .body {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    display: flex;
    justify-content: center;
  }
  .column {
    width: 100%;
    max-width: 680px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 24px 32px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  .gap {
    gap: 8px;
  }
  .note {
    padding: 0 4px;
    align-items: flex-start;
  }
  .nowrap {
    flex: none;
    white-space: nowrap;
  }
  .memory {
    gap: 6px;
  }
</style>
