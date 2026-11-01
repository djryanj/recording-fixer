<script lang="ts">
  import { MAX_FADE_SECS } from './exportOptions';

  interface Props {
    /** Fade lengths in seconds; `null` while the field is empty. */
    fadeIn: number | null;
    fadeOut: number | null;
  }
  let { fadeIn = $bindable(), fadeOut = $bindable() }: Props = $props();

  const PRESETS = [0, 2, 5, 10];
</script>

<div class="fades" role="group" aria-label="Fades">
  <div class="row">
    <label for="fade-in">Fade in</label>
    <div class="inline">
      <input id="fade-in" class="mono" type="number" min="0" max={MAX_FADE_SECS} step="0.5" placeholder="0" bind:value={fadeIn} aria-label="Fade in (seconds)" /> <span class="muted">s</span>
      {#each PRESETS as s}<button class="ghost preset" aria-label={`Fade in ${s === 0 ? 'none' : `${s} seconds`}`} onclick={() => (fadeIn = s)}>{s === 0 ? 'None' : `${s} s`}</button>{/each}
    </div>
  </div>
  <div class="row">
    <label for="fade-out">Fade out</label>
    <div class="inline">
      <input id="fade-out" class="mono" type="number" min="0" max={MAX_FADE_SECS} step="0.5" placeholder="0" bind:value={fadeOut} aria-label="Fade out (seconds)" /> <span class="muted">s</span>
      {#each PRESETS as s}<button class="ghost preset" aria-label={`Fade out ${s === 0 ? 'none' : `${s} seconds`}`} onclick={() => (fadeOut = s)}>{s === 0 ? 'None' : `${s} s`}</button>{/each}
    </div>
  </div>
  <p class="hint">
    A smooth fade at the very start and end of each exported file, in every format. A recording that begins or ends abruptly (a set cut off mid-track) sounds finished with a few seconds of fade.
    The timeline shows the fade over the waveform. It is not applied when playing in the editor.
  </p>
</div>

<style>
  .fades {
    display: grid;
    gap: 12px;
  }
  .row {
    display: grid;
    grid-template-columns: 90px 1fr;
    align-items: center;
    gap: 12px;
  }
  .inline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 10px;
  }
  input {
    width: 84px;
  }
  .preset {
    font-size: 12px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
    line-height: 1.5;
  }
</style>
