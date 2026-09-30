import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import AddInstance from '../add/AddInstance.svelte';
import { Backend, instance, settle, snapshot } from '../../test/backend';
import type { Handler } from '../../test/backend';
import type { Pack, PackVersion, VersionEntry } from '../../lib/types';

afterEach(() => Backend.reset());

const v = (id: string, kind = 'release', installed = false): VersionEntry => ({ id, kind, url: null, sha1: null, installed });
const VERSIONS = [v('1.21.4'), v('25w01a', 'snapshot'), v('1.21.1', 'release', true), v('1.20.1'), v('b1.7.3', 'old_beta'), v('fabric-loader-1.21', 'local')];

const pack = (id: string, title: string): Pack => ({
  platform: 'modrinth', id, title, author: 'someone', summary: `${title}\nsummary`, downloads: 1_234_567, icon_url: null, website: `https://modrinth.com/modpack/${id}`,
});
const packVersion = (id: string, game: string, loader: string, url: string | null = 'https://cdn/x.mrpack'): PackVersion => ({
  id, name: `${id} for ${game}`, game_versions: [game], loaders: [loader], url, file_name: 'x.mrpack', sha1: null, size: 10,
});

function setup(props: Record<string, unknown> = {}, handlers: Record<string, Handler> = {}, over = {}) {
  const backend = new Backend(snapshot(over), {
    list_versions: () => VERSIONS,
    supported_versions: ({ loader }) => (loader === 'fabric' ? ['1.21.1', '1.20.1'] : null),
    loader_versions: () => [{ version: '0.16.9', stable: true }, { version: '0.17.0-beta', stable: false }],
    create_instance: (args) => instance({ id: 'new', name: args.name as string }),
    search_modpacks: ({ query }) => ({
      packs: query === 'none' ? [] : [pack('fo', 'Fabulously Optimized'), pack('aof', 'All of Fabric')],
      total: query === 'none' ? 0 : 2,
    }),
    modpack_versions: ({ id }) =>
      id === 'fo'
        ? [packVersion('6.0', '1.21.1', 'Fabric'), packVersion('5.0', '1.20.1', 'Fabric')]
        : [packVersion('1.0', '1.21.1', 'Fabric', null)],
    install_modpack: () => null,
    import_files: () => null,
    set_curseforge_key: () => null,
    ...handlers,
  });
  const events = { onclose: vi.fn(), onimport: vi.fn(), oncreated: vi.fn(), oninstall: vi.fn() };
  render(AddInstance, { ...events, ...props });
  return { backend, ...events };
}

const versionNames = () => screen.queryAllByRole('option').map((o) => o.textContent?.trim().split(/\s/)[0]);

describe('AddInstance', () => {
  it('lists releases, with snapshots and old versions on request', async () => {
    setup();
    await settle();
    expect(versionNames()).toEqual(['1.21.4', '1.21.1', '1.20.1']);
    expect(screen.getByRole('option', { name: /1\.21\.1/ })).toHaveTextContent('installed');
    await fireEvent.click(screen.getByRole('button', { name: 'Snapshots' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Old' }));
    expect(versionNames()).toEqual(['1.21.4', '25w01a', '1.21.1', '1.20.1', 'b1.7.3']);
    await fireEvent.input(screen.getByRole('textbox', { name: 'Search versions' }), { target: { value: '1.20' } });
    expect(versionNames()).toEqual(['1.20.1']);
  });

  it('creates a vanilla instance named after the version', async () => {
    const { backend, oncreated } = setup();
    await settle();
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveAttribute('placeholder', '1.21.4');
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await settle();
    expect(backend.called('create_instance')).toEqual([{ name: '1.21.4', minecraft: '1.21.4', loader: 'vanilla', loaderVersion: '' }]);
    expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ id: 'new' }));
  });

  it('narrows versions to what the loader supports and picks a build', async () => {
    const { backend } = setup();
    await settle();
    await fireEvent.click(screen.getByRole('radio', { name: 'Fabric' }));
    await settle();
    expect(versionNames()).toEqual(['1.21.1', '1.20.1']);
    expect(screen.getByText('The newest stable Fabric build is installed on first launch.')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Build: Latest stable' }));
    await fireEvent.click(screen.getByRole('option', { name: '0.17.0-beta (beta)' }));
    expect(screen.getByText('This Fabric build is installed on first launch.')).toBeInTheDocument();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Name' }), { target: { value: '  My pack ' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await settle();
    expect(backend.called('create_instance')).toEqual([{ name: 'My pack', minecraft: '1.21.1', loader: 'fabric', loaderVersion: '0.17.0-beta' }]);
    expect(backend.called('loader_versions')).toContainEqual({ loader: 'fabric', minecraft: '1.21.1' });
  });

  it('moves through versions with the keyboard and creates on Enter', async () => {
    const { backend, onclose } = setup();
    await settle();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    await fireEvent.keyDown(window, { key: 'ArrowUp' });
    expect(screen.getByRole('option', { selected: true })).toHaveTextContent('1.21.1');
    await fireEvent.keyDown(window, { key: 'Enter' });
    await settle();
    expect(backend.called('create_instance')[0]).toMatchObject({ minecraft: '1.21.1' });
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onclose).toHaveBeenCalled();
  });

  it('shows why a version list could not load, and creation errors', async () => {
    setup({}, {
      list_versions: () => Promise.reject('offline'),
    });
    await settle();
    expect(screen.getByText('Could not load the version list: offline')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create' })).toBeDisabled();
  });

  it('imports chosen files', async () => {
    const { backend, onimport } = setup({}, { 'plugin:dialog|open': () => ['/a.mrpack', '/b.zip'] });
    await fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    expect(screen.getByText('Drop modpack files here')).toBeInTheDocument();
    expect(screen.getByText('.zip (needs an API key, see the CurseForge page)')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Choose files…' }));
    await settle();
    expect(backend.called('plugin:dialog|open')[0]).toMatchObject({ options: { multiple: true } });
    expect(onimport).toHaveBeenCalledWith(['/a.mrpack', '/b.zip']);
  });

  it('asks for a CurseForge key first', async () => {
    const { backend } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'CurseForge' }));
    expect(screen.getByText('CurseForge API key needed')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save key' })).toBeDisabled();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Paste your API key' }), { target: { value: ' key ' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save key' }));
    expect(backend.called('set_curseforge_key')).toEqual([{ key: 'key' }]);
    // With a key the browser shows.
    await backend.push({ ...backend.snap, settings: { ...backend.snap.settings, curseforge_api_key: 'key' } });
    await settle();
    expect(backend.called('search_modpacks').at(-1)).toMatchObject({ platform: 'curseforge' });
  });

  it('browses Modrinth modpacks and installs the selected version', async () => {
    const { backend, oninstall } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Modrinth' }));
    await settle();
    const list = screen.getByRole('listbox', { name: 'Modpacks' });
    expect(within(list).getAllByRole('option').map((o) => o.textContent)).toEqual([
      expect.stringContaining('Fabulously Optimized'),
      expect.stringContaining('All of Fabric'),
    ]);
    expect(screen.getAllByText('1.2M downloads', { exact: false })).toHaveLength(2);
    const versions = screen.getByRole('listbox', { name: 'Versions' });
    expect(within(versions).getAllByRole('option')).toHaveLength(2);
    await fireEvent.click(within(versions).getAllByRole('option')[1]);
    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await settle();
    expect(backend.called('install_modpack')[0]).toMatchObject({ pack: { id: 'fo' }, version: { id: '5.0' } });
    expect(oninstall).toHaveBeenCalled();
  });

  it('filters modpacks and their versions', async () => {
    const { backend } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Modrinth' }));
    await settle();
    await fireEvent.click(screen.getByRole('button', { name: 'Minecraft: Any' }));
    await fireEvent.click(screen.getByRole('option', { name: '1.20.1' }));
    await settle();
    expect(backend.called('search_modpacks').at(-1)).toMatchObject({ filters: { game_version: '1.20.1', loader: null, sort: 'relevance' } });
    const versions = screen.getByRole('listbox', { name: 'Versions' });
    expect(within(versions).getAllByRole('option').map((o) => o.textContent)).toEqual([expect.stringContaining('5.0 for 1.20.1')]);
    await fireEvent.click(screen.getByRole('button', { name: 'Sort: Relevance' }));
    await fireEvent.click(screen.getByRole('option', { name: 'Downloads' }));
    await settle();
    expect(backend.called('search_modpacks').at(-1)).toMatchObject({ filters: { sort: 'downloads' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    await settle();
    expect(backend.called('search_modpacks').at(-1)).toMatchObject({ filters: { game_version: null, sort: 'downloads' } });
  });

  it('searches after typing stops', async () => {
    vi.useFakeTimers();
    try {
      const { backend } = setup();
      await fireEvent.click(screen.getByRole('button', { name: 'Modrinth' }));
      await vi.advanceTimersByTimeAsync(10);
      const before = backend.called('search_modpacks').length;
      const search = screen.getByRole('textbox', { name: 'Search modpacks' });
      await fireEvent.input(search, { target: { value: 'no' } });
      await fireEvent.input(search, { target: { value: 'none' } });
      await vi.advanceTimersByTimeAsync(100);
      expect(backend.called('search_modpacks').length).toBe(before);
      await vi.advanceTimersByTimeAsync(400);
      expect(backend.called('search_modpacks').slice(before)).toEqual([expect.objectContaining({ query: 'none' })]);
      expect(screen.getByText('Nothing found')).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('can not install versions the author keeps from launchers', async () => {
    setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Modrinth' }));
    await settle();
    await fireEvent.click(screen.getAllByRole('option', { name: /All of Fabric/ })[0]);
    await settle();
    expect(screen.getByText('The author does not allow launchers to download this version')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled();
  });
});
