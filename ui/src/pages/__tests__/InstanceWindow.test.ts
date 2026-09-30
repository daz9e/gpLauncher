import { afterEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { emit } from '@tauri-apps/api/event';
import InstanceWindow from '../InstanceWindow.svelte';
import LogView from '../instance/LogView.svelte';
import { Backend, instance, settle, snapshot } from '../../test/backend';
import type { Handler } from '../../test/backend';
import type { Item, LogLine, SessionView, Update, Version } from '../../lib/types';

afterEach(() => Backend.reset());

const line = (text: string, level: LogLine['level'] = 'info'): LogLine => ({ text, level });
const session = (over: Partial<SessionView> = {}): SessionView => ({
  phase: 'running', status: 'Playing', progress: null, played: null, started_at: Date.now() - 125_000, key: 7, ...over,
});

const item = (name: string, over: Partial<Item> = {}): Item => ({
  path: `/data/instances/mod/minecraft/mods/${name}.jar`, file_name: `${name}.jar`, enabled: true, name, id: name.toLowerCase(),
  version: '1.0', description: `${name} does things`, authors: ['Dev'], icon: null, size: 1_048_576, modified: 0, ...over,
});
const version = (project: string, number: string): Version => ({
  id: `${project}-${number}`, project_id: project, name: number, number, game_versions: ['1.21.1'], loaders: ['fabric'],
  url: '', file_name: '', sha1: null, size: null, dependencies: [],
});

function setup(page = 'console', over = {}, handlers: Record<string, Handler> = {}) {
  const inst = instance({ id: 'mod', name: 'Modded', loader: 'fabric', description: '1.21.1 · Fabric' });
  const backend = new Backend(
    snapshot({ instances: [inst], ...over }),
    {
      get_log: () => null,
      latest_log: () => null,
      launch: () => true,
      kill: () => null,
      open_folder: () => null,
      'plugin:window|set_title': () => null,
      ...handlers,
    },
    'instance-6d6f64',
  );
  render(InstanceWindow, { id: 'mod', initialPage: page });
  return backend;
}

describe('InstanceWindow', () => {
  it('shows the instance, plays and switches pages', async () => {
    const backend = setup('settings', {}, { list_versions: () => [], loader_versions: () => [] });
    await settle();
    expect(screen.getAllByText('Modded').length).toBeGreaterThan(0);
    expect(backend.called('plugin:window|set_title')[0]).toMatchObject({ value: 'Modded' });
    expect(screen.getByRole('button', { name: /Settings/ })).toHaveClass('active');
    await fireEvent.click(screen.getByRole('button', { name: 'Play' }));
    await settle();
    expect(backend.called('launch')).toEqual([{ id: 'mod' }]);
    expect(screen.getByRole('button', { name: /Console/ })).toHaveClass('active');
    await emit('show-page', 'worlds');
    await settle();
    expect(screen.getByRole('button', { name: /Worlds/ })).toHaveClass('active');
  });

  it('shows the running game with a timer and stops it', async () => {
    const backend = setup('console', { sessions: { mod: session() }, running: 1 });
    await settle();
    expect(screen.getByText('Playing for 2m')).toBeInTheDocument();
    expect(screen.getByTitle('Running')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(backend.called('kill')).toEqual([{ id: 'mod' }]);
  });

  it('shows how long a finished game ran', async () => {
    setup('console', { sessions: { mod: session({ phase: 'finished', status: 'Game closed', played: 3700 }) } });
    await settle();
    expect(screen.getByText('Game closed · played 1h 01m')).toBeInTheDocument();
  });

  it('closes when the instance goes away', async () => {
    const backend = setup('console', {}, { 'plugin:window|close': () => null });
    await backend.push({ ...backend.snap, instances: [] });
    await settle();
    expect(backend.called('plugin:window|close')).toHaveLength(1);
  });
});

describe('LogView', () => {
  it('follows a live session and refetches when lines were missed', async () => {
    let log = { id: 'mod', key: 1, start: 0, lines: [line('Launching', 'launcher'), line('[main/INFO]: hi')] };
    const backend = new Backend(snapshot({ instances: [instance({ id: 'mod' })], sessions: { mod: session({ key: 1 }) } }), {
      get_log: () => log,
      latest_log: () => null,
    });
    render(LogView, { session: 'mod', empty: ['Nothing', 'yet'] });
    await settle();
    const lines = () => [...document.querySelectorAll('.line')].map((l) => l.textContent);
    expect(lines()).toEqual(['Launching', '[main/INFO]: hi']);
    await emit('log', { id: 'mod', key: 1, start: 2, lines: [line('[main/ERROR]: boom', 'error')] });
    await emit('log', { id: 'other', key: 1, start: 3, lines: [line('not mine')] });
    await settle();
    expect(lines()).toEqual(['Launching', '[main/INFO]: hi', '[main/ERROR]: boom']);
    expect(document.querySelector('.line.error')).toHaveTextContent('boom');
    // A gap: the whole log is fetched again.
    log = { id: 'mod', key: 1, start: 0, lines: [...log.lines, line('a'), line('b'), line('c')] };
    await emit('log', { id: 'mod', key: 1, start: 5, lines: [line('c')] });
    await settle();
    expect(lines()).toEqual(['Launching', '[main/INFO]: hi', 'a', 'b', 'c']);
    expect(backend.called('get_log')).toHaveLength(2);
  });

  it('filters by severity and text, copies what is shown', async () => {
    const backend = new Backend(snapshot(), { 'plugin:clipboard-manager|write_text': () => null });
    render(LogView, { lines: [line('ok'), line('careful', 'warn'), line('Boom', 'error')], empty: ['a', 'b'] });
    await settle();
    await fireEvent.click(screen.getByRole('radio', { name: 'Warnings' }));
    expect(screen.getByText('2 of 3 lines')).toBeInTheDocument();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Filter' }), { target: { value: 'boo' } });
    expect(screen.getByText('1 of 3 lines')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy shown lines' }));
    await settle();
    expect(backend.called('plugin:clipboard-manager|write_text')[0]).toMatchObject({ text: 'Boom' });
    await fireEvent.input(screen.getByRole('textbox', { name: 'Filter' }), { target: { value: 'zzz' } });
    expect(screen.getByText('No matching lines')).toBeInTheDocument();
  });

  it('shares on mclo.gs after asking', async () => {
    const backend = new Backend(snapshot(), {
      upload_log: () => 'https://mclo.gs/abc',
      'plugin:clipboard-manager|write_text': () => null,
      'plugin:opener|open_url': () => null,
    });
    backend.answer('Upload');
    render(LogView, { lines: [line('crash')], empty: ['a', 'b'] });
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: 'Share on mclo.gs' }));
    await settle(10);
    expect(backend.called('upload_log')).toEqual([{ text: 'crash' }]);
    expect(backend.called('plugin:opener|open_url')[0]).toMatchObject({ url: 'https://mclo.gs/abc' });
  });

  it('shows the last run before any launch', async () => {
    new Backend(snapshot({ instances: [instance({ id: 'mod' })] }), {
      get_log: () => null,
      latest_log: () => [line('from file')],
    });
    render(LogView, { session: 'mod', empty: ['Nothing', 'yet'] });
    await settle();
    expect(screen.getByText(/From the last run/)).toBeInTheDocument();
    expect(screen.getByText('from file')).toBeInTheDocument();
  });
});

describe('ContentPage', () => {
  function content(handlers: Record<string, Handler> = {}, over = {}) {
    let items = [item('Sodium'), item('Lithium', { enabled: false, path: '/data/instances/mod/minecraft/mods/Lithium.jar.disabled' })];
    return setup('mods', over, {
      content_list: () => items,
      content_identify: () => ({ [items[0].path]: version('sodium-id', '0.5') }),
      content_set_enabled: ({ item: it, enabled }) => {
        const i = it as Item;
        const path = enabled ? i.path.replace('.disabled', '') : `${i.path}.disabled`;
        items = items.map((x) => (x.path === i.path ? { ...x, path, enabled: enabled as boolean } : x));
        return path;
      },
      content_delete: ({ item: it }) => {
        items = items.filter((x) => x.path !== (it as Item).path);
        return null;
      },
      content_check_updates: () => [{ item: items[0], current: '0.5', latest: version('sodium-id', '0.6') } satisfies Update],
      content_update: () => 'Updated Sodium',
      content_add_files: () => 2,
      addon_scope: () => ({ loaders: ['fabric'], game_version: '1.21.1' }),
      search_projects: () => ({ projects: [{ id: 'sodium-id', slug: 'sodium', title: 'Sodium', author: 'x', summary: 's', downloads: 5, icon_url: null }, { id: 'iris', slug: 'iris', title: 'Iris', author: 'y', summary: 'shaders', downloads: 4, icon_url: null }], total: 2 }),
      project_versions: () => [version('iris', '1.8')],
      content_install: () => 'Installed Iris',
      'plugin:opener|open_url': () => null,
      ...handlers,
    });
  }

  it('lists files with their state and totals', async () => {
    content();
    await settle();
    expect(screen.getByText('Sodium')).toBeInTheDocument();
    expect(screen.getByText('Off')).toBeInTheDocument();
    expect(screen.getByText('2 mods, 1 off · 2.0 MB')).toBeInTheDocument();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Filter mods' }), { target: { value: 'lith' } });
    expect(screen.queryByText('Sodium')).toBeNull();
  });

  it('turns files on and off', async () => {
    const backend = content();
    await settle();
    await fireEvent.click(screen.getAllByRole('switch', { name: 'Turn off' })[0]);
    await settle();
    expect(backend.called('content_set_enabled')[0]).toMatchObject({ kind: 'mods', enabled: false, item: { name: 'Sodium' } });
    expect(screen.getAllByRole('switch', { name: 'Turn on' })).toHaveLength(2);
    await fireEvent.click(screen.getByRole('button', { name: 'Turn all on' }));
    await settle();
    expect(screen.getAllByRole('switch', { name: 'Turn off' })).toHaveLength(2);
  });

  it('removes a file after asking', async () => {
    const backend = content();
    backend.answer('Remove');
    await settle();
    await fireEvent.click(screen.getAllByRole('button', { name: 'Remove' })[0]);
    await settle();
    expect(backend.called('content_delete')[0]).toMatchObject({ item: { name: 'Sodium' } });
    expect(screen.getByText('Removed Sodium')).toBeInTheDocument();
  });

  it('finds and applies updates', async () => {
    const backend = content();
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await settle();
    expect(screen.getByText('1 update available')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Update all' }));
    await settle();
    expect(backend.called('content_update')[0]).toMatchObject({ kind: 'mods', updates: [{ latest: { number: '0.6' } }] });
    expect(screen.getByText('Updated Sodium')).toBeInTheDocument();
  });

  it('adds dropped files', async () => {
    const backend = content();
    await settle();
    await emit('tauri://drag-drop', { paths: ['/x/a.jar', '/x/b.jar'], position: { x: 0, y: 0 } });
    await settle();
    expect(backend.called('content_add_files')).toEqual([{ id: 'mod', kind: 'mods', paths: ['/x/a.jar', '/x/b.jar'] }]);
    expect(screen.getByText('Added 2 mods')).toBeInTheDocument();
  });

  it('browses Modrinth, knows what is installed and installs', async () => {
    const backend = content();
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: 'Get mods' }));
    await settle();
    expect(screen.getByText('Showing mods for 1.21.1 · Fabric')).toBeInTheDocument();
    const rows = screen.getAllByRole('option');
    expect(within(rows[0]).getByRole('button', { name: 'Installed' })).toBeDisabled();
    await fireEvent.click(within(rows[1]).getByRole('button', { name: 'Install' }));
    await settle();
    expect(backend.called('content_install')[0]).toMatchObject({ project: { id: 'iris' }, version: null, installed: ['sodium-id'] });
    expect(screen.getByText('Installed Iris')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Installed mods' }));
    expect(screen.getByRole('textbox', { name: 'Filter mods' })).toBeInTheDocument();
  });

  it('asks for a loader on vanilla instances', async () => {
    const backend = new Backend(snapshot({ instances: [instance({ id: 'van' })] }), { content_list: () => [], get_log: () => null, latest_log: () => null, list_versions: () => [], 'plugin:window|set_title': () => null }, 'instance-76616e');
    render(InstanceWindow, { id: 'van', initialPage: 'mods' });
    await settle();
    expect(screen.getByText('This instance has no mod loader')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Get mods' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Choose a mod loader' }));
    await settle();
    expect(screen.getByRole('button', { name: /Settings/ })).toHaveClass('active');
    expect(backend.called('list_versions')).toHaveLength(1);
  });
});
