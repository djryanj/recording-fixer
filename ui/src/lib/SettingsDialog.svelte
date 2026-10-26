<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { ToolStatus } from './api';
  import { appVersion, ISSUES_URL, openLink, REPO_URL } from './about';
  import { clampFade, defaultFormat, encoderLabel, sanitizeFormat, type FormatState } from './exportOptions';
  import { loadSavedExport, patchSavedExport } from './exportSettings';
  import FormatPanel from './FormatPanel.svelte';

  interface Props {
    tool: ToolStatus | null;
    /** The ffmpeg the user chose by hand, or null when it is found automatically. */
    userPath: string | null;
    onchoose: () => void;
    onreset: () => void;
    /** Open the audio output (playback device) window. */
    onaudio: () => void;
    onclose: () => void;
  }
  let { tool, userPath, onchoose, onreset, onaudio, onclose }: Props = $props();

  const found = $derived(!!tool?.ffmpeg_path);
  const version = $derived(tool?.ffmpeg_version?.replace(/^ffmpeg version\s+/i, '').split(' ')[0] ?? '');

  let appVer = $state('');
  onMount(() => void appVersion().then((v) => (appVer = v)));

  // The same remembered choices the export editor starts from, so what is set here is what a new export begins with.
  const saved = untrack(() => loadSavedExport());
  // svelte-ignore state_referenced_locally
  let format = $state<FormatState>(sanitizeFormat({ ...defaultFormat(tool), ...saved.format }, tool));
  let fadeIn = $state<number | null>(saved.fadeIn ?? 0);
  let fadeOut = $state<number | null>(saved.fadeOut ?? 0);

  // Save only when something is changed here, not just for opening the window.
  let untouched = true;
  $effect(() => {
    const patch = { format: $state.snapshot(format), fadeIn: clampFade(fadeIn), fadeOut: clampFade(fadeOut) };
    if (untouched) {
      untouched = false;
      return;
    }
    patchSavedExport(patch);
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div class="backdrop" role="presentation" onclick={onclose}>
  <div class="dialog panel" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <h2>Settings</h2>

    <section aria-label="Default export">
      <h3 class="label">Default export</h3>
      <p class="why">What a new export starts with. You can still change it for any one export in the editor, which then remembers what you used last.</p>
      <FormatPanel bind:format bind:fadeIn bind:fadeOut {tool} durationSecs={null} aiffBytes={null} onlocate={onchoose} />
    </section>

    <section aria-label="ffmpeg">
      <h3 class="label">ffmpeg: needed to save MP3 and AAC</h3>
      <p class="why">
        Saving as <b>MP3</b> or <b>AAC (.m4a)</b> uses <b>ffmpeg</b>, a free audio converter this app runs for you.
        That is true even when Apple's AAC encoder does the encoding: ffmpeg still packs its output into the .m4a file.
        You do <b>not</b> need ffmpeg for repairing files, playing them, editing, or saving as <b>AIFF</b>.
      </p>
      <dl>
        <dt class="label">Status</dt>
        <dd>
          {#if found}
            <i class="led ok"></i> Found <span class="mono">{version}</span>
            <div class="path mono" title={tool?.ffmpeg_path ?? ''}>{tool?.ffmpeg_path}</div>
            <div class="muted">{userPath ? 'This is the program you chose.' : 'Found automatically (next to this app, or on your PATH).'}</div>
          {:else}
            <i class="led bad"></i> <b>Not found.</b> MP3 and AAC export are unavailable until it is.
            <div class="muted">
              Install it (on Windows: <code>winget install Gyan.FFmpeg</code>, or download from ffmpeg.org), then reopen this window, or point the app at it yourself.
            </div>
          {/if}
        </dd>

        {#if found}
          <dt class="label">MP3</dt>
          <dd>{#if tool?.mp3}<i class="led ok"></i> This ffmpeg can make MP3 files{:else}<i class="led bad"></i> This ffmpeg has no MP3 encoder (libmp3lame){/if}</dd>

          <dt class="label">AAC</dt>
          <dd>
            {#if tool && tool.aac_encoders.length}
              <div>AAC encoders found on this computer, best first:</div>
              <ol class="encoders">
                {#each tool.aac_encoders as enc, i}
                  <li>{encoderLabel(enc)}{#if i === 0} <span class="tag">used unless you pick another</span>{/if}</li>
                {/each}
              </ol>
              <div class="muted">The app ranks them for you: Apple's, then Fraunhofer's, then ffmpeg's own, then Windows'. This is information only; choose which one to use under <b>AAC</b> in Default export above.</div>
            {:else}
              <i class="led bad"></i> No AAC encoder found
            {/if}
            <div class="muted apple">
              {tool?.apple ?? 'Apple’s higher-quality AAC encoder was not found. On Windows it comes with iTunes; see the README.'}
            </div>
          </dd>
        {/if}
      </dl>
      <div class="row">
        {#if userPath}<button class="ghost" onclick={onreset}>Use the automatic one</button>{/if}
        <button onclick={onchoose}>{found ? 'Choose a different ffmpeg…' : 'I have ffmpeg: choose it…'}</button>
      </div>
    </section>

    <section aria-label="Playback">
      <h3 class="label">Playback</h3>
      <div class="row">
        <button onclick={onaudio}>Audio output device and channels…</button>
        <span class="muted">Where recordings play: WASAPI, ASIO or CoreAudio, and which output pair.</span>
      </div>
    </section>

    <section aria-label="About">
      <h3 class="label">About</h3>
      <p class="why">
        <b>Recording Fixer for Serato DJ</b> <span class="mono" data-testid="app-version">{appVer ? `v${appVer}` : ''}</span><br />
        Free, open-source software under the GNU GPL v3 or later. Not affiliated with or endorsed by Serato.
        ASIO is a registered trademark of Steinberg Media Technologies GmbH.
      </p>
      <div class="row">
        <button onclick={() => openLink(ISSUES_URL)}>Report an issue on GitHub</button>
        <button class="ghost" onclick={() => openLink(REPO_URL)}>Source code</button>
      </div>
    </section>

    <footer>
      <span class="spacer"></span>
      <button class="primary" onclick={onclose}>Done</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.62);
    display: grid;
    place-items: center;
    z-index: 40;
  }
  .dialog {
    width: min(640px, 94vw);
    max-height: 92vh;
    overflow: auto;
    padding: 20px 22px;
    display: grid;
    gap: 16px;
    box-shadow: var(--shadow);
  }
  h2 {
    font-size: 16px;
    color: var(--amber);
  }
  section {
    display: grid;
    gap: 10px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }
  h3 {
    margin: 0;
    color: var(--amber);
  }
  .why {
    margin: 0;
    line-height: 1.5;
    font-size: 13px;
  }
  dl {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 10px 14px;
    margin: 0;
  }
  dt {
    padding-top: 2px;
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  dd > .muted,
  dd .path {
    margin-top: 3px;
    font-size: 12.5px;
  }
  .path {
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .apple {
    margin-top: 6px;
  }
  .encoders {
    margin: 4px 0;
    padding-left: 20px;
  }
  .tag {
    margin-left: 8px;
    font-size: 11.5px;
    color: var(--amber);
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
  }
  footer {
    display: flex;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
</style>
