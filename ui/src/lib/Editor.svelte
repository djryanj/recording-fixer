<script lang="ts" module>
  import type { Clip, LoudnessJob, Metadata, Target } from './api';

  export interface ExportJobSpec {
    clips: Clip[];
    /** Final file name without extension. */
    stem: string;
    metadata: Metadata;
    /** Fade in over the first / out over the last N seconds of this file (0 = none). */
    fadeInSecs: number;
    fadeOutSecs: number;
  }

  export interface ExportRequest {
    target: Target;
    loudness: LoudnessJob | null;
    outputDir: string;
    jobs: ExportJobSpec[];
  }

  export const EDITOR_LABEL = 'Editor';
</script>

<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api, emptyMetadata, type FileEntry, type PlanInfo, type PreviewInfo, type Progress, type ToolStatus } from './api';
  import { audio } from './audio.svelte';
  import { buildLoudness, buildTarget, clampFade, clampLufs, defaultFormat, qualityLabel, sanitizeFormat, sanitizeLoudness, type FormatState, type LoudnessState } from './exportOptions';
  import { loadSavedExport, patchSavedExport } from './exportSettings';
  import FadesPanel from './FadesPanel.svelte';
  import FormatPanel from './FormatPanel.svelte';
  import LoudnessPanel from './LoudnessPanel.svelte';
  import { formatDuration, formatPrecise, parseTime } from './format';
  import { sessionOrder } from './library';
  import { expand, type NameContext } from './naming';
  import TagsPanel from './TagsPanel.svelte';
  import Timeline from './Timeline.svelte';
  import Transport from './Transport.svelte';
  import { frames, locate, move, removeAt, snapToFrame, splitAt, toBackend, totalSecs, trimEnd, trimStart, type TClip } from './timeline';

  interface Props {
    entries: FileEntry[];
    tool: ToolStatus | null;
    onclose: () => void;
    onstart: (req: ExportRequest) => void;
    onlocate: () => void;
    onsettings: () => void;
  }
  let { entries, tool, onclose, onstart, onlocate, onsettings }: Props = $props();

  const saved = loadSavedExport();

  let nextId = 1;
  const make = (e: FileEntry): TClip => {
    const rate = e.sample_rate ?? 44100;
    const total = Math.round((e.duration_secs ?? 0) * rate);
    return { id: nextId++, path: e.path, name: e.name, rate, total, start: 0, end: total, recorded: e.recorded, modified_ms: e.modified_ms };
  };

  // The editor is created fresh each time it opens, so reading props once is intended.
  // svelte-ignore state_referenced_locally
  let clips = $state<TClip[]>(sessionOrder(entries).map(make));
  // svelte-ignore state_referenced_locally
  let selectedId = $state<number | null>(clips[0]?.id ?? null);
  let localPlayhead = $state(0);
  let pxPerSec = $state(1);
  let timeline = $state<ReturnType<typeof Timeline>>();
  let mode = $state<'combine' | 'separate'>('combine');
  let tab = $state<'format' | 'loudness' | 'fades' | 'tags'>('format');
  // svelte-ignore state_referenced_locally
  let format = $state<FormatState>(sanitizeFormat({ ...defaultFormat(tool), ...saved.format }, tool));
  // svelte-ignore state_referenced_locally
  let nameTemplate = $state(saved.nameTemplate ?? (entries.length > 1 ? '{name} (combined)' : '{name}'));
  // svelte-ignore state_referenced_locally
  let tags = $state<Metadata>({ ...emptyMetadata(), title: '{name}', date: '{date}', comment: 'Recorded {date} {clock}', ...saved.tags });
  // svelte-ignore state_referenced_locally
  let outputDir = $state(saved.outputDir || entries[0].folder);
  // svelte-ignore state_referenced_locally
  let fadeIn = $state<number | null>(saved.fadeIn ?? 0);
  // svelte-ignore state_referenced_locally
  let fadeOut = $state<number | null>(saved.fadeOut ?? 0);
  // svelte-ignore state_referenced_locally
  let loudness = $state<LoudnessState>(sanitizeLoudness(saved.loudness));
  const fadeInSecs = $derived(clampFade(fadeIn));
  const fadeOutSecs = $derived(clampFade(fadeOut));
  let plan = $state<PlanInfo | null>(null);
  let planError = $state<string | null>(null);

  const engineHasThis = $derived(audio.state.loaded && audio.label === EDITOR_LABEL);
  const playhead = $derived(engineHasThis ? audio.state.position_secs : localPlayhead);
  const selectedIndex = $derived(clips.findIndex((c) => c.id === selectedId));
  const selected = $derived(selectedIndex >= 0 ? clips[selectedIndex] : null);
  const at = $derived(locate(clips, playhead));
  const canSplit = $derived(!!at && splitAt(clips, at.index, at.sourceFrame, -1) !== clips);
  const ext = $derived(format.kind === 'aiff' ? 'aif' : format.kind === 'mp3' ? 'mp3' : 'm4a');
  const tagsEnabled = $derived(format.kind !== 'aiff');
  const total = $derived(totalSecs(clips));
  const edited = $derived(clips.some((c) => c.start > 0 || c.end < c.total) || clips.length !== entries.length);

  // The preview plays through the loudness processing when it is on and the box says so.
  const previewJobKey = $derived(JSON.stringify(loudness.preview ? buildLoudness(loudness) : null));
  let measuring = $state<Progress | null>(null);
  let loudInfo = $state<PreviewInfo | null>(null);
  let loudError = $state<string | null>(null);

  /** Bumped whenever the recordings' loudness becomes available, so the graph fetches it. */
  let loudVersion = $state(0);
  const paths = $derived([...new Set(clips.map((c) => c.path))]);
  const pathsKey = $derived(JSON.stringify(paths));
  const wantLoudness = $derived(tab === 'loudness' || previewJobKey !== 'null');
  // What the timeline's loudness graph shows: nothing, the recordings as they are, or also the planned result.
  const loudnessLane = $derived(
    wantLoudness
      ? { gainDb: loudInfo ? loudInfo.gain_db : null, target: loudInfo && loudness.mode === 'lufs' ? clampLufs(loudness.lufs) : null, version: loudVersion }
      : null,
  );
  const nameOf = (path: string) => path.split(/[\\/]/).pop() ?? path;

  // The first time a recording is used it is measured in full; show how far that has got.
  $effect(() => {
    let off = () => {};
    let gone = false;
    void api
      .onLoudnessProgress((p) => {
        measuring = p.done >= p.total ? null : { label: `Measuring loudness of ${nameOf(p.path)}`, done: p.done, total: p.total };
      })
      .then((un) => (gone ? un() : (off = un)));
    return () => {
      gone = true;
      off();
    };
  });

  // Measure the recordings when the graph or the processed preview needs them.
  $effect(() => {
    if (!wantLoudness) return;
    const list: string[] = JSON.parse(pathsKey);
    let cancelled = false;
    api.ensureLoudness(list).then(
      () => !cancelled && loudVersion++,
      () => {},
    );
    return () => (cancelled = true);
  });

  // Keep the playback engine in step with the edit list (debounced: trim drags change it constantly).
  $effect(() => {
    const payload = clips.map(toBackend);
    const job = JSON.parse(previewJobKey);
    const list: string[] = JSON.parse(pathsKey);
    let cancelled = false;
    const now = () => untrack(() => Math.min(playhead, totalSecs(clips)));
    const timer = setTimeout(async () => {
      loudError = null;
      try {
        if (job) {
          // Already measured: straight to the processed sound. Otherwise play the original at once
          // and switch when the measurement finishes (the graph fills in meanwhile).
          const measured = api.ensureLoudness(list);
          const quick = await Promise.race([measured.then(() => true), new Promise<boolean>((r) => setTimeout(() => r(false), 120))]);
          if (cancelled) return;
          if (!quick) {
            await audio.load(payload, now(), EDITOR_LABEL, null).catch(() => {});
            await measured;
            if (cancelled) return;
          }
        }
        // What the processing comes to is worked out apart from playback, so the graph and the
        // numbers appear even where nothing can be played (no audio device).
        const plan = job ? await api.loudnessPlan(payload, job) : null;
        if (cancelled) return;
        loudInfo = plan;
        await audio.load(payload, now(), EDITOR_LABEL, job).catch(() => {}); // the transport reports playback problems
      } catch (e) {
        if (cancelled) return;
        loudInfo = null;
        if (job) loudError = String(e);
      }
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  onDestroy(() => {
    void audio.pause();
  });

  // Validate the combined plan in the backend (format mismatches, ranges), debounced.
  $effect(() => {
    if (mode !== 'combine') {
      plan = null;
      planError = null;
      return;
    }
    const payload = clips.map(toBackend);
    const t = setTimeout(async () => {
      try {
        plan = await api.planInfo(payload);
        planError = null;
      } catch (e) {
        plan = null;
        planError = String(e);
      }
    }, 250);
    return () => clearTimeout(t);
  });

  const stemOf = (name: string) => name.replace(/\.aiff?$/i, '');
  const ctxFor = (c: TClip, n: number, count: number): NameContext => ({
    name: stemOf(c.name),
    recorded: c.recorded,
    modifiedMs: c.modified_ms,
    n,
    count,
    ext,
    quality: qualityLabel(format),
  });

  function expandTags(t: Metadata, ctx: NameContext): Metadata {
    const out = emptyMetadata();
    for (const k of Object.keys(out) as (keyof Metadata)[]) out[k] = expand(t[k], ctx).text.trim();
    return out;
  }

  const plans = $derived(
    mode === 'combine' || clips.length === 0
      ? [{ group: clips, ctx: ctxFor(clips[0], 1, clips.length) }]
      : clips.map((c, i) => ({ group: [c], ctx: ctxFor(c, i + 1, clips.length) })),
  );
  const jobs = $derived<ExportJobSpec[]>(
    plans.map(({ group, ctx }) => ({
      clips: group.map(toBackend),
      stem: expand(nameTemplate, ctx).text.trim(),
      metadata: tagsEnabled ? expandTags(tags, ctx) : emptyMetadata(),
      fadeInSecs,
      fadeOutSecs,
    })),
  );
  const unknownTokens = $derived([nameTemplate, ...(tagsEnabled ? Object.values(tags) : [])].flatMap((t) => expand(t, plans[0].ctx).unknown));
  const tooBigForAiff = $derived(format.kind === 'aiff' && mode === 'combine' && plan != null && !plan.fits_aiff);
  const canStart = $derived(clips.length > 0 && !planError && !tooBigForAiff && unknownTokens.length === 0 && jobs.every((j) => j.stem !== '') && outputDir !== '');
  const sample = $derived(plans[0].ctx);

  // ---- editing actions -----------------------------------------------------

  const replace = (c: TClip) => (clips = clips.map((x) => (x.id === c.id ? c : x)));

  function seek(secs: number) {
    const s = snapToFrame(clips, secs); // always a whole sample frame
    localPlayhead = s;
    void audio.seek(s);
  }

  /** A click on the timeline: the playhead goes there and playback starts. */
  function playFrom(secs: number) {
    seek(secs);
    void audio.play();
  }

  function split() {
    if (!at || !canSplit) return;
    const id = nextId++;
    clips = splitAt(clips, at.index, at.sourceFrame, id);
  }
  function trimStartHere() {
    if (at) replace(trimStart(clips[at.index], at.sourceFrame));
  }
  function trimEndHere() {
    if (at) replace(trimEnd(clips[at.index], at.sourceFrame));
  }
  function remove() {
    if (!selected) return;
    // Read the index first: once `clips` changes, `selectedIndex` is recomputed without the removed clip.
    const index = selectedIndex;
    const next = removeAt(clips, index);
    if (next === clips) return;
    clips = next;
    selectedId = next[Math.min(index, next.length - 1)].id;
  }
  // Drag a row's grip to reorder. Pointer events, not HTML5 drag-and-drop, so it behaves the same in every webview.
  let rowsEl = $state<HTMLElement>();
  let drag = $state<{ from: number; over: number } | null>(null);
  const rowAt = (y: number) => {
    const rows = Array.from(rowsEl?.querySelectorAll<HTMLElement>('.clip') ?? []);
    let over = 0;
    rows.forEach((r, i) => {
      const b = r.getBoundingClientRect();
      if (y > b.top + b.height / 2) over = i;
    });
    return over;
  };
  function gripDown(e: PointerEvent, from: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    selectedId = clips[from].id;
    drag = { from, over: from };
  }
  function gripMove(e: PointerEvent) {
    if (drag) drag = { ...drag, over: rowAt(e.clientY) };
  }
  function gripUp() {
    if (!drag) return;
    clips = move(clips, drag.from, drag.over);
    drag = null;
  }
  function gripKey(e: KeyboardEvent, i: number) {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    e.preventDefault();
    const to = i + (e.key === 'ArrowUp' ? -1 : 1);
    clips = move(clips, i, to);
    queueMicrotask(() => rowsEl?.querySelectorAll<HTMLElement>('.grip')[Math.max(0, Math.min(to, clips.length - 1))]?.focus());
  }
  const shift = (d: number) => (clips = move(clips, selectedIndex, selectedIndex + d));
  const resetTrim = () => selected && replace({ ...selected, start: 0, end: selected.total });

  function onkeydown(e: KeyboardEvent) {
    const tag = (e.target as HTMLElement).tagName;
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') return;
    // Ctrl moves by exactly one sample frame, for lining up joins.
    const step = e.ctrlKey ? 1 / (at ? clips[at.index].rate : 44100) : e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
    switch (e.key) {
      case ' ':
        void audio.toggle();
        break;
      case 's':
      case 'S':
        split();
        break;
      case 'Delete':
      case 'Backspace':
        remove();
        break;
      case '[':
        trimStartHere();
        break;
      case ']':
        trimEndHere();
        break;
      case 'ArrowLeft':
        seek(playhead - step);
        break;
      case 'ArrowRight':
        seek(playhead + step);
        break;
      case 'Home':
        seek(0);
        break;
      case '+':
      case '=':
        timeline?.zoomBy(1.25);
        break;
      case '-':
        timeline?.zoomBy(0.8);
        break;
      case '0':
        timeline?.fit();
        break;
      case 'Escape':
        onclose();
        return;
      default:
        return;
    }
    e.preventDefault();
  }

  function editTime(c: TClip, which: 'start' | 'end', input: HTMLInputElement) {
    // "#12345" is an exact sample frame of the source recording; anything else is a time.
    const exact = /^\s*#\s*(\d+)\s*$/.exec(input.value);
    const t = exact ? Number(exact[1]) / c.rate : parseTime(input.value);
    const refresh = () => (input.value = formatPrecise((which === 'start' ? c.start : c.end) / c.rate));
    if (t == null) return refresh();
    const frame = exact ? Number(exact[1]) : t * c.rate;
    replace(which === 'start' ? trimStart(c, frame) : trimEnd(c, frame));
    refresh();
  }

  function start() {
    patchSavedExport({ format, nameTemplate, tags, outputDir, fadeIn: fadeInSecs, fadeOut: fadeOutSecs, loudness: $state.snapshot(loudness) });
    void audio.pause();
    onstart({ target: buildTarget(format), loudness: buildLoudness(loudness), outputDir, jobs });
  }
</script>

<svelte:window {onkeydown} />

<div class="editor" role="dialog" aria-modal="true" aria-label="Editor">
  <header>
    <h2>{entries.length > 1 ? 'Combine & export' : 'Edit & export'}</h2>
    <span class="meta mono">
      {clips.length} clip{clips.length === 1 ? '' : 's'} · {formatDuration(total)}{edited ? ' · edited' : ''}
    </span>
    <span class="spacer"></span>
    <button class="ghost" onclick={onclose} aria-label="Close editor" title="Close (Esc)">Close ✕</button>
  </header>

  <section class="stage">
    <Timeline
      {clips}
      {selectedId}
      playheadSecs={playhead}
      playing={audio.state.playing && engineHasThis}
      {fadeInSecs}
      {fadeOutSecs}
      loudness={loudnessLane}
      bind:pxPerSec
      bind:this={timeline}
      onselect={(id) => (selectedId = id)}
      onseek={playFrom}
      onchange={(c) => (clips = c)}
    />
  </section>

  <div class="deck"><Transport compact {onsettings} /></div>

  <div class="tools" role="toolbar" aria-label="Timeline tools">
    <span class="label">Zoom</span>
    <button class="icon" onclick={() => timeline?.zoomBy(0.8)} aria-label="Zoom out" title="Zoom out (−, or pinch / Ctrl+wheel)">−</button>
    <button class="icon" onclick={() => timeline?.zoomBy(1.25)} aria-label="Zoom in" title="Zoom in (+, or pinch / Ctrl+wheel)">+</button>
    <button onclick={() => timeline?.fit()} title="Fit everything in view (0)">Fit</button>
    <span class="sep"></span>
    <button onclick={split} disabled={!canSplit} title="Cut the clip under the playhead in two (S)">Split at playhead</button>
    <button onclick={trimStartHere} disabled={!at} title="Drop everything before the playhead ([)">Trim start here</button>
    <button onclick={trimEndHere} disabled={!at} title="Drop everything after the playhead (])">Trim end here</button>
    <span class="sep"></span>
    <button class="icon" onclick={() => shift(-1)} disabled={selectedIndex <= 0} aria-label="Move clip earlier">◀</button>
    <button class="icon" onclick={() => shift(1)} disabled={selectedIndex < 0 || selectedIndex >= clips.length - 1} aria-label="Move clip later">▶</button>
    <button onclick={remove} disabled={!selected || clips.length < 2} title="Remove the selected clip from the result (Delete)">Remove clip</button>
    <button onclick={resetTrim} disabled={!selected || (selected.start === 0 && selected.end === selected.total)}>Reset trim</button>
    <span class="sep"></span>
    <span class="mono muted playpos" title="The playhead's sample frame in the clip under it (Ctrl+Left / Right moves one sample)">{at ? `clip ${at.index + 1} · sample ${at.sourceFrame.toLocaleString()}` : '—'}</span>
    <span class="spacer"></span>
    <span class="hint muted"><b>Click</b> a waveform to play from there · <b>drag across</b> it or <b>pinch</b> to zoom (down to single samples) · <b>drag a clip's name bar</b> (or its ⠿ grip in the list) to reorder · drag its <b>edges</b> to trim</span>
  </div>

  <div class="lower">
    <section class="clips panel" aria-label="Clips">
      <div class="clip head label"><span>#</span><span>Recording</span><span>Start</span><span>End</span><span>Length</span></div>
      <div class="rows" bind:this={rowsEl}>
        {#each clips as c, i (c.id)}
          <div
            class="clip"
            class:sel={c.id === selectedId}
            class:dragging={drag?.from === i}
            class:drop-before={drag && drag.over === i && drag.from > i}
            class:drop-after={drag && drag.over === i && drag.from < i}
            onclick={() => (selectedId = c.id)}
            role="presentation"
          >
            <span class="num">
              <button
                class="grip"
                title="Drag to reorder (or press Up / Down)"
                aria-label={`Reorder clip ${i + 1}: ${c.name}`}
                onpointerdown={(e) => gripDown(e, i)}
                onpointermove={gripMove}
                onpointerup={gripUp}
                onpointercancel={() => (drag = null)}
                onkeydown={(e) => gripKey(e, i)}>⠿</button>
              <span class="mono muted">{String(i + 1).padStart(2, '0')}</span>
            </span>
            <span class="nm" title={c.path}>{c.name}</span>
            <input class="mono" type="text" value={formatPrecise(c.start / c.rate)} onchange={(e) => editTime(c, 'start', e.currentTarget)} aria-label={`Start of clip ${i + 1}`} title={`Sample frame ${c.start.toLocaleString()}. Type a time, or #N for an exact sample frame.`} />
            <input class="mono" type="text" value={formatPrecise(c.end / c.rate)} onchange={(e) => editTime(c, 'end', e.currentTarget)} aria-label={`End of clip ${i + 1}`} title={`Sample frame ${c.end.toLocaleString()} (the first one left out). Type a time, or #N for an exact sample frame.`} />
            <span class="mono muted">{formatDuration(frames(c) / c.rate)}</span>
          </div>
        {/each}
      </div>
      <div class="modes">
        {#if clips.length > 1}
          <label><input type="radio" bind:group={mode} value="combine" /> Combine into one file</label>
          <label><input type="radio" bind:group={mode} value="separate" /> One file per clip <span class="muted">({clips.length})</span></label>
        {/if}
        {#if planError}
          <p class="err">{planError}</p>
        {:else if plan && mode === 'combine'}
          <p class="muted result"><span class="mono">{formatDuration(plan.duration_secs)}</span> · {plan.format}</p>
        {/if}
        {#if tooBigForAiff}
          <p class="err">Over AIFF's 4 GB limit. Choose MP3 or AAC, or export one file per clip.</p>
        {/if}
      </div>
    </section>

    <section class="output panel" aria-label="Output">
      <div class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === 'format'} class:active={tab === 'format'} onclick={() => (tab = 'format')}>Format &amp; quality</button>
        <button role="tab" aria-selected={tab === 'loudness'} class:active={tab === 'loudness'} onclick={() => (tab = 'loudness')}>Loudness{#if loudness.mode !== 'off'} <i class="led ok" title="On"></i>{/if}</button>
        <button role="tab" aria-selected={tab === 'fades'} class:active={tab === 'fades'} onclick={() => (tab = 'fades')}>Fades{#if fadeInSecs > 0 || fadeOutSecs > 0} <i class="led ok" title="On"></i>{/if}</button>
        <button role="tab" aria-selected={tab === 'tags'} class:active={tab === 'tags'} onclick={() => (tab = 'tags')}>Name, folder &amp; tags</button>
      </div>
      <div class="tabpanel">
        {#if tab === 'format'}
          <FormatPanel bind:format {tool} durationSecs={total} aiffBytes={plan?.bytes ?? null} {onlocate} />
        {:else if tab === 'fades'}
          <FadesPanel bind:fadeIn bind:fadeOut />
        {:else if tab === 'loudness'}
          <LoudnessPanel bind:loudness preview={{ measuring, info: loudInfo, error: loudError }} />
        {:else}
          <TagsPanel bind:nameTemplate bind:tags bind:outputDir {sample} {tagsEnabled} separate={mode === 'separate'} {ext} />
        {/if}
      </div>
    </section>
  </div>

  <footer>
    {#if unknownTokens.length}
      <span class="err">Unknown placeholder {unknownTokens.map((u) => `{${u}}`).join(', ')} — fix it under "Name, folder &amp; tags"</span>
    {:else}
      <span class="muted summary mono">{jobs.length > 1 ? `${jobs.length} files` : jobs[0]?.stem ? `${jobs[0].stem}.${ext}` : ''}</span>
    {/if}
    <span class="spacer"></span>
    <button onclick={onclose}>Cancel</button>
    <button class="primary" disabled={!canStart} onclick={start}>Export</button>
  </footer>
</div>

<style>
  .editor {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto auto auto auto minmax(0, 1fr) auto;
    background: var(--bg);
  }
  .editor > * {
    min-width: 0;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 16px;
    padding: 12px 16px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }
  header h2 {
    font-size: 15px;
    color: var(--amber);
  }
  .meta {
    font-size: 12px;
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .stage {
    padding: 12px 16px 12px;
    min-width: 0;
    overflow: hidden;
  }
  .tools {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--line-strong);
    margin: 0 4px;
  }
  .hint {
    font-size: 12px;
  }
  .hint b {
    color: var(--text);
    font-weight: 600;
  }
  .lower {
    display: grid;
    grid-template-columns: minmax(380px, 5fr) minmax(420px, 6fr);
    gap: 14px;
    padding: 14px 16px;
    min-height: 0;
  }
  .clips,
  .output {
    min-height: 0;
    display: grid;
  }
  .clips {
    grid-template-rows: auto minmax(0, 1fr) auto;
  }
  .rows {
    overflow: auto;
  }
  .clip {
    display: grid;
    grid-template-columns: 52px 1fr 92px 92px 62px;
    gap: 8px;
    align-items: center;
    padding: 4px 10px;
    border-left: 3px solid transparent;
  }
  .clip.head {
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
  }
  .clip.sel {
    border-left-color: var(--amber);
    background: var(--amber-wash);
  }
  .num {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .grip {
    all: unset;
    cursor: grab;
    color: var(--muted);
    padding: 2px 3px;
    touch-action: none;
    line-height: 1;
  }
  .grip:hover,
  .grip:focus-visible {
    color: var(--amber);
  }
  .clip.dragging {
    opacity: 0.55;
  }
  .clip.drop-before {
    box-shadow: inset 0 2px 0 var(--amber);
  }
  .clip.drop-after {
    box-shadow: inset 0 -2px 0 var(--amber);
  }
  .clip input {
    padding: 3px 6px;
    font-size: 12.5px;
  }
  .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .modes {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    align-items: center;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
  }
  .modes label {
    display: inline-flex;
    gap: 7px;
    align-items: center;
  }
  .modes p {
    margin: 0;
    flex-basis: 100%;
  }
  .result {
    font-size: 13px;
  }
  .output {
    grid-template-rows: auto minmax(0, 1fr);
  }
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    padding: 10px 16px;
    color: var(--muted);
  }
  .tabs button.active {
    border-bottom-color: var(--amber);
    color: var(--text);
  }
  .tabpanel {
    padding: 14px 16px;
    overflow: auto;
  }
  .err {
    color: var(--led-bad);
    margin: 0;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-top: 1px solid var(--line);
    background: var(--panel);
  }
  .summary {
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
