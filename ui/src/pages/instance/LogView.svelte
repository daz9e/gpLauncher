<script lang="ts">
  // Log console: the live output of a game, or the lines of a log file. Follows new lines while
  // scrolled to the bottom, filters by text and severity, copies and shares what is shown.
  import { onMount, tick, untrack } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { ask, message as alert } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { launcher } from '../../lib/store.svelte';
  import { api, message } from '../../lib/api';
  import { LogBuffer, filterLines, type Severity } from '../../lib/logs';
  import type { LogChunk, LogLine } from '../../lib/types';
  import TextField from '../../components/TextField.svelte';
  import Segments from '../../components/Segments.svelte';
  import IconButton from '../../components/IconButton.svelte';
  import Button from '../../components/Button.svelte';
  import EmptyState from '../../components/EmptyState.svelte';

  let {
    session,
    lines: fixed,
    empty,
  }: {
    /** Instance whose last launch is shown. */
    session?: string;
    /** Fixed lines, e.g. from a file. */
    lines?: LogLine[];
    /** Title and detail shown while there are no lines. */
    empty: [string, string];
  } = $props();

  const ROW = 18;
  const OVERSCAN = 20;

  const buffer = new LogBuffer();
  /** Bumped whenever the buffer changes; the buffer itself is not reactive. */
  let version = $state(0);
  let query = $state('');
  let severity = $state<Severity>('all');
  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(400);
  let follow = $state(true);
  let uploading = $state(false);
  /** For a session that has not run since the launcher started: the game's last log file. */
  let fallback = $state(false);

  let hasSession = $derived(!!session && !!launcher.snap?.sessions[session]);

  async function refetch() {
    if (!session) return;
    const chunk = await api.getLog(session);
    if (chunk) {
      buffer.reset(chunk);
      fallback = false;
      version++;
    } else if (!fallback) {
      const lines = await api.latestLog(session).catch(() => null);
      if (lines && !launcher.snap?.sessions[session]) {
        buffer.setLines(lines);
        fallback = true;
        version++;
      }
    }
  }

  $effect(() => {
    const lines = fixed;
    if (lines)
      untrack(() => {
        buffer.setLines(lines);
        fallback = false;
        version++;
        follow = true;
      });
  });

  // A new launch replaces the last run's log file.
  $effect(() => {
    if (session && hasSession && fallback) refetch();
  });

  onMount(() => {
    if (!session) return;
    refetch();
    const unlisten = listen<LogChunk>('log', (e) => {
      if (e.payload.id !== session) return;
      if (fallback || !buffer.append(e.payload)) refetch();
      else version++;
    });
    return () => unlisten.then((f) => f());
  });

  let matches = $derived.by(() => {
    version;
    return filterLines(buffer.lines, severity, query);
  });
  let count = $derived.by(() => {
    version;
    return matches ? matches.length : buffer.lines.length;
  });
  let total = $derived.by(() => {
    version;
    return buffer.lines.length;
  });

  let first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  let last = $derived(Math.min(count, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  let rows = $derived.by(() => {
    version;
    const out: { ix: number; line: LogLine }[] = [];
    for (let i = first; i < last; i++) {
      const ix = matches ? matches[i] : i;
      const line = buffer.lines[ix];
      if (line) out.push({ ix: i, line });
    }
    return out;
  });

  // Stay at the bottom while following.
  $effect(() => {
    count;
    if (follow && viewport) tick().then(() => viewport && (viewport.scrollTop = viewport.scrollHeight));
  });

  function scrolled() {
    if (!viewport) return;
    scrollTop = viewport.scrollTop;
    follow = viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - ROW * 2;
  }

  function jumpToBottom() {
    follow = true;
    if (viewport) viewport.scrollTop = viewport.scrollHeight;
  }

  function visibleText(): string {
    const pick = matches ? matches.map((i) => buffer.lines[i]) : buffer.lines;
    return pick.map((l) => l.text).join('\n');
  }

  async function copy() {
    await writeText(visibleText());
  }

  async function upload() {
    if (uploading) return;
    const text = visibleText();
    if (!text.trim()) return;
    const yes = await ask('Anyone with the link can read it. The link is copied to the clipboard and opened.', {
      title: 'Share this log on mclo.gs?',
      kind: 'info',
      okLabel: 'Upload',
      cancelLabel: 'Cancel',
    });
    if (!yes) return;
    uploading = true;
    try {
      const url = await api.uploadLog(text);
      await writeText(url).catch(() => {});
      await openUrl(url);
    } catch (e) {
      await alert(message(e), { title: 'Upload failed', kind: 'error' });
    } finally {
      uploading = false;
    }
  }

  function refilter() {
    follow = true;
  }
</script>

<div class="log-view">
  <div class="toolbar">
    <div class="filter"><TextField bind:value={query} placeholder="Filter" icon="search" oninput={refilter} /></div>
    <Segments
      small
      label="Severity"
      options={[
        { value: 'all' as Severity, label: 'All' },
        { value: 'warnings' as Severity, label: 'Warnings' },
        { value: 'errors' as Severity, label: 'Errors' },
      ]}
      value={severity}
      onchange={(s) => {
        severity = s;
        refilter();
      }}
    />
    <span class="grow"></span>
    {#if matches}<span class="xs subtle">{count} of {total} lines</span>{/if}
    <IconButton icon="copy" title="Copy shown lines" disabled={total === 0} onclick={copy} />
    <IconButton
      icon="upload"
      title={uploading ? 'Uploading…' : 'Share on mclo.gs'}
      disabled={total === 0 || uploading}
      onclick={upload}
    />
  </div>
  {#if fallback}
    <div class="banner xs muted">From the last run (logs/latest.log). Press Play to see live output.</div>
  {/if}
  <div class="area">
    {#if total === 0}
      <EmptyState icon="terminal" title={empty[0]} detail={empty[1]} />
    {:else if count === 0}
      <EmptyState icon="search" title="No matching lines" detail="Try another filter." />
    {:else}
      <div class="viewport selectable" bind:this={viewport} bind:clientHeight={height} onscroll={scrolled} role="log">
        <div class="spacer" style:height="{count * ROW + 8}px">
          {#each rows as { ix, line } (ix)}
            <div class="line {line.level}" style:top="{ix * ROW + 4}px">{line.text || ' '}</div>
          {/each}
        </div>
      </div>
      {#if !follow}
        <div class="jump"><Button icon="arrow-down" label="Latest" onclick={jumpToBottom} /></div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .log-view {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .filter {
    width: 220px;
  }
  .banner {
    flex: none;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--accent-soft);
  }
  .area {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--panel);
  }
  .viewport {
    position: absolute;
    inset: 0;
    overflow: auto;
    font: 12px/18px var(--mono);
  }
  .spacer {
    position: relative;
    min-width: max-content;
  }
  .line {
    position: absolute;
    left: 0;
    right: 0;
    height: 18px;
    padding: 0 12px;
    white-space: pre;
    color: var(--text);
  }
  .line.launcher {
    color: var(--accent);
    font-weight: 500;
  }
  .line.warn {
    color: var(--warning);
  }
  .line.error {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 6%, transparent);
  }
  .line.debug {
    color: var(--subtle);
  }
  .jump {
    position: absolute;
    right: 16px;
    bottom: 12px;
    box-shadow: var(--shadow);
    border-radius: 6px;
  }
</style>
