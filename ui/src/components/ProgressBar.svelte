<script lang="ts">
  let { ratio }: { ratio: number } = $props();
</script>

<div class="bar" role="progressbar" aria-valuenow={Math.round(Math.min(Math.max(ratio, 0), 1) * 100)}>
  <div class="fill" style:width="{Math.min(Math.max(ratio, 0), 1) * 100}%"></div>
</div>

<style>
  .bar {
    height: 4px;
    border-radius: 999px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.15s;
  }
</style>
