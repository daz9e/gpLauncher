<script lang="ts">
  import { launcher } from './lib/store.svelte';
  import MainWindow from './pages/MainWindow.svelte';
  import InstanceWindow from './pages/InstanceWindow.svelte';
  import type { InstancePage } from './lib/types';

  // `#/instance/<id>/<page>` is an instance window; anything else is the main window.
  const route = location.hash.match(/^#\/instance\/([^/]+)\/([^/]+)/);
  const instanceId = route ? decodeURIComponent(route[1]) : null;
  const page = (route ? decodeURIComponent(route[2]) : 'console') as InstancePage;

  let ready = $state(false);
  launcher.init().then(() => (ready = true));

  $effect(() => {
    document.documentElement.dataset.theme = launcher.dark ? 'dark' : 'light';
  });

  // No browser menu: the app has its own.
  function contextmenu(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('input, textarea, .selectable')) e.preventDefault();
  }
</script>

<svelte:window oncontextmenu={contextmenu} />

{#if ready && launcher.snap}
  {#if instanceId}
    <InstanceWindow id={instanceId} initialPage={page} />
  {:else}
    <MainWindow />
  {/if}
{/if}
