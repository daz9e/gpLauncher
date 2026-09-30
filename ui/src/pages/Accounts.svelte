<script lang="ts">
  // "Accounts" dialog: pick the account to play with, add offline profiles, sign in with Microsoft.
  import { onDestroy } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { launcher } from '../lib/store.svelte';
  import { api, message } from '../lib/api';
  import type { DeviceCode } from '../lib/types';
  import Modal from '../components/Modal.svelte';
  import Button from '../components/Button.svelte';
  import TextField from '../components/TextField.svelte';
  import Avatar from '../components/Avatar.svelte';
  import IconButton from '../components/IconButton.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const AZURE_APPS = 'https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade';

  let snap = $derived(launcher.snap!);
  let offlineName = $state('');
  let offlineError = $state<string | null>(null);
  let clientIdText = $state(launcher.snap!.settings.ms_client_id);
  let clientIdInput = $state<HTMLInputElement>();
  let editingClientId = $state(false);
  type Login = { state: 'idle' } | { state: 'requesting' } | { state: 'waiting'; code: DeviceCode } | { state: 'failed'; error: string };
  let login = $state<Login>({ state: 'idle' });
  /** Bumped when a sign-in is cancelled, so its late result is ignored. */
  let attempt = 0;

  let clientId = $derived(snap.settings.ms_client_id);

  async function addOffline() {
    const name = offlineName.trim();
    try {
      await api.addOfflineAccount(name);
      offlineName = '';
    } catch (e) {
      offlineError = message(e);
    }
  }

  async function saveClientId() {
    const id = clientIdText.trim();
    if (!id) return;
    await api.setClientId(id);
    editingClientId = false;
  }

  async function signIn() {
    cancelSignIn();
    const mine = ++attempt;
    login = { state: 'requesting' };
    try {
      const code = await api.msRequestCode();
      if (mine !== attempt) return;
      login = { state: 'waiting', code };
      await api.msComplete();
      if (mine === attempt) login = { state: 'idle' };
    } catch (e) {
      if (mine === attempt) login = { state: 'failed', error: message(e) };
    }
  }

  function cancelSignIn() {
    attempt++;
    login = { state: 'idle' };
    api.msCancel();
  }

  function close() {
    cancelSignIn();
    onclose();
  }

  onDestroy(() => api.msCancel());

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'Enter') {
      if (document.activeElement === clientIdInput) saveClientId();
      else if (offlineName.trim()) addOffline();
    }
  }

  function changeClientId() {
    login = { state: 'idle' };
    editingClientId = true;
    queueMicrotask(() => clientIdInput?.focus());
  }
</script>

<svelte:window onkeydown={keydown} />

<Modal onclose={close}>
  <div class="dialog" role="dialog" aria-label="Accounts">
    <div class="title semibold">Accounts</div>
    <div class="body">
      {#if snap.accounts.length === 0}
        <div class="sm muted empty">No accounts yet: the game starts offline as Player.</div>
      {:else}
        <div class="list">
          {#each snap.accounts as account, i (account.kind + account.uuid)}
            {@const selected = i === snap.selected_account}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="account"
              class:selected
              role="option"
              aria-selected={selected}
              tabindex="-1"
              onclick={() => api.selectAccount(i)}
            >
              <Avatar name={account.name} size={30} />
              <div class="grow col">
                <span class="sm medium truncate">{account.name}</span>
                <span class="xs muted">{account.kind}</span>
              </div>
              {#if selected}<span class="xs medium active">Active</span>{/if}
              <IconButton
                icon="trash"
                title="Remove {account.name}"
                onclick={(e) => {
                  e.stopPropagation();
                  api.removeAccount(i);
                }}
              />
            </div>
          {/each}
        </div>
      {/if}
      <div class="rule"></div>
      <div class="section">
        <span class="caption">Offline account</span>
        <div class="row">
          <div class="grow">
            <TextField
              bind:value={offlineName}
              placeholder="Player name"
              autofocus
              oninput={() => (offlineError = null)}
            />
          </div>
          <Button label="Add" disabled={!offlineName.trim()} onclick={addOffline} />
        </div>
        {#if offlineError}
          <div class="xs danger">{offlineError}</div>
        {:else}
          <div class="xs subtle">For singleplayer and servers in offline mode.</div>
        {/if}
      </div>
      <div class="section">
        <span class="caption">Microsoft account</span>
        {#if !clientId || editingClientId}
          <div class="xs muted">
            Signing in needs the client ID of an Azure application that Mojang allowed to use the Minecraft API.
          </div>
          <div class="row">
            <div class="grow">
              <TextField bind:value={clientIdText} bind:input={clientIdInput} placeholder="Application (client) ID" />
            </div>
            <Button label="Save" disabled={!clientIdText.trim()} onclick={saveClientId} />
          </div>
          <button class="link" onclick={() => openUrl(AZURE_APPS)}>Open Azure app registrations</button>
        {:else if login.state === 'idle'}
          <div class="row gap">
            <Button label="Sign in with Microsoft" variant="primary" onclick={signIn} />
            <button class="link" onclick={changeClientId}>Change client ID</button>
          </div>
        {:else if login.state === 'requesting'}
          <div class="sm muted">Requesting a code…</div>
        {:else if login.state === 'waiting'}
          {@const code = login.code}
          <div class="sm muted">Open {code.verification_uri} and enter the code</div>
          <div class="row gap">
            <span class="code semibold selectable">{code.user_code}</span>
            <span class="grow"></span>
            <Button
              label="Copy code & open"
              variant="primary"
              onclick={async () => {
                await writeText(code.user_code).catch(() => {});
                await openUrl(code.verification_uri);
              }}
            />
            <Button label="Cancel" onclick={cancelSignIn} />
          </div>
          <div class="xs subtle">Waiting for you to sign in…</div>
        {:else}
          <div class="xs danger">{login.error}</div>
          <div class="row gap">
            <Button label="Try again" onclick={signIn} />
            <button class="link" onclick={changeClientId}>Change client ID</button>
          </div>
        {/if}
      </div>
    </div>
    <footer class="footer"><Button label="Done" onclick={close} /></footer>
  </div>
</Modal>

<style>
  .dialog {
    width: 460px;
    max-width: 100%;
    max-height: 100%;
    display: flex;
    flex-direction: column;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--bg);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .title {
    padding: 16px 20px 4px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 12px 20px;
  }
  .empty {
    padding: 12px 0;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .account {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid transparent;
    cursor: pointer;
    outline: none;
  }
  .account:hover {
    background: var(--hover);
  }
  .account.selected {
    background: var(--accent-soft);
    border-color: var(--accent-edge);
  }
  .active {
    color: var(--accent);
  }
  .rule {
    height: 1px;
    background: var(--border);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .section .link {
    align-self: flex-start;
  }
  .caption {
    font-size: 12px;
    font-weight: 500;
    color: var(--muted);
  }
  .gap {
    gap: 12px;
  }
  .code {
    font-size: 24px;
    letter-spacing: 0.04em;
  }
  .footer {
    display: flex;
    justify-content: flex-end;
    padding: 12px 20px;
    border-top: 1px solid var(--border);
  }
</style>
