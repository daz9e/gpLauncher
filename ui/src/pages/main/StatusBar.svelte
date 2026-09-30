<script lang="ts">
  // One line at the bottom: the running job, the selected game's status, or the last notice.
  import { launcher } from '../../lib/store.svelte';
  import ProgressBar from '../../components/ProgressBar.svelte';

  let { selected }: { selected: string | null } = $props();

  let snap = $derived(launcher.snap!);
  let line = $derived.by(() => {
    const job = snap.job;
    const session = selected ? snap.sessions[selected] : undefined;
    // An active job wins; a finished one stays until the user selects something.
    if (job?.active) return { text: job.status, active: true, progress: job.progress };
    if (job) return { text: job.status, active: false, progress: null };
    if (session && session.phase !== 'finished')
      return { text: session.status, active: true, progress: session.progress };
    return { text: 'Ready', active: false, progress: null };
  });
  let ratio = $derived(line.progress ? Math.min(Math.max(line.progress[0] / line.progress[1], 0), 1) : 0);
</script>

<footer class="status xs" aria-label="Status">
  <span class="dot" class:active={line.active}></span>
  <span class="grow truncate" data-testid="status-text">{line.text}</span>
  {#if line.progress}
    <div class="bar"><ProgressBar {ratio} /></div>
    <span class="percent">{Math.round(ratio * 100)}%</span>
  {/if}
  {#if snap.running > 0}
    <span class="row running">
      <span class="dot green"></span>
      {snap.running === 1 ? '1 game running' : `${snap.running} games running`}
    </span>
  {/if}
</footer>

<style>
  .status {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    color: var(--muted);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--subtle);
    flex: none;
  }
  .dot.active {
    background: var(--accent);
  }
  .dot.green {
    background: var(--success);
  }
  .bar {
    width: 160px;
  }
  .percent {
    width: 32px;
    text-align: right;
  }
  .running {
    gap: 6px;
  }
</style>
