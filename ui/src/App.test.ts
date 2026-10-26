// @vitest-environment jsdom
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: h.open, save: vi.fn() }));

import App from './App.svelte';
import { audio } from './lib/audio.svelte';
import { entry, installBackend, SAMPLE_FILES, type Backend } from './test/backend';

let backend: Backend;
const user = () => userEvent.setup();

async function mount(cfg: Parameters<typeof installBackend>[1] = {}) {
  backend = installBackend(h.invoke, cfg);
  const view = render(App);
  await screen.findByText('Serato Recording 1.aif');
  return view;
}

const rows = () => screen.getAllByTestId('row');
const rowFor = (name: string) => rows().find((r) => within(r).queryByText(name))!;

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  h.invoke.mockReset();
  h.open.mockReset();
  audio.stop();
  audio.state = { ...audio.state, loaded: false, playing: false, position_secs: 0, duration_secs: 0 };
  audio.label = '';
  audio.sourcePath = '';
});

describe('identity', () => {
  it('is named for what it does', async () => {
    await mount();
    expect(screen.getByLabelText('Recording Fixer for Serato DJ')).toHaveTextContent(/Recording\s*Fixer\s*for Serato DJ/i);
    expect(document.title === '' || /Recording Fixer for Serato DJ/.test(document.title)).toBe(true);
  });
});

describe('the recording list', () => {
  it('lists every recording with its status, length and format', async () => {
    await mount();
    expect(rows()).toHaveLength(4);
    const broken = rowFor('Serato Recording 5.aif');
    expect(within(broken).getByText('Needs repair')).toBeInTheDocument();
    expect(within(broken).getByText(/length is missing/)).toBeInTheDocument();
    expect(within(broken).getByText('19:08')).toBeInTheDocument();
    expect(within(broken).getByText('48 kHz · 16-bit · stereo')).toBeInTheDocument();
    expect(within(rowFor('Serato Recording 1.aif')).getByText('3:00:00')).toBeInTheDocument();
    expect(within(rowFor('Serato Recording 6.aif')).getByText('In use')).toBeInTheDocument();
    expect(within(rowFor('Serato Recording 2.aif')).getByText('2025-10-04 21:11:51')).toBeInTheDocument();
  });

  it('scans the default folders on start', async () => {
    await mount();
    expect(backend.of('scan')[0]).toEqual({ folders: ['C:/rec', 'C:/rec temp'] });
  });

  it('shows counts on the filter tabs and filters by them', async () => {
    const u = user();
    await mount();
    expect(screen.getByRole('tab', { name: /Needs repair/ })).toHaveTextContent('1');
    expect(screen.getByRole('tab', { name: /^OK/ })).toHaveTextContent('2');
    await u.click(screen.getByRole('tab', { name: /Needs repair/ }));
    expect(rows()).toHaveLength(1);
    expect(rows()[0]).toHaveTextContent('Serato Recording 5.aif');
    await u.click(screen.getByRole('tab', { name: /^All/ }));
    expect(rows()).toHaveLength(4);
  });

  it('searches by name', async () => {
    const u = user();
    await mount();
    await u.type(screen.getByRole('searchbox', { name: 'Search' }), 'recording 2');
    expect(rows()).toHaveLength(1);
    expect(rows()[0]).toHaveTextContent('Serato Recording 2.aif');
  });

  it('sorts when a column header is clicked, and reverses on a second click', async () => {
    const u = user();
    await mount();
    const names = () => rows().map((r) => r.querySelector('.name')!.textContent);
    await u.click(screen.getByRole('button', { name: /^Name/ }));
    expect(names()).toEqual(['Serato Recording 1.aif', 'Serato Recording 2.aif', 'Serato Recording 5.aif', 'Serato Recording 6.aif']);
    await u.click(screen.getByRole('button', { name: /^Name/ }));
    expect(names()[0]).toBe('Serato Recording 6.aif');
    await u.click(screen.getByRole('button', { name: /^Length/ }));
    expect(names()[0]).toBe('Serato Recording 1.aif'); // longest first
  });

  it('tells you when there is nothing to show', async () => {
    backend = installBackend(h.invoke, { files: [] });
    render(App);
    expect(await screen.findByText(/No recordings found/)).toBeInTheDocument();
  });

  it('says so, with the reason, when the scan itself fails', async () => {
    backend = installBackend(h.invoke, { failures: { scan: 'disk exploded' } });
    render(App);
    expect(await screen.findByText(/Scan failed: disk exploded/)).toBeInTheDocument();
  });
});

describe('selection and actions', () => {
  it('enables Repair only when a broken file is selected', async () => {
    const u = user();
    await mount();
    const repair = screen.getByRole('button', { name: /Repair/ });
    expect(repair).toBeDisabled();
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    expect(repair).toBeDisabled();
    await u.click(within(rowFor('Serato Recording 5.aif')).getByRole('checkbox'));
    expect(repair).toBeEnabled();
    await u.click(within(rowFor('Serato Recording 6.aif')).getByRole('checkbox')); // in use: never repairable
    expect(repair).toBeDisabled();
  });

  it('clicking anywhere on a row selects it and shows the count', async () => {
    const u = user();
    await mount();
    await u.click(rowFor('Serato Recording 1.aif'));
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    await u.click(rowFor('Serato Recording 2.aif'));
    expect(screen.getByText('2 selected')).toBeInTheDocument();
    await u.click(rowFor('Serato Recording 1.aif'));
    expect(screen.getByText('1 selected')).toBeInTheDocument();
  });

  it('select-all selects what is visible', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('checkbox', { name: 'Select all' }));
    expect(screen.getByText('4 selected')).toBeInTheDocument();
    await u.click(screen.getByRole('checkbox', { name: 'Select all' }));
    expect(screen.queryByText(/selected/)).not.toBeInTheDocument();
  });

  it('repairs a copy by default and reports where it went', async () => {
    const u = user();
    await mount();
    await u.click(within(rowFor('Serato Recording 5.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: /Repair/ }));
    const dialog = screen.getByRole('dialog', { name: 'Repair recordings' });
    await u.click(within(dialog).getByRole('button', { name: 'Repair' }));
    await waitFor(() => expect(backend.of('repair')).toHaveLength(1));
    const broken = SAMPLE_FILES.find((f) => f.status === 'broken')!;
    expect(backend.of('repair')[0]).toEqual({ paths: [broken.path], inPlace: false });
    expect(await screen.findByText(/Saved repaired copy of Serato Recording 5.aif/)).toBeInTheDocument();
    expect(backend.of('scan').length).toBeGreaterThanOrEqual(2); // the list refreshes afterwards
  });

  it('can repair in place instead', async () => {
    const u = user();
    await mount();
    await u.click(within(rowFor('Serato Recording 5.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: /Repair/ }));
    const dialog = screen.getByRole('dialog', { name: 'Repair recordings' });
    await u.click(within(dialog).getByRole('radio', { name: /Fix in place/ }));
    await u.click(within(dialog).getByRole('button', { name: 'Repair' }));
    await waitFor(() => expect(backend.of('repair')[0]).toMatchObject({ inPlace: true }));
    expect(await screen.findByText(/Fixed Serato Recording 5.aif/)).toBeInTheDocument();
  });

  it('cancelling the repair dialog does nothing', async () => {
    const u = user();
    await mount();
    await u.click(within(rowFor('Serato Recording 5.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: /Repair/ }));
    await u.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.of('repair')).toHaveLength(0);
  });

  it('opens the editor for the selection, labelled for combining when several are chosen', async () => {
    const u = user();
    await mount();
    const open = screen.getByRole('button', { name: /Edit & export|Combine & export/ });
    expect(open).toBeDisabled();
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    expect(screen.getByRole('button', { name: 'Edit & export…' })).toBeEnabled();
    await u.click(within(rowFor('Serato Recording 1.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Combine & export…' }));
    expect(await screen.findByRole('dialog', { name: 'Editor' })).toBeInTheDocument();
  });

  it('will not open the editor for an unreadable file', async () => {
    const u = user();
    await mount({ files: [...SAMPLE_FILES, entry({ name: 'junk.aif', status: 'unreadable', detail: 'not an AIFF file' })] });
    await u.click(within(rowFor('junk.aif')).getByRole('checkbox'));
    expect(screen.getByRole('button', { name: /Edit & export/ })).toBeDisabled();
  });
});

describe('exporting from the list', () => {
  it('sends the chosen format, tags and fades to the backend for each file', async () => {
    const u = user();
    await mount();
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Edit & export…' }));
    const editor = await screen.findByRole('dialog', { name: 'Editor' });
    await u.type(within(editor).getByLabelText('Fade in (seconds)'), '2');
    await u.type(within(editor).getByLabelText('Fade out (seconds)'), '5');
    await u.click(within(editor).getByRole('radio', { name: /MP3/ }));
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(1));
    const job = (backend.of('export_audio')[0] as { job: Record<string, unknown> }).job;
    expect(job).toMatchObject({ file_stem: 'Serato Recording 2', fade_in_secs: 2, fade_out_secs: 5, target: { kind: 'mp3', mode: 'cbr', value: 320 } });
    expect(await screen.findByText(/Exported Serato Recording 2.out/)).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Editor' })).not.toBeInTheDocument(); // the editor closes
  });

  it('reports an export that fails and keeps going with the next file', async () => {
    const u = user();
    await mount();
    await u.click(within(rowFor('Serato Recording 1.aif')).getByRole('checkbox'));
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Combine & export…' }));
    const editor = await screen.findByRole('dialog', { name: 'Editor' });
    await u.click(within(editor).getByRole('radio', { name: /One file per clip/ }));
    backend.fail('export_audio', 'disk full');
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(2));
    expect((await screen.findAllByText('disk full')).length).toBe(2);
  });
});

describe('folders', () => {
  const RECORDING = /^Recording(?! temp)/;
  const chip = (name: RegExp) => screen.getByRole('checkbox', { name }).closest('label') as HTMLElement;

  it('says what the row is for and explains each Serato folder', async () => {
    await mount();
    expect(screen.getByText('Show recordings from')).toBeInTheDocument();
    // No inline jargon next to the name; hovering gives the full path and what the folder is.
    expect(chip(RECORDING)).not.toHaveTextContent(/finished/i);
    expect(chip(/Recording temp/)).not.toHaveTextContent(/finished/i);
    expect(chip(RECORDING).title).toMatch(/C:\/rec\n\nSerato saves recordings here when you stop them normally/);
    expect(chip(/Recording temp/).title).toMatch(/crashes before finishing/);
  });

  it('explains the folders on first run, once, until dismissed', async () => {
    const u = userEvent.setup();
    await mount();
    const note = screen.getByRole('complementary', { name: 'About these folders' });
    expect(note).toHaveTextContent(/Recording temp/);
    expect(note).toHaveTextContent(/usually need repair/);
    await u.click(within(note).getByRole('button', { name: 'Got it' }));
    expect(screen.queryByRole('complementary', { name: 'About these folders' })).toBeNull();
    expect(JSON.parse(localStorage.getItem('folderHelpSeen')!)).toBe(true);
  });

  it('stays hidden once seen, and the info button shows one folder on demand', async () => {
    const u = userEvent.setup();
    localStorage.setItem('folderHelpSeen', 'true');
    await mount();
    expect(screen.queryByRole('complementary', { name: 'About these folders' })).toBeNull();
    const info = screen.getByRole('button', { name: 'What is Recording temp?' });
    await u.click(info);
    const note = screen.getByRole('complementary', { name: 'About these folders' });
    expect(note).toHaveTextContent(/leaves them here if it is closed or crashes/);
    // Clicking ⓘ must not tick or untick the folder.
    expect(screen.getByRole('checkbox', { name: /Recording temp/ })).toBeChecked();
    await u.click(info);
    expect(screen.queryByRole('complementary', { name: 'About these folders' })).toBeNull();
  });

  it('shows how many recordings each folder holds', async () => {
    await mount();
    expect(chip(RECORDING).querySelector('.cnt')).toHaveTextContent('2');
    expect(chip(/Recording temp/).querySelector('.cnt')).toHaveTextContent('2');
  });

  it('a folder that does not exist says so instead of showing a count', async () => {
    await mount({ missingFolders: ['C:/rec temp'] });
    expect(chip(/Recording temp/)).toHaveTextContent('not found');
    expect(chip(/Recording temp/).querySelector('.cnt')).toBeNull();
  });

  it('the folder choices are real checkboxes, ticked when the folder is shown', async () => {
    await mount();
    expect(screen.getByRole('checkbox', { name: /Recording temp/ })).toBeChecked();
    await user().click(screen.getByRole('checkbox', { name: /Recording temp/ }));
    expect(screen.getByRole('checkbox', { name: /Recording temp/ })).not.toBeChecked();
  });

  it('switching a folder off removes its recordings from the scan and the list', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('checkbox', { name: /Recording temp/ }));
    await waitFor(() => expect(backend.of('scan').at(-1)).toEqual({ folders: ['C:/rec'] }));
  });

  it('remembers an added folder and rescans including it', async () => {
    const u = user();
    await mount();
    h.open.mockResolvedValue('D:/Gigs');
    await u.click(screen.getByRole('button', { name: '+ Add folder…' }));
    await waitFor(() => expect(backend.of('scan').at(-1)).toEqual({ folders: ['C:/rec', 'C:/rec temp', 'D:/Gigs'] }));
    expect(JSON.parse(localStorage.getItem('folders')!)).toContainEqual(expect.objectContaining({ path: 'D:/Gigs', custom: true }));
    expect(screen.getByRole('button', { name: 'Remove Gigs' })).toBeInTheDocument();
    expect(chip(/Gigs/).title).toMatch(/A folder you added/);
  });

  it('ignores a cancelled folder picker and duplicates', async () => {
    const u = user();
    await mount();
    const scans = backend.of('scan').length;
    h.open.mockResolvedValue(null);
    await u.click(screen.getByRole('button', { name: '+ Add folder…' }));
    h.open.mockResolvedValue('C:/rec');
    await u.click(screen.getByRole('button', { name: '+ Add folder…' }));
    expect(backend.of('scan')).toHaveLength(scans);
  });

  it('removes a custom folder without toggling it', async () => {
    const u = user();
    await mount();
    h.open.mockResolvedValue('D:/Gigs');
    await u.click(screen.getByRole('button', { name: '+ Add folder…' }));
    await u.click(await screen.findByRole('button', { name: 'Remove Gigs' }));
    await waitFor(() => expect(backend.of('scan').at(-1)).toEqual({ folders: ['C:/rec', 'C:/rec temp'] }));
    expect(screen.queryByRole('checkbox', { name: /Gigs/ })).not.toBeInTheDocument();
  });

  it('Rescan looks again', async () => {
    const u = user();
    await mount();
    const scans = backend.of('scan').length;
    await u.click(screen.getByRole('button', { name: '↻ Rescan' }));
    await waitFor(() => expect(backend.of('scan').length).toBe(scans + 1));
  });
});
describe('previewing from the list', () => {
  it('the row button loads that file whole and plays it', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('button', { name: 'Preview Serato Recording 2.aif' }));
    await waitFor(() => expect(backend.of('audio_play')).toHaveLength(1));
    expect(backend.of('audio_load')[0]).toEqual({
      clips: [{ path: 'C:/rec/Serato Recording 2.aif', start_frame: 0, end_frame: null }],
      startSecs: 0,
      loudness: null,
    });
    // The transport reflects what is loaded.
    expect(await screen.findByRole('button', { name: 'Pause Serato Recording 2.aif' })).toBeInTheDocument();
  });

  it('pressing it again pauses instead of reloading', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('button', { name: 'Preview Serato Recording 2.aif' }));
    await u.click(await screen.findByRole('button', { name: 'Pause Serato Recording 2.aif' }));
    await waitFor(() => expect(backend.of('audio_pause')).toHaveLength(1));
    expect(backend.of('audio_load')).toHaveLength(1);
  });

  it('cannot preview an unreadable file', async () => {
    await mount({ files: [entry({ name: 'junk.aif', status: 'unreadable', detail: 'not an AIFF file' }), ...SAMPLE_FILES] });
    expect(screen.getByRole('button', { name: 'Preview junk.aif' })).toBeDisabled();
  });

  it('reports a file that will not play', async () => {
    const u = user();
    await mount();
    backend.fail('audio_load', 'cannot open file');
    await u.click(screen.getByRole('button', { name: 'Preview Serato Recording 1.aif' }));
    expect(await screen.findByText(/Cannot play Serato Recording 1.aif: cannot open file/)).toBeInTheDocument();
    expect(backend.of('audio_play')).toHaveLength(0);
  });

  it('Space toggles playback when something is loaded', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('button', { name: 'Preview Serato Recording 2.aif' }));
    await screen.findByRole('button', { name: 'Pause Serato Recording 2.aif' });
    await u.keyboard(' ');
    await waitFor(() => expect(backend.of('audio_pause')).toHaveLength(1));
  });
});

describe('theme', () => {
  it('cycles system, dark, light and remembers the choice', async () => {
    const u = user();
    await mount();
    const btn = () => screen.getByRole('button', { name: /^Theme:/ });
    expect(btn()).toHaveAccessibleName('Theme: Auto');
    await u.click(btn());
    expect(document.documentElement.dataset.theme).toBe('dark');
    await u.click(btn());
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem('theme.v1')).toBe('light');
    await u.click(btn());
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});

describe('Settings (export defaults and ffmpeg)', () => {
  const open = async () => {
    await user().click(screen.getByRole('button', { name: /Settings/ }));
    return screen.findByRole('dialog', { name: 'Settings' });
  };
  const noTool = { ffmpeg_path: null, ffmpeg_version: null, aac_encoders: [], best_aac: null, mp3: false, apple: null };

  it('the top bar has a plain Settings button, with no status when everything is fine', async () => {
    await mount();
    const btn = screen.getByRole('button', { name: /Settings/ });
    expect(btn).toHaveTextContent('Settings');
    expect(btn).not.toHaveTextContent('ffmpeg');
    expect(btn.querySelector('.led')).toBeNull();
  });

  it('flags the Settings button when ffmpeg is missing', async () => {
    await mount({ tool: noTool });
    const btn = screen.getByRole('button', { name: /Settings/ });
    expect(btn).toHaveTextContent('needs ffmpeg');
    expect(btn.title).toMatch(/ffmpeg, which was not found/);
  });

  it('sets the default export format and quality, shared with the editor', async () => {
    const u = user();
    await mount();
    const d = await open();
    await u.click(within(d).getByRole('radio', { name: /MP3/ }));
    await u.selectOptions(within(d).getByLabelText('Bitrate mode'), 'vbr');
    await u.selectOptions(within(d).getByLabelText('Quality'), '0');
    await u.clear(within(d).getByLabelText('Fade in (seconds)'));
    await u.type(within(d).getByLabelText('Fade in (seconds)'), '2');
    const saved = JSON.parse(localStorage.getItem('exportSettings.v1')!);
    expect(saved.format).toMatchObject({ kind: 'mp3', mp3Mode: 'vbr', mp3Value: 0 });
    expect(saved.fadeIn).toBe(2);
    // The editor starts from it.
    await u.click(screen.getByRole('button', { name: 'Done' }));
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Edit & export…' }));
    const editor = await screen.findByRole('dialog', { name: 'Editor' });
    expect(within(editor).getByRole('radio', { name: /MP3/ })).toBeChecked();
    expect(within(editor).getByLabelText('Quality')).toHaveValue('0');
    expect(within(editor).getByLabelText('Fade in (seconds)')).toHaveValue(2);
  });

  it('opening and closing Settings without a change leaves what was remembered alone', async () => {
    const u = user();
    localStorage.setItem('exportSettings.v1', JSON.stringify({ format: { kind: 'aac' }, nameTemplate: 'x' }));
    await mount();
    await open();
    await u.click(screen.getByRole('button', { name: 'Done' }));
    expect(JSON.parse(localStorage.getItem('exportSettings.v1')!)).toEqual({ format: { kind: 'aac' }, nameTemplate: 'x' });
  });

  it('lists the AAC encoders as information, best first, and says where to choose', async () => {
    await mount();
    const d = await open();
    const items = [...d.querySelectorAll('.encoders li')].map((li) => li.textContent);
    expect(items[0]).toMatch(/^Apple AAC \(CoreAudio\)\s*used unless you pick another/);
    expect(items).toHaveLength(3);
    expect(d).toHaveTextContent('This is information only; choose which one to use under AAC');
  });

  it('shows the version in the status bar and in an About section with the notices', async () => {
    const u = user();
    await mount();
    const footer = document.querySelector('footer.status') as HTMLElement;
    expect(await within(footer).findByRole('button', { name: 'v0.1.0' })).toBeInTheDocument();
    await u.click(within(footer).getByRole('button', { name: 'v0.1.0' }));
    const d = await screen.findByRole('dialog', { name: 'Settings' });
    const about = within(d).getByRole('region', { name: 'About' });
    await waitFor(() => expect(within(about).getByTestId('app-version')).toHaveTextContent('v0.1.0'));
    expect(about).toHaveTextContent('GNU GPL v3 or later');
    expect(about).toHaveTextContent('Not affiliated with or endorsed by Serato');
    expect(about).toHaveTextContent('ASIO is a registered trademark of Steinberg');
  });

  it('opens the GitHub issues page from the status bar and from About', async () => {
    const u = user();
    await mount();
    const issues = 'https://github.com/djryanj/recording-fixer/issues';
    await u.click(screen.getByRole('button', { name: 'Report an issue' }));
    await waitFor(() => expect(backend.of('plugin:opener|open_url').at(-1)).toMatchObject({ url: issues }));
    const d = await open();
    await u.click(within(d).getByRole('button', { name: 'Report an issue on GitHub' }));
    await waitFor(() => expect(backend.of('plugin:opener|open_url')).toHaveLength(2));
    await u.click(within(d).getByRole('button', { name: 'Source code' }));
    await waitFor(() => expect(backend.of('plugin:opener|open_url').at(-1)).toMatchObject({ url: 'https://github.com/djryanj/recording-fixer' }));
  });

  it('opens the audio output window from Settings', async () => {
    const u = user();
    await mount();
    const d = await open();
    await u.click(within(d).getByRole('button', { name: /Audio output device and channels/ }));
    expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull();
    expect(await screen.findByRole('dialog', { name: /audio/i })).toBeInTheDocument();
  });

  it('explains what ffmpeg is for, and what does not need it', async () => {
    await mount();
    const d = await open();
    expect(within(d).getByText(/free audio converter/)).toBeInTheDocument();
    expect(d).toHaveTextContent(/even when Apple's AAC encoder does the encoding/);
    expect(d).toHaveTextContent(/do not need ffmpeg for repairing files, playing them, editing, or saving as AIFF/);
  });

  it('shows where ffmpeg was found, its version, and the encoders it offers', async () => {
    await mount();
    const d = await open();
    expect(within(d).getByText('C:/ffmpeg.exe')).toBeInTheDocument();
    expect(d).toHaveTextContent('9.0.2-full_build');
    expect(d).toHaveTextContent('Found automatically');
    expect(d).toHaveTextContent('Apple AAC (CoreAudio)');
    expect(d).toHaveTextContent('Windows Media Foundation AAC');
    expect(d).toHaveTextContent('Apple AAC (CoreAudioToolbox)');
  });

  it('when ffmpeg is missing it says how to get it and offers to choose one', async () => {
    await mount({ tool: noTool });
    const d = await open();
    expect(d).toHaveTextContent('Not found');
    expect(d).toHaveTextContent('winget install Gyan.FFmpeg');
    expect(within(d).getByRole('button', { name: /I have ffmpeg: choose it/ })).toBeInTheDocument();
  });

  it('choosing an ffmpeg re-checks the tools and remembers it', async () => {
    const u = user();
    await mount();
    const d = await open();
    h.open.mockResolvedValue('D:/tools/ffmpeg.exe');
    await u.click(within(d).getByRole('button', { name: 'Choose a different ffmpeg…' }));
    await waitFor(() => expect(backend.of('tool_status').at(-1)).toEqual({ userPath: 'D:/tools/ffmpeg.exe' }));
    expect(JSON.parse(localStorage.getItem('ffmpegPath')!)).toBe('D:/tools/ffmpeg.exe');
    expect(h.open.mock.calls.at(-1)![0]).toMatchObject({ title: expect.stringMatching(/ffmpeg\.exe.*MP3 and AAC/) });
    expect(await within(d).findByText('This is the program you chose.')).toBeInTheDocument();
  });

  it('can go back to the automatically found one', async () => {
    const u = user();
    localStorage.setItem('ffmpegPath', JSON.stringify('D:/tools/ffmpeg.exe'));
    await mount();
    const d = await open();
    await u.click(within(d).getByRole('button', { name: 'Use the automatic one' }));
    await waitFor(() => expect(backend.of('tool_status').at(-1)).toEqual({ userPath: null }));
    expect(JSON.parse(localStorage.getItem('ffmpegPath')!)).toBeNull();
    expect(within(d).queryByRole('button', { name: 'Use the automatic one' })).not.toBeInTheDocument();
  });

  it('Done and Escape close it', async () => {
    const u = user();
    await mount();
    await open();
    await u.click(screen.getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('dialog', { name: 'Settings' })).not.toBeInTheDocument();
    await open();
    await u.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Settings' })).not.toBeInTheDocument();
  });

  it('the editor opens Settings from its "Locate ffmpeg" link', async () => {
    const u = user();
    await mount({ tool: noTool });
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Edit & export…' }));
    const editor = await screen.findByRole('dialog', { name: 'Editor' });
    await u.click(within(editor).getByRole('button', { name: 'Locate ffmpeg…' }));
    expect(await screen.findByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
  });
});
describe('updates and troubleshooting', () => {
  const newer = { current: '0.1.0', latest: '0.2.0', url: 'https://github.com/djryanj/recording-fixer/releases/tag/v0.2.0', newer: true };
  const open = async () => {
    await user().click(screen.getByRole('button', { name: /Settings/ }));
    return screen.findByRole('dialog', { name: 'Settings' });
  };

  it('looks for a newer version once at start and says nothing when up to date', async () => {
    await mount();
    await waitFor(() => expect(backend.of('check_for_update')).toHaveLength(1));
    expect(screen.queryByText(/Update available/)).toBeNull();
  });

  it('offers a newer version in the status bar and opens its release page', async () => {
    const u = user();
    await mount({ update: newer });
    const btn = await screen.findByRole('button', { name: /Update available: v0\.2\.0/ });
    await u.click(btn);
    await waitFor(() => expect(backend.of('plugin:opener|open_url').at(-1)).toMatchObject({ url: newer.url }));
  });

  it('does not ask at all when switched off, or when it already asked today', async () => {
    localStorage.setItem('updateCheck.v1', JSON.stringify({ enabled: false }));
    await mount();
    expect(backend.of('check_for_update')).toHaveLength(0);
    document.body.innerHTML = '';
    localStorage.setItem('updateCheck.v1', JSON.stringify({ lastChecked: Date.now() - 60_000 }));
    await mount();
    expect(backend.of('check_for_update')).toHaveLength(0);
  });

  it('a failed check (offline) is silent', async () => {
    await mount({ failures: { check_for_update: 'curl was not found' } });
    await waitFor(() => expect(backend.of('check_for_update')).toHaveLength(1));
    expect(screen.queryByText(/curl/)).toBeNull();
    expect(screen.queryByText(/Update available/)).toBeNull();
  });

  it('Settings can check on demand, and remembers the automatic-check switch', async () => {
    const u = user();
    await mount({ update: newer });
    const d = await open();
    const sec = within(d).getByRole('region', { name: 'Updates and privacy' });
    expect(sec).toHaveTextContent('collects nothing');
    await u.click(within(sec).getByRole('button', { name: 'Check now' }));
    expect(await within(sec).findByRole('status')).toHaveTextContent('Version 0.2.0 is available');
    await u.click(within(sec).getByRole('checkbox', { name: /Look for a newer version/ }));
    expect(JSON.parse(localStorage.getItem('updateCheck.v1')!).enabled).toBe(false);
    await u.click(within(sec).getByRole('button', { name: 'Privacy policy' }));
    await waitFor(() => expect(backend.of('plugin:opener|open_url').at(-1)).toMatchObject({ url: 'https://github.com/djryanj/recording-fixer/blob/main/PRIVACY.md' }));
  });

  it('Settings says plainly when the check could not be made', async () => {
    const u = user();
    await mount({ failures: { check_for_update: 'offline' } });
    const sec = within(await open()).getByRole('region', { name: 'Updates and privacy' });
    await u.click(within(sec).getByRole('button', { name: 'Check now' }));
    expect(await within(sec).findByRole('status')).toHaveTextContent('Could not check: offline');
  });

  it('shows the log file and copies the log for a bug report', async () => {
    const u = user();
    await mount();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText'); // user-event provides the clipboard
    const sec = within(await open()).getByRole('region', { name: 'Troubleshooting' });
    await u.click(within(sec).getByRole('button', { name: 'Show log file' }));
    await waitFor(() => expect(backend.of('reveal_in_folder').at(-1)).toMatchObject({ path: 'C:/logs/recording-fixer.log' }));
    await u.click(within(sec).getByRole('button', { name: 'Copy log to clipboard' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(expect.stringContaining('starting')));
    expect(sec).toHaveTextContent('Copied');
  });
});

describe('loudness maximizer', () => {
  async function openEditor(u: ReturnType<typeof user>) {
    await u.click(within(rowFor('Serato Recording 2.aif')).getByRole('checkbox'));
    await u.click(screen.getByRole('button', { name: 'Edit & export…' }));
    return screen.findByRole('dialog', { name: 'Editor' });
  }
  /** The loudness controls live on their own tab of the editor. */
  async function loudnessTab(u: ReturnType<typeof user>, editor: HTMLElement) {
    await u.click(within(editor).getByRole('tab', { name: /Loudness/ }));
    return within(editor).getByRole('group', { name: 'Loudness' });
  }
  const sentJob = () => (backend.of('export_audio')[0] as { job: Record<string, unknown> }).job;

  it('leaves the audio alone unless asked', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    // Not among the format and quality choices.
    expect(within(editor).queryByRole('group', { name: 'Loudness' })).toBeNull();
    const group = await loudnessTab(u, editor);
    expect(within(group).getByRole('radio', { name: 'Leave as recorded' })).toBeChecked();
    expect(within(group).queryByLabelText('Target loudness')).toBeNull();
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(1));
    expect(sentJob().loudness).toBeNull();
  });

  it('matches a loudness with the chosen evening out and limit, and says what it did', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    await u.click(within(group).getByRole('button', { name: 'Quieter −16' }));
    await u.selectOptions(within(group).getByLabelText('Evening out'), 'strong');
    const limit = within(group).getByLabelText('Peak limit');
    await u.clear(limit);
    await u.type(limit, '-1.5');
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(1));
    expect(sentJob().loudness).toEqual({ mode: 'lufs', lufs: -16, ceiling_db: -1.5, leveling: 'strong' });
    expect(await screen.findByText(/Exported Serato Recording 2.out. Loudness -14.0 LUFS/)).toBeInTheDocument();
  });

  it('peak mode asks only for the limit and sends no evening out', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Raise to a peak limit' }));
    expect(within(group).queryByLabelText('Evening out')).toBeNull();
    const limit = within(group).getByLabelText('Peak limit');
    await u.clear(limit);
    await u.type(limit, '-0.1');
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(1));
    expect(sentJob().loudness).toMatchObject({ mode: 'peak', ceiling_db: -0.1, leveling: 'off' });
  });

  it('is remembered between exports and shared with Settings', async () => {
    const u = user();
    await mount();
    await u.click(screen.getByRole('button', { name: /Settings/ }));
    const d = await screen.findByRole('dialog', { name: 'Settings' });
    await u.click(within(within(d).getByRole('group', { name: 'Loudness' })).getByRole('radio', { name: 'Match a loudness' }));
    expect(JSON.parse(localStorage.getItem('exportSettings.v1')!).loudness).toMatchObject({ mode: 'lufs', lufs: -14 });
    await u.click(screen.getByRole('button', { name: 'Done' }));
    const editor = await openEditor(u);
    expect(within(await loudnessTab(u, editor)).getByRole('radio', { name: 'Match a loudness' })).toBeChecked();
  });

  const loadsWith = () => backend.of('audio_load').map((a) => a.loudness);

  it('plays the editor preview through the processing when it is on, and says what it is doing', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    await waitFor(() => expect(loadsWith().length).toBeGreaterThan(0));
    expect(loadsWith().at(-1)).toBeNull(); // off: the original

    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    await waitFor(() => expect(loadsWith().at(-1)).toEqual({ mode: 'lufs', lufs: -14, ceiling_db: -1, leveling: 'medium' }));
    const note = await within(group).findByText(/Playing with it on/);
    expect(note).toHaveTextContent('−19.6 LUFS');
    expect(note).toHaveTextContent('+5.6 dB');
    expect(note).toHaveTextContent('between +3.4 dB and +9.0 dB');
  });

  it('changing the target while listening reloads the preview with it', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    await u.click(within(group).getByRole('button', { name: 'Louder −11' }));
    await waitFor(() => expect(loadsWith().at(-1)).toMatchObject({ lufs: -11 }));
  });

  it('can be switched off in the preview to compare with the original, without changing the export', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    await waitFor(() => expect(loadsWith().at(-1)).not.toBeNull());
    await u.click(within(group).getByRole('checkbox', { name: /Hear this when playing/ }));
    await waitFor(() => expect(loadsWith().at(-1)).toBeNull());
    // The export still carries the setting.
    await u.click(within(editor).getByRole('button', { name: 'Export' }));
    await waitFor(() => expect(backend.of('export_audio')).toHaveLength(1));
    expect(sentJob().loudness).toMatchObject({ mode: 'lufs' });
  });

  it('shows how far the first measurement has got', async () => {
    const u = user();
    await mount();
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    // Hold the reload open, as the first measurement of a long recording does.
    const real = h.invoke.getMockImplementation()!;
    let finish!: () => void;
    h.invoke.mockImplementation((cmd: string, args: Record<string, unknown>) =>
      cmd === 'audio_load' && args.loudness ? new Promise((r) => (finish = () => r(real(cmd, args)))) : real(cmd, args),
    );
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    expect(await within(group).findByRole('progressbar', { name: 'Measuring loudness' })).toBeInTheDocument();
    finish();
    await within(group).findByText(/Playing with it on/);
    expect(within(group).queryByRole('progressbar')).toBeNull();
  });

  it('says so when the preview cannot use the setting', async () => {
    const u = user();
    await mount({ failures: { audio_load: 'the clips do not share one audio format' } });
    const editor = await openEditor(u);
    const group = await loudnessTab(u, editor);
    await u.click(within(group).getByRole('radio', { name: 'Match a loudness' }));
    expect(await within(group).findByText(/could not use this: the clips do not share one audio format/)).toBeInTheDocument();
  });
});
