<script lang="ts">
  import type { PreviewInfo, Progress } from './api';
  import { LEVELING_CHOICES, LUFS_PRESETS, type LoudnessState } from './exportOptions';

  /** What the editor's preview is doing with the loudness settings; `null` where there is no preview (Settings). */
  export interface PreviewStatus {
    /** Measuring the recordings (the first time only), with progress. */
    measuring: Progress | null;
    info: PreviewInfo | null;
    error: string | null;
  }

  interface Props {
    loudness: LoudnessState;
    preview?: PreviewStatus | null;
  }
  let { loudness = $bindable(), preview = null }: Props = $props();

  const db = (v: number) => `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(1)} dB`;
  const lufs = (v: number) => (Number.isFinite(v) ? `${v.toFixed(1).replace('-', '−')} LUFS` : 'silence');
  const percent = (p: Progress) => Math.min(100, Math.floor((p.done / Math.max(1, p.total)) * 100));
</script>

<div class="loud" role="group" aria-label="Loudness">
  <div class="modes" role="radiogroup" aria-label="Loudness mode">
    <label><input type="radio" bind:group={loudness.mode} value="off" /> Leave as recorded</label>
    <label><input type="radio" bind:group={loudness.mode} value="lufs" /> Match a loudness</label>
    <label><input type="radio" bind:group={loudness.mode} value="peak" /> Raise to a peak limit</label>
  </div>

  {#if loudness.mode === 'lufs'}
    <div class="grid">
      <label for="lufs">Target loudness</label>
      <div class="inline">
        <input id="lufs" class="mono num" type="number" min="-40" max="-3" step="0.5" bind:value={loudness.lufs} /> <span class="muted">LUFS</span>
        {#each LUFS_PRESETS as p}<button class="ghost preset" onclick={() => (loudness.lufs = p.lufs)}>{p.label}</button>{/each}
      </div>
      <label for="leveling">Evening out</label>
      <select id="leveling" bind:value={loudness.leveling}>
        {#each LEVELING_CHOICES as c}<option value={c.value}>{c.label}</option>{/each}
      </select>
      <label for="ceiling">Peak limit</label>
      <div class="inline"><input id="ceiling" class="mono num" type="number" min="-20" max="0" step="0.1" bind:value={loudness.ceiling} /> <span class="muted">dBTP</span></div>
    </div>
    <p class="hint">
      Brings the whole set to this loudness. Stretches much louder than their surroundings (two tracks beatmixed) are turned down, and a track that was left too quiet is turned up,
      while breakdowns stay quiet. A limiter keeps peaks under the limit. For MP3 and AAC use −1 or lower. This changes the audio, so it is no longer an exact copy of the recording.
    </p>
  {:else if loudness.mode === 'peak'}
    <div class="grid">
      <label for="ceiling">Peak limit</label>
      <div class="inline"><input id="ceiling" class="mono num" type="number" min="-20" max="0" step="0.1" bind:value={loudness.ceiling} /> <span class="muted">dBTP</span></div>
    </div>
    <p class="hint">Makes the whole recording louder or quieter by one steady amount so its highest peak just reaches the limit. Nothing else changes.</p>
  {:else}
    <p class="hint">The audio is exported exactly as recorded. Choose one of the other two to change its loudness.</p>
  {/if}

  {#if preview}
    <p class="hint">
      The graph under the waveform shows the loudness over time (grey, a 3 second average in LUFS) and, once a mode is chosen, what it will be afterwards (amber).
      Look for the beatmixes that stick out and the stretches that are too quiet.
    </p>
  {/if}

  {#if preview?.measuring}
    <div class="measuring">
      <div class="status" role="status">
        <progress max="100" value={percent(preview.measuring)} aria-label="Measuring loudness"></progress>
        <span>{preview.measuring.label} {percent(preview.measuring)}%</span>
      </div>
      <p class="hint">
        The first time a recording is used it is measured from start to end. The graph under the waveform fills in as it goes, and you can keep editing and playing the original meanwhile.
        After this it is instant and remembered between sessions.
      </p>
    </div>
  {/if}

  {#if preview && loudness.mode !== 'off'}
    <div class="preview">
      <label class="check"><input type="checkbox" bind:checked={loudness.preview} /> Hear this when playing in the editor</label>
      {#if loudness.preview}
        {#if preview.measuring}
          <p class="hint">Playing the original until the measurement is done, then it switches to the processed sound.</p>
        {:else if preview.error}
          <p class="err" role="status">The preview could not use this: {preview.error}</p>
        {:else if preview.info}
          <p class="hint" role="status">
            Playing with it on: the recording measures <b>{lufs(preview.info.input_lufs)}</b>; overall {db(preview.info.static_gain_db)}, moving between {db(preview.info.gain_min_db)} and {db(preview.info.gain_max_db)}.
            Turn it off to compare with the original.
          </p>
        {/if}
        <p class="hint">The preview is within about half a LU of the export, which makes one more check pass before it writes the file.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .loud {
    display: grid;
    gap: 10px;
  }
  .modes {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
  }
  .grid {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 8px 12px;
    align-items: center;
  }
  .inline {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    align-items: center;
  }
  .num {
    width: 84px;
  }
  .preset {
    font-size: 12px;
  }
  .measuring {
    display: grid;
    gap: 6px;
  }
  .preview {
    display: grid;
    gap: 6px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
  }
  .status progress {
    width: 180px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
    line-height: 1.5;
  }
  .err {
    margin: 0;
    color: var(--led-bad);
  }
</style>
