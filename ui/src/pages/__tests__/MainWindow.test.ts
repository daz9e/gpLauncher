import { afterEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { emit } from '@tauri-apps/api/event';
import MainWindow from '../MainWindow.svelte';
import { Backend, instance, settle, snapshot } from '../../test/backend';
import type { Handler } from '../../test/backend';

afterEach(() => Backend.reset());

function setup(over = {}, handlers: Record<string, Handler> = {}) {
  const backend = new Backend(
    snapshot({
      instances: [
        instance({ id: 'fab', name: 'Fabric World', loader: 'fabric', description: '1.21.1 · Fabric', group: 'Modded', play_time: 3725 }),
        instance({ id: 'van', name: 'Vanilla', minecraft: '1.20.4', description: '1.20.4' }),
        instance({ id: 'old', name: 'Beta', minecraft: 'b1.7.3', description: 'b1.7.3' }),
      ],
      ...over,
    }),
    { launch: () => true, kill: () => null, open_instance_window: () => null, ...handlers },
  );
  render(MainWindow);
  return backend;
}

const tile = (name: string) => screen.getByRole('button', { name });

describe('MainWindow', () => {
  it('shows instances grouped, ungrouped first, and the first one in the sidebar', () => {
    setup();
    const groups = screen.getAllByRole('button', { expanded: true }).map((b) => b.textContent);
    expect(groups[0]).toContain('Ungrouped');
    expect(groups[1]).toContain('Modded');
    const sidebar = screen.getByRole('complementary', { name: 'Selected instance' });
    expect(within(sidebar).getByText('Fabric World')).toBeInTheDocument();
    expect(within(sidebar).getByText('1h 02m')).toBeInTheDocument();
    expect(within(sidebar).getByText('Fabric (latest)')).toBeInTheDocument();
    expect(screen.getByTestId('status-text')).toHaveTextContent('Ready');
  });

  it('filters by name, version or group and folds groups', async () => {
    setup();
    const search = screen.getByRole('textbox', { name: 'Search instances' });
    await fireEvent.input(search, { target: { value: 'b1.7' } });
    expect(screen.queryByRole('button', { name: 'Fabric World' })).toBeNull();
    expect(tile('Beta')).toBeInTheDocument();
    await fireEvent.input(search, { target: { value: 'modded' } });
    expect(tile('Fabric World')).toBeInTheDocument();
    await fireEvent.input(search, { target: { value: 'zzz' } });
    expect(screen.getByText('No instances match')).toBeInTheDocument();
    await fireEvent.input(search, { target: { value: '' } });
    await fireEvent.click(screen.getByRole('button', { name: /Modded/ }));
    expect(screen.queryByRole('button', { name: 'Fabric World' })).toBeNull();
  });

  it('sorts by name', async () => {
    setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Sort: Last played' }));
    await fireEvent.click(screen.getByRole('option', { name: 'Name' }));
    const names = [...document.querySelectorAll('.tile')].map((t) => t.getAttribute('aria-label'));
    // Ungrouped first, each group by name.
    expect(names).toEqual(['Beta', 'Vanilla', 'Fabric World']);
  });

  it('selects on click, launches on double click and from the tile button', async () => {
    const backend = setup();
    await fireEvent.click(tile('Vanilla'), { detail: 1 });
    const sidebar = screen.getByRole('complementary', { name: 'Selected instance' });
    expect(within(sidebar).getByText('Vanilla')).toBeInTheDocument();
    expect(backend.called('launch')).toEqual([]);
    await fireEvent.click(tile('Vanilla'), { detail: 2 });
    expect(backend.called('launch')).toEqual([{ id: 'van' }]);
    await fireEvent.click(screen.getByRole('button', { name: 'Play Beta' }));
    expect(backend.called('launch').at(-1)).toEqual({ id: 'old' });
    await fireEvent.click(within(sidebar).getByRole('button', { name: 'Open' }));
    await settle();
    expect(backend.called('open_instance_window').at(-1)).toEqual({ id: 'old', page: 'mods' });
  });

  it('follows state events: running games, jobs and the status bar', async () => {
    const backend = setup();
    await backend.push({
      ...backend.snap,
      sessions: { fab: { phase: 'running', status: 'Playing', progress: null, played: null, started_at: 0, key: 1 } },
      running: 1,
      busy: true,
      job: { active: true, status: 'Importing', progress: [1, 4] },
    });
    await settle();
    expect(screen.getByText('1 game running')).toBeInTheDocument();
    expect(screen.getByTestId('status-text')).toHaveTextContent('Importing');
    expect(screen.getByText('25%')).toBeInTheDocument();
    const sidebar = screen.getByRole('complementary', { name: 'Selected instance' });
    await fireEvent.click(within(sidebar).getByRole('button', { name: 'Stop' }));
    expect(backend.called('kill')).toEqual([{ id: 'fab' }]);
    expect(within(sidebar).getByRole('button', { name: /Delete/ })).toBeDisabled();
    expect(within(tile('Fabric World')).getByText('Playing')).toBeInTheDocument();
  });

  it('shows a crashed game in red with a link to its log', async () => {
    const backend = setup();
    await backend.push({
      ...backend.snap,
      sessions: { fab: { phase: 'finished', status: 'Game crashed (exit code 1)', progress: null, played: 3, started_at: 0, key: 1 } },
    });
    await settle();
    const sidebar = screen.getByRole('complementary', { name: 'Selected instance' });
    expect(within(sidebar).getByText('Game crashed (exit code 1)').closest('.session')).toHaveClass('error');
    await fireEvent.click(within(sidebar).getByRole('button', { name: 'Show the log' }));
    await settle();
    expect(backend.called('open_instance_window').at(-1)).toEqual({ id: 'fab', page: 'console' });
  });

  it('selects revealed instances and falls back when the selection goes away', async () => {
    const backend = setup();
    await emit('reveal', 'old');
    await settle();
    const sidebar = () => screen.getByRole('complementary', { name: 'Selected instance' });
    expect(within(sidebar()).getByText('Beta')).toBeInTheDocument();
    await backend.push({ ...backend.snap, instances: backend.snap.instances.filter((i) => i.id !== 'old') });
    await settle();
    expect(within(sidebar()).queryByText('Beta')).toBeNull();
  });

  it('deletes after asking, from the context menu', async () => {
    const backend = setup({}, { delete_instance: () => null });
    backend.answer('Delete');
    await fireEvent.contextMenu(tile('Vanilla'));
    const menu = screen.getByRole('menu');
    await fireEvent.click(within(menu).getByRole('menuitem', { name: 'Delete' }));
    await settle();
    expect(backend.called('plugin:dialog|message')[0]).toMatchObject({ title: 'Delete "Vanilla"?', kind: 'warning' });
    expect(backend.called('delete_instance')).toEqual([{ id: 'van' }]);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('does not delete when the answer is no', async () => {
    const backend = setup({}, { delete_instance: () => null });
    backend.answer('Cancel');
    await fireEvent.contextMenu(tile('Vanilla'));
    await fireEvent.click(within(screen.getByRole('menu')).getByRole('menuitem', { name: 'Delete' }));
    await settle();
    expect(backend.called('delete_instance')).toEqual([]);
  });

  it('imports dropped files', async () => {
    const backend = setup({}, { import_files: () => null });
    await settle();
    await emit('tauri://drag-drop', { paths: ['/tmp/pack.mrpack'], position: { x: 1, y: 1 } });
    await settle();
    expect(backend.called('import_files')).toEqual([{ paths: ['/tmp/pack.mrpack'] }]);
  });

  it('runs keyboard shortcuts and menu commands', async () => {
    const backend = setup();
    await fireEvent.keyDown(window, { key: 'Enter', metaKey: true });
    expect(backend.called('launch')).toEqual([{ id: 'fab' }]);
    await fireEvent.keyDown(window, { key: 'o', ctrlKey: true });
    await settle();
    expect(backend.called('open_instance_window')).toEqual([{ id: 'fab', page: 'mods' }]);
    await emit('menu', 'settings');
    await settle();
    expect(screen.getByRole('heading', { name: 'General' })).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('heading', { name: 'General' })).toBeNull();
  });

  it('offers to create the first instance', () => {
    new Backend(snapshot(), { list_versions: () => [] });
    render(MainWindow);
    expect(screen.getByText('No instances yet')).toBeInTheDocument();
  });
});
