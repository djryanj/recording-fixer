<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, type FileEntry, type FolderInfo, type Progress, type ToolStatus } from './lib/api';
  import { audio } from './lib/audio.svelte';
  import AudioSettings from './lib/AudioSettings.svelte';
  import Editor, { type ExportRequest } from './lib/Editor.svelte';
  import { describeFormat, formatDuration, formatSize } from './lib/format';
  import {
    canExport,
    canRepair,
    countByStatus,
    filterEntries,
    sortEntries,
    type SortDir,
    type SortKey,
    type StatusFilter,
  } from './lib/library';
  import { applyTheme, loadTheme, nextTheme, themeLabel, type Theme } from './lib/theme';
  import { appVersion, ISSUES_URL, openLink } from './lib/about';
  import SettingsDialog from './lib/SettingsDialog.svelte';
  import Transport from './lib/Transport.svelte';

  interface FolderRow extends FolderInfo {
    enabled: boolean;
    custom: boolean;
  }

  const load = <T,>(key: string, fallback: T): T => {
    try {
      const v = localStorage.getItem(key);
      return v ? (JSON.parse(v) as T) : fallback;
    } catch {
      return fallback;
    }
  };
  const save = (key: string, value: unknown) => {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* storage unavailable: settings just won't persist */
    }
  };

  let folders = $state<FolderRow[]>([]);
  let entries = $state<FileEntry[]>([]);
  let scanning = $state(false);
  let selected = $state(new Set<string>());
  let statusFilter = $state<StatusFilter>('all');
  let query = $state('');
  let sortKey = $state<SortKey>('modified');
  let sortDir = $state<SortDir>('desc');
  let tool = $state<ToolStatus | null>(null);
  let userFfmpeg = $state<string | null>(load<string | null>('ffmpegPath', null));
  let progress = $state<Progress | null>(null);
  let busy = $state(false);
  let messages = $state<{ id: number; kind: 'ok' | 'error'; text: string; reveal?: string }[]>([]);
  let showRepair = $state(false);
  let repairInPlace = $state(false);
  let showEditor = $state(false);
  let showAudio = $state(false);
  let showTools = $state(false);
  let appVer = $state('');
  onMount(() => void appVersion().then((v) => (appVer = v)));
  let nextId = 0;
  let theme = $state<Theme>(loadTheme());
  function cycleTheme() {
    theme = nextTheme(theme);
    applyTheme(theme);
  }

  const visible = $derived(sortEntries(filterEntries(entries, { status: statusFilter, query }), sortKey, sortDir));
  const counts = $derived(countByStatus(entries));
  const selectedEntries = $derived(entries.filter((e) => selected.has(e.path)));
  const allVisibleSelected = $derived(visible.length > 0 && visible.every((e) => selected.has(e.path)));
  const modalOpen = $derived(showRepair || showEditor || showAudio || showTools);

  function say(kind: 'ok' | 'error', text: string, reveal?: string) {
    messages = [...messages, { id: nextId++, kind, text, reveal }].slice(-4);
  }

  async function refresh() {
    const paths = folders.filter((f) => f.enabled).map((f) => f.path);
    scanning = true;
    try {
      entries = await api.scan(paths);
      const have = new Set(entries.map((e) => e.path));
      selected = new Set([...selected].filter((p) => have.has(p)));
    } catch (e) {
      say('error', `Scan failed: ${e}`);
    } finally {
      scanning = false;
    }
  }

  async function refreshTools() {
    try {
      tool = await api.toolStatus(userFfmpeg);
    } catch (e) {
      say('error', `Could not check ffmpeg: ${e}`);
    }
  }

  function persistFolders() {
    save(
      'folders',
      folders.map((f) => ({ path: f.path, enabled: f.enabled, custom: f.custom, label: f.label, kind: f.kind })),
    );
  }

  async function init() {
    const defaults = await api.defaultFolders();
    const saved = load<Pick<FolderRow, 'path' | 'enabled' | 'custom' | 'label' | 'kind'>[]>('folders', []);
    const enabledOf = new Map(saved.map((s) => [s.path, s.enabled]));
    const rows: FolderRow[] = defaults.map((d) => ({ ...d, enabled: enabledOf.get(d.path) ?? d.exists, custom: false }));
    for (const s of saved.filter((s) => s.custom)) {
      rows.push({ path: s.path, label: s.label, kind: s.kind, exists: true, enabled: s.enabled, custom: true });
    }
    folders = rows;
    await Promise.all([refresh(), refreshTools()]);
  }

  async function addFolder() {
    const dir = await open({ directory: true, multiple: false, title: 'Choose a folder with recordings' });
    if (typeof dir !== 'string' || folders.some((f) => f.path === dir)) return;
    folders = [...folders, { path: dir, label: dir.split(/[\\/]/).filter(Boolean).pop() ?? dir, kind: 'other', exists: true, enabled: true, custom: true }];
    persistFolders();
    await refresh();
  }

  function removeFolder(path: string) {
    folders = folders.filter((f) => f.path !== path);
    persistFolders();
    refresh();
  }

  function toggleFolder(f: FolderRow) {
    f.enabled = !f.enabled;
    persistFolders();
    refresh();
  }

  const folderWhat = (f: FolderRow) =>
    f.kind === 'recording'
      ? 'Serato saves recordings here when you stop them normally.'
      : f.kind === 'temp'
        ? 'Serato keeps recordings here while it is recording, and leaves them here if it is closed or crashes before finishing. These are the ones that usually need repair.'
        : 'A folder you added.';

  const folderTip = (f: FolderRow) => `${f.path}\n\n${folderWhat(f)}${f.exists ? '' : '\n\nThis folder was not found on this computer.'}`;

  // The explanation panel under the folder row: open on first run, then only when an ⓘ is clicked.
  let folderHelp = $state<'intro' | string | null>(load<boolean>('folderHelpSeen', false) ? null : 'intro');
  const helpFolder = $derived(folders.find((f) => f.path === folderHelp));
  function closeHelp() {
    folderHelp = null;
    save('folderHelpSeen', true);
  }
  const toggleHelp = (f: FolderRow) => (folderHelp = folderHelp === f.path ? null : f.path);

  /** How many recordings were found in a folder. */
  const countIn = (f: FolderRow) => entries.filter((e) => e.folder === f.path).length;

  async function resetFfmpeg() {
    userFfmpeg = null;
    save('ffmpegPath', null);
    await refreshTools();
  }

  async function locateFfmpeg() {
    const picked = await open({ multiple: false, title: 'Choose the ffmpeg program (ffmpeg.exe) used for MP3 and AAC export' });
    if (typeof picked !== 'string') return;
    userFfmpeg = picked;
    save('ffmpegPath', picked);
    await refreshTools();
  }

  function sortBy(key: SortKey) {
    if (sortKey === key) sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    else {
      sortKey = key;
      sortDir = key === 'name' ? 'asc' : 'desc';
    }
  }

  function toggle(path: string) {
    const next = new Set(selected);
    if (!next.delete(path)) next.add(path);
    selected = next;
  }

  function toggleAll() {
    selected = allVisibleSelected ? new Set() : new Set(visible.map((e) => e.path));
  }

  async function runJob<T>(work: () => Promise<T>): Promise<T | undefined> {
    busy = true;
    progress = null;
    const unlisten = await api.onProgress((p) => (progress = p));
    try {
      return await work();
    } finally {
      unlisten();
      busy = false;
      progress = null;
    }
  }

  async function doRepair() {
    showRepair = false;
    const paths = selectedEntries.filter((e) => e.status === 'broken').map((e) => e.path);
    const results = await runJob(() => api.repair(paths, repairInPlace));
    if (!results) return;
    for (const r of results) {
      const name = r.path.split(/[\\/]/).pop();
      if (r.ok) say('ok', repairInPlace ? `Fixed ${name}` : `Saved repaired copy of ${name}`, r.output ?? undefined);
      else say('error', `${name}: ${r.error}`);
    }
    await refresh();
  }

  async function doExport(req: ExportRequest) {
    showEditor = false;
    for (const job of req.jobs) {
      try {
        const out = await runJob(() =>
          api.exportAudio({
            clips: job.clips,
            target: req.target,
            output_dir: req.outputDir,
            file_stem: job.stem,
            metadata: job.metadata,
            fade_in_secs: job.fadeInSecs,
            fade_out_secs: job.fadeOutSecs,
            ffmpeg_path: userFfmpeg,
          }),
        );
        if (out) say('ok', `Exported ${out.split(/[\\/]/).pop()}`, out);
      } catch (e) {
        say('error', String(e));
        if (String(e) === 'cancelled') break;
      }
    }
    await refresh();
  }

  async function preview(e: FileEntry) {
    if (audio.sourcePath === e.path && audio.state.loaded) {
      await audio.toggle();
      return;
    }
    try {
      await audio.preview(e.path, e.name);
    } catch (err) {
      say('error', `Cannot play ${e.name}: ${err}`);
    }
  }

  const isPlayingRow = (e: FileEntry) => audio.sourcePath === e.path && audio.state.playing;

  function onkeydown(ev: KeyboardEvent) {
    if (modalOpen || ev.key !== ' ') return;
    const tag = (ev.target as HTMLElement).tagName;
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'BUTTON' || tag === 'TEXTAREA') return;
    ev.preventDefault();
    void audio.toggle();
  }

  onMount(() => {
    applyTheme(theme);
    void init();
    audio.start();
    void audio.restore();
    const onFocus = () => !busy && refresh();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  });
  onDestroy(() => audio.stop());

  const statusLabel: Record<string, string> = {
    ok: 'OK',
    broken: 'Needs repair',
    inuse: 'In use',
    unreadable: 'Unreadable',
    unsupported: 'Unsupported',
  };
  const ledClass: Record<string, string> = { ok: 'ok', broken: 'bad', inuse: 'busy', unreadable: '', unsupported: '' };
  const arrow = (k: SortKey) => (sortKey === k ? (sortDir === 'asc' ? ' ▲' : ' ▼') : '');
  const fmtDate = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
</script>

<svelte:window {onkeydown} />

<main>
  <header class="top">
    <div class="brand" aria-label="Recording Fixer for Serato DJ">
      <svg viewBox="0 0 28 20" width="28" height="20" aria-hidden="true">
        <g fill="currentColor">
          <rect x="1" y="8" width="2.4" height="4" /><rect x="5" y="4" width="2.4" height="12" /><rect x="9" y="1" width="2.4" height="18" />
          <rect x="13" y="6" width="2.4" height="8" /><rect x="17" y="2" width="2.4" height="16" /><rect x="21" y="6" width="2.4" height="8" /><rect x="25" y="9" width="2.4" height="2" />
        </g>
      </svg>
      <span class="word"><b>Recording</b> Fixer</span>
      <span class="for">for Serato DJ</span>
    </div>

    <div class="spacer"></div>

    <div class="tool">
      <button
        class="ghost settings"
        onclick={() => (showTools = true)}
        title={tool?.ffmpeg_path
          ? 'Default export format and quality, ffmpeg, and the audio output'
          : 'Settings. MP3 and AAC export need ffmpeg, which was not found: open Settings to fix that.'}
      >
        {#if !tool?.ffmpeg_path}<i class="led bad"></i>{/if}
        <span>⚙ Settings</span>
        {#if !tool?.ffmpeg_path}<span class="state">needs ffmpeg</span>{/if}
      </button>
      <button class="ghost theme" onclick={cycleTheme} title="Colour theme: Auto follows your system" aria-label={`Theme: ${themeLabel[theme]}`}>◐ {themeLabel[theme]}</button>
    </div>
  </header>

  <div class="srcwrap">
  <section class="sources" aria-label="Folders">
    <span class="label lead" title="Serato keeps its recordings in these folders. Tick the ones you want listed below.">Show recordings from</span>
    {#each folders as f (f.path)}
      <label class="src" class:off={!f.enabled} class:missing={!f.exists} title={folderTip(f)}>
        <input type="checkbox" checked={f.enabled} onchange={() => toggleFolder(f)} />
        <span class="nm">{f.label}</span>
        {#if f.kind !== 'other'}
          <button class="ghost info" aria-label={`What is ${f.label}?`} aria-expanded={folderHelp === f.path} onclick={(ev) => { ev.preventDefault(); toggleHelp(f); }}>ⓘ</button>
        {/if}
        {#if !f.exists}<span class="nf">not found</span>{:else}<span class="cnt mono">{countIn(f)}</span>{/if}
        {#if f.custom}
          <button class="ghost x" title={`Stop looking in ${f.label}`} aria-label={`Remove ${f.label}`} onclick={(ev) => { ev.preventDefault(); removeFolder(f.path); }}>×</button>
        {/if}
      </label>
    {/each}
    <button class="ghost" onclick={addFolder} title="Also look for recordings in another folder, for example on an external drive">+ Add folder…</button>
    <span class="grow"></span>
    <button class="ghost" onclick={refresh} disabled={scanning} title="Look again for new or changed recordings (this also happens whenever you come back to this window)">{scanning ? 'Scanning…' : '↻ Rescan'}</button>
  </section>

  {#if folderHelp}
    <aside class="folderhelp" aria-label="About these folders">
      {#if folderHelp === 'intro'}
        <p>
          <b>Recording</b>: recordings Serato saved when you stopped them normally.
          <b>Recording temp</b>: recordings Serato was still writing; if it closed or crashed, they stay here and usually need repair.
          Tick the folders you want listed. Click ⓘ any time to see this again.
        </p>
      {:else if helpFolder}
        <p><b>{helpFolder.label}</b>: {folderWhat(helpFolder)} <span class="muted mono">{helpFolder.path}</span></p>
      {/if}
      <button class="ghost" onclick={closeHelp}>{folderHelp === 'intro' ? 'Got it' : 'Close'}</button>
    </aside>
  {/if}
  </div>

  <section class="bar">
    <div class="tabs" role="tablist">
      {#each [['all', 'All'], ['broken', 'Needs repair'], ['ok', 'OK'], ['inuse', 'In use']] as [key, label]}
        <button role="tab" aria-selected={statusFilter === key} class:active={statusFilter === key} onclick={() => (statusFilter = key as StatusFilter)}>
          {label}<span class="count mono">{counts[key as StatusFilter]}</span>
        </button>
      {/each}
    </div>
    <input type="search" class="search" placeholder="Search name or date" bind:value={query} aria-label="Search" />
    <div class="spacer"></div>
    <span class="sel mono">{selected.size ? `${selected.size} selected` : ''}</span>
    <button disabled={busy || !canRepair(selectedEntries)} onclick={() => (showRepair = true)}>Repair…</button>
    <button class="primary" disabled={busy || !canExport(selectedEntries)} onclick={() => (showEditor = true)}>
      {selected.size > 1 ? 'Combine & export…' : 'Edit & export…'}
    </button>
  </section>

  <section class="list">
    <table>
      <thead>
        <tr>
          <th class="pv"></th>
          <th class="check"><input type="checkbox" checked={allVisibleSelected} onchange={toggleAll} aria-label="Select all" /></th>
          <th><button class="th" onclick={() => sortBy('name')}>Name{arrow('name')}</button></th>
          <th><button class="th" onclick={() => sortBy('status')}>Status{arrow('status')}</button></th>
          <th class="r"><button class="th" onclick={() => sortBy('duration')}>Length{arrow('duration')}</button></th>
          <th>Format</th>
          <th><button class="th" onclick={() => sortBy('recorded')}>Recorded{arrow('recorded')}</button></th>
          <th class="r"><button class="th" onclick={() => sortBy('size')}>Size{arrow('size')}</button></th>
          <th>Folder</th>
        </tr>
      </thead>
      <tbody>
        {#each visible as e (e.path)}
          <tr class:selected={selected.has(e.path)} class:playing={isPlayingRow(e)} onclick={() => toggle(e.path)} data-testid="row">
            <td class="pv">
              <button
                class="pbtn"
                class:on={isPlayingRow(e)}
                disabled={e.status === 'unreadable' || e.status === 'unsupported'}
                aria-label={isPlayingRow(e) ? `Pause ${e.name}` : `Preview ${e.name}`}
                title="Preview this recording"
                onclick={(ev) => {
                  ev.stopPropagation();
                  void preview(e);
                }}
              >
                {#if isPlayingRow(e)}
                  <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><rect x="2" y="1" width="3" height="10" /><rect x="7" y="1" width="3" height="10" /></svg>
                {:else}
                  <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M2.5 1l8.5 5-8.5 5z" /></svg>
                {/if}
              </button>
            </td>
            <td class="check"><input type="checkbox" checked={selected.has(e.path)} onclick={(ev) => ev.stopPropagation()} onchange={() => toggle(e.path)} aria-label={`Select ${e.name}`} /></td>
            <td class="name" title={e.path}>{e.name}</td>
            <td class="st">
              <span class="stat" title={e.detail ?? ''}><i class={`led ${ledClass[e.status]}`} class:pulse={e.status === 'broken'}></i>{statusLabel[e.status]}</span>
              {#if e.detail && e.status !== 'ok'}<span class="detail muted" title={e.detail}>{e.detail}</span>{/if}
            </td>
            <td class="r mono">{formatDuration(e.duration_secs)}</td>
            <td class="muted fmt">{describeFormat(e)}</td>
            <td class="muted mono rec">{e.recorded ?? fmtDate(e.modified_ms)}</td>
            <td class="r mono">{formatSize(e.size)}</td>
            <td class="muted">{e.folder_kind === 'temp' ? 'Recording temp' : e.folder_kind === 'recording' ? 'Recording' : e.folder.split(/[\\/]/).pop()}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if !visible.length}
      <p class="empty muted">
        {#if scanning}Scanning…{:else if !entries.length}No recordings found. Switch a folder on above, or add one.{:else}Nothing matches the filter.{/if}
      </p>
    {/if}
  </section>

  <Transport onsettings={() => (showAudio = true)} />

  <footer class="status">
    {#if busy}
      <div class="progress">
        <span class="label">{progress?.label ?? 'Working…'}</span>
        <progress max="100" value={progress && progress.total ? (progress.done / progress.total) * 100 : undefined}></progress>
        <button class="ghost" onclick={api.cancelJob}>Cancel</button>
      </div>
    {:else}
      <span class="muted idle">{entries.length} recording{entries.length === 1 ? '' : 's'}{counts.broken ? ` · ${counts.broken} need repair` : ''}</span>
    {/if}
    <span class="grow"></span>
    <button class="ghost ver" onclick={() => (showTools = true)} title="About, and how to report a problem">{appVer ? `v${appVer}` : 'About'}</button>
    <button class="ghost ver" onclick={() => openLink(ISSUES_URL)} title="Opens GitHub in your browser">Report an issue</button>
  </footer>

  {#if messages.length}
    <div class="toasts" aria-live="polite">
      {#each messages as m (m.id)}
        <div class={`toast ${m.kind}`}>
          <i class={`led ${m.kind === 'ok' ? 'ok' : 'bad'}`}></i>
          <span>{m.text}</span>
          {#if m.reveal}<button class="ghost" onclick={() => api.reveal(m.reveal!)}>Show</button>{/if}
          <button class="ghost x" title="Dismiss" aria-label="Dismiss" onclick={() => (messages = messages.filter((x) => x.id !== m.id))}>×</button>
        </div>
      {/each}
    </div>
  {/if}

  {#if showRepair}
    <div class="backdrop" role="presentation">
      <div class="dialog panel" role="dialog" aria-modal="true" aria-label="Repair recordings">
        <h2>Repair {selectedEntries.filter((e) => e.status === 'broken').length} recording(s)</h2>
        <p class="muted">Serato wrote these files but never recorded their length. The audio is intact; only the header needs fixing.</p>
        <label class="radio"><input type="radio" bind:group={repairInPlace} value={false} /> <span><b>Save a repaired copy</b> next to the original (recommended). The original is left untouched.</span></label>
        <label class="radio"><input type="radio" bind:group={repairInPlace} value={true} /> <span><b>Fix in place.</b> Instant, no extra disk space. The original header is saved as <code>.hdrbak</code>.</span></label>
        <div class="actions">
          <button onclick={() => (showRepair = false)}>Cancel</button>
          <button class="primary" onclick={doRepair}>Repair</button>
        </div>
      </div>
    </div>
  {/if}

  {#if showEditor}
    <Editor entries={selectedEntries} {tool} onclose={() => (showEditor = false)} onstart={doExport} onlocate={() => (showTools = true)} onsettings={() => (showAudio = true)} />
  {/if}

  {#if showAudio}
    <AudioSettings onclose={() => (showAudio = false)} />
  {/if}

  {#if showTools}
    <SettingsDialog {tool} userPath={userFfmpeg} onchoose={locateFfmpeg} onreset={resetFfmpeg} onaudio={() => { showTools = false; showAudio = true; }} onclose={() => (showTools = false)} />
  {/if}
</main>

<style>
  main {
    display: grid;
    grid-template-rows: auto auto auto minmax(0, 1fr) auto auto;
    height: 100%;
    position: relative;
  }

  /* ---- top strip ---- */
  .top {
    display: flex;
    align-items: center;
    gap: 22px;
    padding: 0 16px;
    height: 46px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--amber);
  }
  .brand svg {
    filter: drop-shadow(0 0 6px var(--amber-glow));
  }
  .word {
    font-family: var(--font-label);
    font-weight: 600;
    font-size: 14px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--text);
    white-space: nowrap;
  }
  .word b {
    color: var(--amber);
    font-weight: 700;
  }
  .for {
    font-family: var(--font-label);
    font-weight: 500;
    font-size: 12px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--muted);
    white-space: nowrap;
    margin-left: -2px;
  }
  .top .spacer {
    flex: 1;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .settings {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }
  .settings .state {
    font-size: 12.5px;
    color: var(--text);
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
  }

  /* ---- where recordings are looked for ---- */
  .sources {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 8px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  .lead {
    margin-right: 4px;
  }
  .src {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px 4px 10px;
    border: 1px solid var(--line-strong);
    border-radius: var(--radius);
    background: var(--inset);
    cursor: pointer;
  }
  .src:hover {
    border-color: var(--amber);
  }
  .src.off {
    opacity: 0.55;
  }
  .src .nm {
    font-weight: 600;
  }
  .src .info {
    padding: 0 3px;
    min-height: 0;
    border: 0;
    font-size: 13px;
    color: var(--muted);
  }
  .src .info:hover,
  .src .info[aria-expanded='true'] {
    color: var(--amber);
  }
  .folderhelp {
    display: flex;
    gap: 14px;
    align-items: center;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--amber-wash);
    font-size: 13px;
  }
  .folderhelp p {
    margin: 0;
    flex: 1;
  }
  .src .cnt {
    font-size: 11.5px;
    color: var(--amber);
    min-width: 1.4em;
    text-align: right;
  }
  .nf {
    font-size: 12px;
    color: var(--led-bad);
  }
  .x {
    padding: 0 6px;
    margin-right: -4px;
    color: var(--muted);
    border: none;
  }
  .grow {
    flex: 1;
  }

  /* ---- filter bar ---- */
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 16px;
    height: 50px;
    border-bottom: 1px solid var(--line);
  }
  .tabs {
    display: flex;
    gap: 2px;
    align-self: stretch;
  }
  .tabs button {
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    color: var(--muted);
    padding: 0 12px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--amber);
  }
  .count {
    font-size: 11px;
    color: var(--dim);
    font-weight: 400;
  }
  .active .count {
    color: var(--amber);
  }
  .search {
    width: 210px;
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--line-strong);
    border-radius: 0;
    padding: 6px 2px;
  }
  .search:focus {
    box-shadow: none;
  }
  .spacer {
    flex: 1;
  }
  .sel {
    font-size: 12px;
    color: var(--amber);
  }

  /* ---- list ---- */
  .list {
    overflow: auto;
    background: var(--bg);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg);
    text-align: left;
    border-bottom: 1px solid var(--line-strong);
    padding: 9px 10px;
    white-space: nowrap;
  }
  .th {
    border: none;
    background: none;
    padding: 0;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }
  .th:hover:not(:disabled) {
    color: var(--amber-hot);
  }
  td {
    padding: 0 10px;
    height: 38px;
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
  }
  tbody tr {
    border-left: 3px solid transparent;
    cursor: default;
  }
  tbody tr:hover {
    background: var(--panel);
  }
  tbody tr.selected {
    background: var(--amber-wash);
    box-shadow: inset 3px 0 0 var(--amber);
  }
  tbody tr.playing .name {
    color: var(--amber);
  }
  .pv {
    width: 40px;
    padding-left: 14px;
  }
  .check {
    width: 30px;
  }
  .r {
    text-align: right;
  }
  .name {
    font-weight: 600;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fmt,
  .rec {
    font-size: 12.5px;
  }
  .st {
    min-width: 190px;
    max-width: 260px;
    white-space: normal;
    line-height: 1.15;
  }
  .stat {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--font-label);
    font-weight: 600;
    font-size: 12.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .detail {
    display: block;
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-left: 16px;
    margin-top: 1px;
  }
  td.st {
    padding-top: 4px;
    padding-bottom: 4px;
  }
  .pulse {
    animation: pulse 2.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
  .pbtn {
    width: 24px;
    height: 24px;
    padding: 0;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: transparent;
  }
  .pbtn svg {
    fill: currentColor;
  }
  .pbtn.on {
    color: var(--amber);
    border-color: var(--amber);
    box-shadow: 0 0 8px var(--amber-glow);
  }
  .empty {
    text-align: center;
    padding: 56px 20px;
  }

  /* ---- status strip + toasts ---- */
  .status {
    display: flex;
    align-items: center;
    padding: 0 16px;
    height: 28px;
    background: var(--bg);
    font-size: 12px;
  }
  .status .ver {
    min-height: 0;
    padding: 0 8px;
    border: 0;
    font-size: 12px;
    color: var(--muted);
  }
  .status .ver:hover {
    color: var(--amber);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 1;
  }
  .progress progress {
    flex: 1;
    max-width: 420px;
  }
  .toasts {
    position: absolute;
    left: 16px;
    bottom: 112px;
    display: grid;
    gap: 6px;
    width: min(520px, 70vw);
    z-index: 5;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px 7px 12px;
    background: var(--panel-2);
    border: 1px solid var(--line-strong);
    border-left: 3px solid var(--led-ok);
    box-shadow: var(--shadow);
    font-size: 13px;
  }
  .toast.error {
    border-left-color: var(--led-bad);
  }
  .toast span {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  /* ---- repair dialog ---- */
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.62);
    display: grid;
    place-items: center;
    z-index: 30;
  }
  .dialog {
    width: min(540px, 92vw);
    padding: 20px 22px;
    display: grid;
    gap: 14px;
    box-shadow: var(--shadow);
  }
  .dialog h2 {
    font-size: 16px;
    color: var(--amber);
  }
  .radio {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }
  .radio input {
    margin-top: 3px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
