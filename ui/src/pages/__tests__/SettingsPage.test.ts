import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/svelte';
import SettingsPage from '../settings/SettingsPage.svelte';
import { Backend, settle, snapshot } from '../../test/backend';
import type { Handler } from '../../test/backend';
import type { Settings } from '../../lib/types';

afterEach(() => Backend.reset());

function setup(over = {}, handlers: Record<string, Handler> = {}) {
  const backend = new Backend(snapshot(over), {
    set_settings: () => null,
    is_file: ({ path }) => path === '/usr/bin/java',
    java_version: () => 'openjdk version "21"',
    ...handlers,
  });
  const onclose = vi.fn();
  render(SettingsPage, { onclose });
  const last = () => backend.called('set_settings').at(-1)?.settings as Settings;
  return { backend, onclose, last };
}

describe('SettingsPage', () => {
  it('applies appearance and minimize right away', async () => {
    const { last } = setup();
    await fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    await settle();
    expect(last().appearance).toBe('dark');
    await fireEvent.click(screen.getByRole('switch', { name: 'Minimize while playing' }));
    await settle();
    expect(last().on_launch).toBe('minimize');
  });

  it('keeps the last valid memory and says why', async () => {
    const { last } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Java' }));
    const memory = screen.getByRole('textbox', { name: 'Memory in MB' });
    await fireEvent.input(memory, { target: { value: '100' } });
    await settle();
    expect(screen.getByText('At least 512 MB')).toBeInTheDocument();
    expect(last().memory_mb).toBe(4096);
    await fireEvent.click(screen.getByRole('button', { name: '6 GB' }));
    await settle();
    expect(last().memory_mb).toBe(6144);
    expect(screen.queryByText('At least 512 MB')).toBeNull();
    await fireEvent.input(memory, { target: { value: '' } });
    await settle();
    expect(last().memory_mb).toBe(4096);
  });

  it('checks the Java path and tests it', async () => {
    const { last, backend } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Java' }));
    const java = screen.getByRole('textbox', { name: 'Java executable' });
    await fireEvent.input(java, { target: { value: '/nope/java' } });
    await settle();
    expect(screen.getByText('No Java at /nope/java')).toBeInTheDocument();
    expect(last().java_path).toBe('');
    await fireEvent.input(java, { target: { value: '/usr/bin/java' } });
    await settle();
    expect(last().java_path).toBe('/usr/bin/java');
    await fireEvent.click(screen.getByRole('button', { name: 'Test' }));
    await settle();
    expect(backend.called('java_version')).toEqual([{ path: '/usr/bin/java' }]);
    expect(screen.getByText('openjdk version "21"')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Use automatic Java' }));
    await settle();
    expect(last().java_path).toBe('');
  });

  it('takes window sizes only in pairs', async () => {
    const { last } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Game' }));
    await fireEvent.input(screen.getByRole('textbox', { name: 'Width' }), { target: { value: '1280' } });
    await settle();
    expect(screen.getByText('Set both width and height, or neither')).toBeInTheDocument();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Height' }), { target: { value: '720' } });
    await settle();
    expect([last().window_width, last().window_height]).toEqual([1280, 720]);
    await fireEvent.click(screen.getByRole('switch', { name: 'Start in fullscreen' }));
    await settle();
    expect(last().fullscreen).toBe(true);
  });

  it('stores service keys', async () => {
    const { last } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Services' }));
    await fireEvent.input(screen.getByRole('textbox', { name: 'CurseForge API key' }), { target: { value: ' k ' } });
    await fireEvent.input(screen.getByRole('textbox', { name: 'Microsoft client ID' }), { target: { value: 'id' } });
    await settle();
    expect(last().curseforge_api_key).toBe('k');
    expect(last().ms_client_id).toBe('id');
  });

  it('changes the launcher folder unless busy, and offers the previous one', async () => {
    const { last } = setup({}, { 'plugin:dialog|open': () => '/new/place' });
    await fireEvent.click(screen.getByRole('button', { name: 'Change…' }));
    await settle();
    expect(last().data_dir).toBe('/new/place');
    expect(screen.getByText(/were not moved/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Use the previous folder' }));
    await settle();
    expect(last().data_dir).toBe('/data');
  });

  it('can not change the folder while busy', () => {
    setup({ busy: true });
    expect(screen.getByRole('button', { name: 'Change…' })).toBeDisabled();
    expect(screen.getByText('Can not be changed while a game or a job is running.')).toBeInTheDocument();
  });

  it('shows errors from the backend and goes back', async () => {
    const { onclose } = setup({}, { set_settings: () => Promise.reject('disk full') });
    await fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    await settle();
    expect(screen.getByText('disk full')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Instances' }));
    expect(onclose).toHaveBeenCalled();
  });
});
