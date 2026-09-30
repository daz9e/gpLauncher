import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import Accounts from '../Accounts.svelte';
import { Backend, settle, snapshot } from '../../test/backend';
import type { Handler } from '../../test/backend';

afterEach(() => Backend.reset());

function setup(over = {}, handlers: Record<string, Handler> = {}) {
  const backend = new Backend(
    snapshot({
      accounts: [
        { kind: 'Offline', name: 'Steve', uuid: 'a' },
        { kind: 'Microsoft', name: 'Notch', uuid: 'b' },
      ],
      selected_account: 1,
      ...over,
    }),
    {
      select_account: () => null,
      remove_account: () => null,
      add_offline_account: ({ name }) => ((name as string).length < 3 ? Promise.reject('Use 3–16 characters: letters, digits and _') : null),
      set_client_id: () => null,
      ms_request_code: () => ({ user_code: 'ABCD-1234', verification_uri: 'https://microsoft.com/link' }),
      ms_complete: () => new Promise(() => {}),
      ms_cancel: () => null,
      ...handlers,
    },
  );
  const onclose = vi.fn();
  render(Accounts, { onclose });
  return { backend, onclose };
}

describe('Accounts', () => {
  it('lists accounts and switches or removes them', async () => {
    const { backend } = setup();
    const steve = screen.getByRole('option', { name: /Steve/ });
    expect(screen.getByRole('option', { selected: true })).toHaveTextContent('Notch');
    expect(within(screen.getByRole('option', { selected: true })).getByText('Active')).toBeInTheDocument();
    await fireEvent.click(steve);
    expect(backend.called('select_account')).toEqual([{ index: 0 }]);
    await fireEvent.click(screen.getByRole('button', { name: 'Remove Notch' }));
    expect(backend.called('remove_account')).toEqual([{ index: 1 }]);
    // Removing does not select.
    expect(backend.called('select_account')).toHaveLength(1);
  });

  it('adds offline accounts and shows why a name is refused', async () => {
    const { backend } = setup();
    const name = screen.getByRole('textbox', { name: 'Player name' });
    expect(screen.getByRole('button', { name: 'Add' })).toBeDisabled();
    await fireEvent.input(name, { target: { value: 'ab' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await settle();
    expect(screen.getByText('Use 3–16 characters: letters, digits and _')).toBeInTheDocument();
    await fireEvent.input(name, { target: { value: 'Alex' } });
    expect(screen.queryByText('Use 3–16 characters: letters, digits and _')).toBeNull();
    await fireEvent.keyDown(window, { key: 'Enter' });
    await settle();
    expect(backend.called('add_offline_account').at(-1)).toEqual({ name: 'Alex' });
    expect(name).toHaveValue('');
  });

  it('asks for a client ID before signing in', async () => {
    const { backend } = setup();
    expect(screen.queryByRole('button', { name: 'Sign in with Microsoft' })).toBeNull();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Application (client) ID' }), { target: { value: ' my-app ' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(backend.called('set_client_id')).toEqual([{ id: 'my-app' }]);
  });

  it('signs in with a device code and can cancel', async () => {
    const { backend } = setup({ settings: { ...snapshot().settings, ms_client_id: 'app' } }, { 'plugin:clipboard-manager|write_text': () => null, 'plugin:opener|open_url': () => null });
    await fireEvent.click(screen.getByRole('button', { name: 'Sign in with Microsoft' }));
    await settle();
    expect(screen.getByText('ABCD-1234')).toBeInTheDocument();
    expect(screen.getByText('Open https://microsoft.com/link and enter the code')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy code & open' }));
    await settle();
    expect(backend.called('plugin:clipboard-manager|write_text')[0]).toMatchObject({ text: 'ABCD-1234' });
    expect(backend.called('plugin:opener|open_url')[0]).toMatchObject({ url: 'https://microsoft.com/link' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(backend.called('ms_cancel').length).toBeGreaterThan(1);
    expect(screen.getByRole('button', { name: 'Sign in with Microsoft' })).toBeInTheDocument();
  });

  it('shows sign-in failures with a retry', async () => {
    setup({ settings: { ...snapshot().settings, ms_client_id: 'app' } }, { ms_request_code: () => Promise.reject('network down') });
    await fireEvent.click(screen.getByRole('button', { name: 'Sign in with Microsoft' }));
    await settle();
    expect(screen.getByText('network down')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Change client ID' }));
    expect(screen.getByRole('textbox', { name: 'Application (client) ID' })).toBeInTheDocument();
  });

  it('closes on Escape and Done, stopping a sign-in', async () => {
    const { backend, onclose } = setup();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onclose).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(onclose).toHaveBeenCalledTimes(2);
    expect(backend.called('ms_cancel').length).toBeGreaterThan(0);
  });

  it('says the game starts as Player without accounts', () => {
    setup({ accounts: [], selected_account: 0 });
    expect(screen.getByText('No accounts yet: the game starts offline as Player.')).toBeInTheDocument();
  });
});
