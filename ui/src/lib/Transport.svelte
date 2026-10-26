<script lang="ts">
  import { audio, deviceLabel } from './audio.svelte';
  import { formatDuration, formatPrecise, meterPercent } from './format';

  interface Props {
    /** Hide the seek bar (the editor's timeline is the seek control). */
    compact?: boolean;
    onsettings: () => void;
  }
  let { compact = false, onsettings }: Props = $props();

  const st = $derived(audio.state);
  let dragging = $state<number | null>(null);
  const shown = $derived(dragging ?? st.position_secs);
  const outputLabel = $derived(audio.opened ? `${audio.opened.name}` : 'System default');
  const outputDetail = $derived(
    audio.opened
      ? `${audio.opened.host.toUpperCase()} · ${(audio.opened.rate / 1000).toFixed(audio.opened.rate % 1000 ? 1 : 0)} kHz · out ${audio.opened.out_pair * 2 + 1}-${audio.opened.out_pair * 2 + 2}`
      : 'opens on first play',
  );
  const fill = (v: number, max: number) => `${max > 0 ? Math.min(100, (v / max) * 100) : 0}%`;
</script>

<div class="transport" class:compact class:idle={!st.loaded} role="group" aria-label="Playback">
  <button class="play" class:on={st.playing} disabled={!st.loaded} onclick={() => audio.toggle()} aria-label={st.playing ? 'Pause' : 'Play'} title="Play / pause (Space)">
    {#if st.playing}
      <svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true"><rect x="3" y="2" width="3.4" height="12" /><rect x="9.6" y="2" width="3.4" height="12" /></svg>
    {:else}
      <svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true"><path d="M4 2l10 6-10 6z" /></svg>
    {/if}
  </button>

  <div class="readout">
    <div class="time mono" data-testid="position">{formatPrecise(shown)}</div>
    <div class="sub">
      {#if st.loaded}
        <span class="mono">/ {formatDuration(st.duration_secs)}</span>
        <span class="what" title={audio.label}>{audio.label}</span>
      {:else}
        <span>Nothing loaded</span>
      {/if}
    </div>
  </div>

  {#if !compact}
    <div class="seek">
      <input
        type="range"
        min="0"
        max={Math.max(st.duration_secs, 0.01)}
        step="0.01"
        value={shown}
        disabled={!st.loaded}
        style={`--fill:${fill(shown, st.duration_secs)}`}
        aria-label="Seek"
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={(e) => {
          audio.seek(Number(e.currentTarget.value));
          dragging = null;
        }}
      />
    </div>
  {:else}
    <div class="seek spacer"></div>
  {/if}

  <div class="meter" aria-hidden="true" title="Output level">
    <div class="bar"><i style={`width:${meterPercent(st.peak_l)}%`}></i></div>
    <div class="bar"><i style={`width:${meterPercent(st.peak_r)}%`}></i></div>
  </div>

  <label class="vol">
    <span class="label">Vol</span>
    <input
      type="range"
      min="0"
      max="1"
      step="0.01"
      value={st.volume}
      style={`--fill:${fill(st.volume, 1)}`}
      aria-label="Volume"
      oninput={(e) => audio.setVolume(Number(e.currentTarget.value))}
    />
  </label>

  <button class="out" onclick={onsettings} title="Choose the audio output device and channels">
    <span class="label">Output</span>
    <span class="name">{outputLabel}</span>
    <span class="detail mono">{outputDetail}</span>
  </button>
</div>
{#if audio.error || st.error}
  <div class="problem" role="alert">{st.error ?? audio.error}</div>
{/if}
{#if st.underruns > 0 && st.playing}
  <div class="problem soft">Audio dropouts: {st.underruns}. Try a larger buffer in the output settings.</div>
{/if}

<style>
  .transport {
    display: grid;
    grid-template-columns: 52px 210px minmax(80px, 1fr) 84px 110px minmax(150px, 230px);
    align-items: center;
    gap: 16px;
    padding: 10px 16px;
    background: var(--panel);
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    min-height: 72px;
  }
  .transport.idle .readout .time {
    color: var(--dim);
  }
  .play {
    width: 48px;
    height: 48px;
    padding: 0;
    display: grid;
    place-items: center;
    background: var(--inset);
    border: 1px solid var(--line-strong);
    border-radius: 50%;
  }
  .play svg {
    fill: currentColor;
  }
  .play.on {
    border-color: var(--amber);
    color: var(--amber);
    box-shadow: 0 0 14px var(--amber-glow);
  }
  .readout {
    min-width: 0;
  }
  .time {
    font-size: 28px;
    font-weight: 500;
    line-height: 1.05;
    color: var(--amber);
    text-shadow: 0 0 12px var(--amber-glow);
    white-space: nowrap;
  }
  .sub {
    display: flex;
    gap: 8px;
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
  }
  .what {
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-label);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .seek input {
    width: 100%;
  }
  .meter {
    display: grid;
    gap: 3px;
  }
  .bar {
    height: 7px;
    background: var(--inset);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: linear-gradient(to right, var(--led-ok) 0 62%, var(--amber) 62% 86%, var(--led-bad) 86%);
    background-size: 84px 100%;
    transition: width 60ms linear;
  }
  .vol {
    display: grid;
    gap: 2px;
  }
  .vol input {
    width: 100%;
  }
  .out {
    display: grid;
    gap: 1px;
    text-align: left;
    text-transform: none;
    letter-spacing: 0;
    padding: 5px 10px;
    min-width: 0;
  }
  .out .name {
    font-family: var(--font-ui);
    font-weight: 600;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .out .detail {
    font-size: 10.5px;
    color: var(--muted);
    font-weight: 400;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .problem {
    padding: 5px 16px;
    font-size: 12.5px;
    color: var(--led-bad);
    background: color-mix(in srgb, var(--led-bad) 9%, var(--panel));
    border-bottom: 1px solid var(--line);
  }
  .problem.soft {
    color: var(--amber-hot);
    background: var(--amber-wash);
  }
</style>
