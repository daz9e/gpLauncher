import { afterEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/svelte';
import InstanceSettings from '../instance/InstanceSettings.svelte';
import WorldsPage from '../instance/WorldsPage.svelte';
import LogFilesPage from '../instance/LogFilesPage.svelte';
import ScreenshotsPage from '../instance/ScreenshotsPage.svelte';
import { Backend, instance, settle, snapshot } from '../../test/backend';
import type { InstanceData } from '../../lib/types';

afterEach(() => Backend.reset());

describe('InstanceSettings', () => {
  function setup(over = {}) {
    const inst = instance({ id: 'i', name: 'Mine', loader: 'fabric', loader_version: '', minecraft: '1.21.1' });
    const backend: Backend = new Backend(snapshot({ instances: [inst], ...over }), {
      list_versions: () => [{ id: '1.21.4', kind: 'release' }, { id: '1.21.1', kind: 'release' }, { id: '24w01a', kind: 'snapshot' }],
      loader_versions: () => [{ version: '0.16.9', stable: true }],
      is_file: () => false,
      save_instance: ({ data }) => {
        const updated = { ...backend.snap.instances[0], ...(data as InstanceData) };
        backend.push({ ...backend.snap, instances: [updated] });
        return updated;
      },
    });
    render(InstanceSettings, { id: 'i' });
    const saved = () => backend.called('save_instance').map((c) => c.data as InstanceData);
    return { backend, saved };
  }

  it('saves names, groups and memory as they are typed', async () => {
    const { saved } = setup();
    await settle();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Name' }), { target: { value: '  ' } });
    await settle();
    expect(saved().at(-1)?.name).toBe('1.21.1');
    await fireEvent.input(screen.getByRole('textbox', { name: 'Group' }), { target: { value: ' Fun ' } });
    await settle();
    expect(saved().at(-1)?.group).toBe('Fun');
    await fireEvent.input(screen.getByRole('textbox', { name: 'Memory in MB' }), { target: { value: '12' } });
    await settle();
    expect(screen.getByText('At least 512 MB')).toBeInTheDocument();
    expect(saved().at(-1)?.memory_mb).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: '8 GB' }));
    await settle();
    expect(saved().at(-1)?.memory_mb).toBe(8192);
  });

  it('changes versions and loaders, clearing the loader build', async () => {
    const { saved } = setup();
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: 'Version: Latest stable' }));
    await fireEvent.click(screen.getByRole('option', { name: '0.16.9' }));
    await settle();
    expect(saved().at(-1)?.loader_version).toBe('0.16.9');
    await fireEvent.click(screen.getByRole('button', { name: 'Minecraft: 1.21.1' }));
    expect(screen.getAllByRole('option').map((o) => o.textContent?.trim())).toEqual(['1.21.4', '1.21.1']);
    await fireEvent.click(screen.getByRole('option', { name: '1.21.4' }));
    await settle();
    expect(saved().at(-1)).toMatchObject({ minecraft: '1.21.4', loader_version: '' });
    await fireEvent.click(screen.getByRole('radio', { name: 'None' }));
    await settle();
    expect(saved().at(-1)).toMatchObject({ loader: 'vanilla' });
    await fireEvent.click(screen.getByRole('radio', { name: 'Fullscreen' }));
    await settle();
    expect(saved().at(-1)?.fullscreen).toBe(true);
  });

  it('locks versions while the game runs', async () => {
    setup({ sessions: { i: { phase: 'running', status: 'Playing', progress: null, played: null, started_at: 0, key: 1 } } });
    await settle();
    expect(screen.getByRole('button', { name: 'Minecraft: 1.21.1' })).toBeDisabled();
    expect(screen.getByRole('radio', { name: 'Quilt' })).toBeDisabled();
    expect(screen.getByText('Close the game to change versions.')).toBeInTheDocument();
  });

  it('takes window sizes in pairs and refuses missing Java', async () => {
    const { saved } = setup();
    await settle();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Width' }), { target: { value: '800' } });
    await settle();
    expect(screen.getByText('Set both width and height, or neither')).toBeInTheDocument();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Height' }), { target: { value: '600' } });
    await settle();
    expect(saved().at(-1)).toMatchObject({ window_width: 800, window_height: 600 });
    await fireEvent.input(screen.getByRole('textbox', { name: 'Java executable' }), { target: { value: '/no/java' } });
    await settle();
    expect(screen.getByText('No Java at /no/java')).toBeInTheDocument();
    expect(saved().at(-1)?.java_path).toBe('');
  });
});

describe('WorldsPage', () => {
  it('lists worlds with sizes and deletes after asking', async () => {
    let worlds = [{ path: '/g/saves/w', folder: 'w', name: 'My World', icon: null, last_played: 0, game_mode: 'Creative' }];
    const backend = new Backend(snapshot({ instances: [instance({ id: 'i' })] }), {
      worlds: () => worlds,
      dir_size: () => 2048,
      delete_world: () => {
        worlds = [];
        return null;
      },
    });
    backend.answer('Delete');
    render(WorldsPage, { id: 'i', reload: 1, active: true });
    await settle();
    expect(screen.getByText('My World')).toBeInTheDocument();
    expect(screen.getByText('Creative · Played never · 2 KB')).toBeInTheDocument();
    expect(screen.getByText('w')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await settle();
    expect(backend.called('delete_world')).toEqual([{ id: 'i', path: '/g/saves/w' }]);
    expect(screen.getByText('No worlds yet')).toBeInTheDocument();
  });
});

describe('LogFilesPage', () => {
  it('opens the newest file and switches files', async () => {
    const backend = new Backend(snapshot({ instances: [instance({ id: 'i' })] }), {
      log_files: () => [
        { path: '/g/logs/latest.log', name: 'latest.log', crash: false, modified: 0 },
        { path: '/g/crash-reports/crash.txt', name: 'crash.txt', crash: true, modified: 0 },
      ],
      read_log: ({ path }) => [{ text: `contents of ${path}`, level: 'info' }],
    });
    render(LogFilesPage, { id: 'i', reload: 1, active: true });
    await settle();
    expect(screen.getByText('contents of /g/logs/latest.log')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: /crash\.txt/ }));
    await settle();
    expect(screen.getByText('contents of /g/crash-reports/crash.txt')).toBeInTheDocument();
    expect(backend.called('read_log')).toHaveLength(2);
  });

  it('says when there are no logs', async () => {
    new Backend(snapshot({ instances: [instance({ id: 'i' })] }), { log_files: () => [] });
    render(LogFilesPage, { id: 'i', reload: 1, active: true });
    await settle();
    expect(screen.getByText('No log files')).toBeInTheDocument();
  });
});

describe('ScreenshotsPage', () => {
  it('shows screenshots and opens them', async () => {
    const backend = new Backend(snapshot({ instances: [instance({ id: 'i' })] }), {
      screenshots: () => ['/g/screenshots/2024-05-01_10.00.00.png'],
      open_file: () => null,
    });
    render(ScreenshotsPage, { id: 'i', reload: 1, active: true });
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: /2024-05-01_10\.00\.00/ }));
    expect(backend.called('open_file')).toEqual([{ path: '/g/screenshots/2024-05-01_10.00.00.png' }]);
  });

  it('does not load while hidden', async () => {
    const backend = new Backend(snapshot({ instances: [instance({ id: 'i' })] }), { screenshots: () => [] });
    render(ScreenshotsPage, { id: 'i', reload: 1, active: false });
    await settle();
    expect(backend.called('screenshots')).toEqual([]);
  });
});
