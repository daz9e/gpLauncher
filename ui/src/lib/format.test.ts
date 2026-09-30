import { describe, expect, it } from 'vitest';
import {
  ago,
  capitalize,
  count,
  duration,
  fileStem,
  humanSize,
  isError,
  loaderText,
  memoryLabel,
  oneLine,
  parseMemory,
  parseResolution,
  shortDescription,
  shortNumber,
  validOfflineName,
  versionKindLabel,
} from './format';
import { instance } from '../test/backend';

describe('format', () => {
  it('says how long ago', () => {
    const now = 1_000_000;
    expect(ago(0, now)).toBe('Never');
    expect(ago(now - 10, now)).toBe('Just now');
    expect(ago(now - 60, now)).toBe('1 minute ago');
    expect(ago(now - 7200, now)).toBe('2 hours ago');
    expect(ago(now - 90000, now)).toBe('Yesterday');
    expect(ago(now - 86400 * 12, now)).toBe('12 days ago');
    expect(ago(now - 86400 * 90, now)).toBe('3 months ago');
    expect(ago(now - 86400 * 800, now)).toBe('2 years ago');
  });

  it('formats durations, numbers and sizes like the core', () => {
    expect(duration(40)).toBe('40s');
    expect(duration(720)).toBe('12m');
    expect(duration(3725)).toBe('1h 02m');
    expect(shortNumber(42)).toBe('42');
    expect(shortNumber(950_400)).toBe('950K');
    expect(shortNumber(17_700_000)).toBe('17.7M');
    expect(humanSize(512)).toBe('512 B');
    expect(humanSize(2048)).toBe('2 KB');
    expect(humanSize(5 * 1_048_576)).toBe('5.0 MB');
    expect(humanSize(3 * 1_073_741_824)).toBe('3.00 GB');
    expect(memoryLabel(4096)).toBe('4 GB');
    expect(memoryLabel(1536)).toBe('1536 MB');
  });

  it('describes instances and content', () => {
    expect(shortDescription(instance({ loader: 'neoforge', minecraft: '1.21.1' }))).toBe('1.21.1 · NeoForge');
    expect(shortDescription(instance({ loader: 'vanilla', minecraft: '1.20' }))).toBe('1.20');
    expect(loaderText(instance({ loader: 'fabric' }))).toBe('Fabric (latest)');
    expect(loaderText(instance({ loader: 'forge', loader_version: '52.0.1' }))).toBe('Forge 52.0.1');
    expect(loaderText(instance())).toBe('None');
    expect(count(1, 'mods')).toBe('1 mod');
    expect(count(3, 'resourcepacks')).toBe('3 resource packs');
    expect(capitalize('neoforge')).toBe('NeoForge');
    expect(capitalize('fabric')).toBe('Fabric');
    expect(versionKindLabel('old_alpha')).toBe('Alpha');
    expect(versionKindLabel('local')).toBe('');
    expect(oneLine(' a\n  b\tc ')).toBe('a b c');
    expect(fileStem('/x/2024-01-01_12.00.00.png')).toBe('2024-01-01_12.00.00');
    expect(fileStem('C:\\shots\\a.png')).toBe('a');
    expect(isError('Error: x')).toBe(true);
    expect(isError('Game crashed (exit code 1)')).toBe(true);
    expect(isError('Game closed')).toBe(false);
  });

  it('validates fields like the settings pages', () => {
    expect(parseMemory('')).toEqual({ ok: true, mb: null });
    expect(parseMemory('4096')).toEqual({ ok: true, mb: 4096 });
    expect(parseMemory('100')).toEqual({ ok: false, error: 'At least 512 MB' });
    expect(parseMemory('4g')).toEqual({ ok: false, error: 'At least 512 MB' });
    expect(parseResolution('', '')).toEqual({ ok: true, width: null, height: null });
    expect(parseResolution('1280', '720')).toEqual({ ok: true, width: 1280, height: 720 });
    expect(parseResolution('1280', '')).toEqual({ ok: false, error: 'Set both width and height, or neither' });
    expect(parseResolution('wide', '720')).toEqual({ ok: false, error: 'Width and height are numbers of pixels' });
    expect(parseResolution('0', '720').ok).toBe(false);
    expect(validOfflineName('Steve_1')).toBe(true);
    expect(validOfflineName('no')).toBe(false);
    expect(validOfflineName('with space')).toBe(false);
    expect(validOfflineName('a'.repeat(17))).toBe(false);
  });
});
