<script lang="ts">
  import type { ToolStatus } from './api';
  import { formatSize } from './format';
  import {
    AAC_CBR,
    AAC_VBR,
    encoderLabel,
    estimateKbps,
    MP3_ABR,
    MP3_CBR,
    MP3_VBR,
    type FormatState,
  } from './exportOptions';

  interface Props {
    format: FormatState;
    tool: ToolStatus | null;
    durationSecs: number | null;
    aiffBytes: number | null;
    onlocate: () => void;
  }
  let { format = $bindable(), tool, durationSecs, aiffBytes, onlocate }: Props = $props();

  const ffmpegOk = $derived(!!tool?.ffmpeg_path);
  const mp3Ok = $derived(ffmpegOk && !!tool?.mp3);
  const encoders = $derived(tool?.aac_encoders ?? []);
  const aacOk = $derived(ffmpegOk && encoders.length > 0);
  const kbps = $derived(estimateKbps(format));
  const estimate = $derived(
    kbps != null && durationSecs != null ? (kbps * 1000 * durationSecs) / 8 : format.kind === 'aiff' ? aiffBytes : null,
  );
  const noVbr = $derived(format.kind === 'aac' && (format.aacEncoder === 'aac_mf' || !['apple_coreaudio', 'aac_at', 'libfdk_aac', 'aac'].includes(format.aacEncoder)));

  function setMp3Mode(mode: 'cbr' | 'vbr' | 'abr') {
    format.mp3Mode = mode;
    // pick a sensible value when switching families
    format.mp3Value = mode === 'vbr' ? 2 : 320;
  }
  function setAacMode(mode: 'cbr' | 'vbr') {
    format.aacMode = mode;
    format.aacValue = mode === 'vbr' ? 4 : 256;
  }
</script>

<div class="fpanel">
  <div class="row kinds" role="radiogroup" aria-label="Format">
    <label><input type="radio" bind:group={format.kind} value="aiff" /> <b>AIFF</b> <span class="muted">lossless</span></label>
    <label class:dim={!mp3Ok}><input type="radio" bind:group={format.kind} value="mp3" disabled={!mp3Ok} /> <b>MP3</b></label>
    <label class:dim={!aacOk}><input type="radio" bind:group={format.kind} value="aac" disabled={!aacOk} /> <b>AAC</b> <span class="muted">.m4a</span></label>
  </div>

  {#if format.kind === 'mp3'}
    <div class="grid">
      <label for="mp3mode">Bitrate mode</label>
      <select id="mp3mode" value={format.mp3Mode} onchange={(e) => setMp3Mode(e.currentTarget.value as 'cbr' | 'vbr' | 'abr')}>
        <option value="cbr">Constant (CBR): predictable size, best compatibility</option>
        <option value="vbr">Variable (VBR): best quality for the size</option>
        <option value="abr">Average (ABR): target an average</option>
      </select>

      <label for="mp3val">{format.mp3Mode === 'vbr' ? 'Quality' : 'Bitrate'}</label>
      <select id="mp3val" bind:value={format.mp3Value}>
        {#if format.mp3Mode === 'vbr'}
          {#each MP3_VBR as v}<option value={v.v}>V{v.v} · ~{v.kbps} kbps{v.v === 0 ? ' (best)' : ''}</option>{/each}
        {:else}
          {#each format.mp3Mode === 'cbr' ? MP3_CBR : MP3_ABR as k}<option value={k}>{k} kbps{k === 320 ? ' (max)' : ''}</option>{/each}
        {/if}
      </select>

      <label for="id3v">ID3 tag version</label>
      <div class="inline">
        <select id="id3v" bind:value={format.id3Version}>
          <option value={3}>ID3v2.3 (most compatible)</option>
          <option value={4}>ID3v2.4</option>
        </select>
        <label class="check"><input type="checkbox" bind:checked={format.id3v1} /> also write ID3v1</label>
      </div>
    </div>
  {:else if format.kind === 'aac'}
    <div class="grid">
      <label for="aacenc">Encoder</label>
      <select id="aacenc" bind:value={format.aacEncoder}>
        {#each encoders as enc, i}<option value={enc}>{encoderLabel(enc)}{i === 0 ? ' (best)' : ''}</option>{/each}
      </select>

      <label for="aacmode">Bitrate mode</label>
      <select id="aacmode" value={format.aacMode} onchange={(e) => setAacMode(e.currentTarget.value as 'cbr' | 'vbr')}>
        <option value="cbr">Constant (CBR)</option>
        <option value="vbr">Variable (VBR / quality)</option>
      </select>

      <label for="aacval">{format.aacMode === 'vbr' ? 'Quality' : 'Bitrate'}</label>
      <select id="aacval" bind:value={format.aacValue}>
        {#if format.aacMode === 'vbr'}
          {#each AAC_VBR as v}<option value={v.level}>{v.label}</option>{/each}
        {:else}
          {#each AAC_CBR as k}<option value={k}>{k} kbps{k === 256 ? ' (iTunes Plus)' : ''}</option>{/each}
        {/if}
      </select>
    </div>
    {#if noVbr && format.aacMode === 'vbr'}
      <p class="hint">This encoder has no true VBR; the quality level is approximated with a constant bitrate.</p>
    {/if}
    {#if !tool?.apple}
      <p class="hint">Apple's AAC encoder was not found, so a different encoder is used. See the README to add it.</p>
    {:else if format.aacEncoder === 'apple_coreaudio'}
      <p class="hint">Apple's encoder delay is compensated in the file, so players that honour it start exactly on time.</p>
    {/if}
  {:else}
    <p class="hint">AIFF keeps the audio exactly as recorded. It has no tags and cannot exceed 4 GB.</p>
  {/if}

  {#if !ffmpegOk}
    <p class="err">ffmpeg was not found, so MP3 and AAC are unavailable. <button class="link" onclick={onlocate}>Locate ffmpeg…</button></p>
  {:else if format.kind === 'mp3' && !mp3Ok}
    <p class="err">This ffmpeg build has no MP3 encoder (libmp3lame).</p>
  {/if}

  {#if estimate != null}
    <p class="muted est">Estimated size: about {formatSize(estimate)}</p>
  {/if}
</div>

<style>
  .fpanel {
    display: grid;
    gap: 10px;
  }
  .kinds {
    display: flex;
    gap: 20px;
  }
  .dim {
    opacity: 0.5;
  }
  .grid {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 8px 12px;
    align-items: center;
  }
  .inline {
    display: flex;
    gap: 14px;
    align-items: center;
  }
  .check {
    white-space: nowrap;
  }
  .hint,
  .est {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
  }
  .err {
    margin: 0;
    color: var(--led-bad);
  }
</style>
