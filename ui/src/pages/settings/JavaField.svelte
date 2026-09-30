<script lang="ts">
  // Java executable field with Browse and a `java -version` test; used by both settings pages.
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, message } from '../../lib/api';
  import Button from '../../components/Button.svelte';
  import TextField from '../../components/TextField.svelte';

  let {
    value = $bindable(''),
    placeholder,
    emptyHint,
    resetLabel,
    onchange,
  }: {
    value?: string;
    placeholder: string;
    /** Shown while the field is empty: what happens without a binary. */
    emptyHint: string;
    /** Link that clears the field. */
    resetLabel: string;
    onchange: () => void;
  } = $props();

  type Check = { state: 'idle' } | { state: 'running' } | { state: 'done'; ok: boolean; text: string };
  let check = $state<Check>({ state: 'idle' });
  let hasPath = $derived(value.trim() !== '');

  async function pick() {
    const path = await open({ multiple: false, directory: false });
    if (typeof path !== 'string') return;
    value = path;
    onchange();
    test();
  }

  async function test() {
    const path = value.trim();
    if (!path) return;
    check = { state: 'running' };
    try {
      const text = await api.javaVersion(path);
      // Only when the path was not edited meanwhile.
      if (check.state === 'running' && value.trim() === path) check = { state: 'done', ok: true, text };
    } catch (e) {
      if (check.state === 'running' && value.trim() === path) check = { state: 'done', ok: false, text: message(e) };
    }
  }
</script>

<div class="java">
  <div class="row">
    <div class="grow">
      <TextField
        bind:value
        {placeholder}
        label="Java executable"
        oninput={() => {
          check = { state: 'idle' };
          onchange();
        }}
      />
    </div>
    <Button label="Browse…" onclick={pick} />
    <Button label="Test" disabled={!hasPath || check.state === 'running'} onclick={test} />
  </div>
  {#if !hasPath}
    <div class="hint">{emptyHint}</div>
  {:else if check.state === 'idle'}
    <div class="hint">Press Test to check this Java.</div>
  {:else if check.state === 'running'}
    <div class="hint">Checking…</div>
  {:else if check.ok}
    <div class="xs selectable">{check.text}</div>
  {:else}
    <div class="error selectable">{check.text}</div>
  {/if}
  {#if hasPath}
    <button
      class="link"
      onclick={() => {
        value = '';
        check = { state: 'idle' };
        onchange();
      }}>{resetLabel}</button
    >
  {/if}
</div>

<style>
  .java {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .link {
    align-self: flex-start;
  }
</style>
