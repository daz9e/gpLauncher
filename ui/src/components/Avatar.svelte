<script lang="ts">
  let { name, size = 22 }: { name: string; size?: number } = $props();
</script>

<span class="avatar" style:width="{size}px" style:height="{size}px">{name.charAt(0).toUpperCase()}</span>

<style>
  .avatar {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: var(--tile);
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
</style>
