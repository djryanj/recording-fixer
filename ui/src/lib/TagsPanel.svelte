<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { emptyMetadata, type Metadata } from './api';
  import { expand, FILENAME_PRESETS, TOKENS, type NameContext } from './naming';

  interface Props {
    nameTemplate: string;
    tags: Metadata;
    outputDir: string;
    /** Context for the first output, used to preview templates. */
    sample: NameContext;
    tagsEnabled: boolean;
    separate: boolean;
    ext: string;
  }
  let { nameTemplate = $bindable(), tags = $bindable(), outputDir = $bindable(), sample, tagsEnabled, separate, ext }: Props = $props();

  const preview = $derived(expand(nameTemplate, sample));
  const FIELDS: { key: keyof Metadata; label: string; placeholder: string }[] = $derived([
    { key: 'title', label: 'Title', placeholder: '{name}' },
    { key: 'artist', label: 'Artist', placeholder: 'DJ name' },
    { key: 'album', label: 'Album', placeholder: 'e.g. Live Sets 2025' },
    { key: 'album_artist', label: 'Album artist', placeholder: '' },
    { key: 'genre', label: 'Genre', placeholder: 'House' },
    { key: 'date', label: 'Date / year', placeholder: '{date}' },
    { key: 'track', label: 'Track no.', placeholder: separate ? '{n}' : '' },
    { key: 'comment', label: 'Comment', placeholder: 'Recorded {date} {clock}' },
  ]);

  async function browse() {
    const dir = await open({ directory: true, multiple: false, title: 'Choose an output folder', defaultPath: outputDir });
    if (typeof dir === 'string') outputDir = dir;
  }

  function addToken(t: string) {
    nameTemplate += `{${t}}`;
  }

  const hint = (template: string) => (template.includes('{') ? expand(template, sample).text : '');
  const unknownIn = (template: string) => expand(template, sample).unknown;
  const reset = () => (tags = { ...emptyMetadata(), title: '{name}', date: '{date}', comment: 'Recorded {date} {clock}' });
</script>

<div class="tpanel">
  <section>
    <h3>File name</h3>
    <div class="row">
      <input type="text" bind:value={nameTemplate} aria-label="File name template" spellcheck="false" />
      <span class="ext muted">.{ext}</span>
      <select
        aria-label="File name presets"
        onchange={(e) => {
          if (e.currentTarget.value) nameTemplate = e.currentTarget.value;
          e.currentTarget.value = '';
        }}
      >
        <option value="">Presets…</option>
        {#each FILENAME_PRESETS as p}<option value={p.template}>{p.label}: {p.template}</option>{/each}
      </select>
    </div>
    <div class="tokens">
      {#each TOKENS as t}
        <button class="token" title={t.help} onclick={() => addToken(t.token)}>{`{${t.token}}`}</button>
      {/each}
    </div>
    {#if preview.unknown.length}
      <p class="err">Unknown placeholder: {preview.unknown.map((u) => `{${u}}`).join(', ')}</p>
    {:else}
      <p class="muted preview">Result: <b>{preview.text || '(empty)'}.{ext}</b>{separate ? ' (first file)' : ''}</p>
    {/if}
  </section>

  <section>
    <h3>Save to</h3>
    <div class="row">
      <input type="text" readonly value={outputDir} aria-label="Output folder" />
      <button onclick={browse}>Browse…</button>
    </div>
  </section>

  <section>
    <div class="head">
      <h3>Tags</h3>
      {#if tagsEnabled}<button class="link" onclick={reset}>Reset</button>{/if}
    </div>
    {#if !tagsEnabled}
      <p class="muted">AIFF export does not write tags. Choose MP3 or AAC to set them.</p>
    {:else}
      <div class="fields">
        {#each FIELDS as f}
          <label for={`tag-${f.key}`}>{f.label}</label>
          <div class="field">
            <input id={`tag-${f.key}`} type="text" bind:value={tags[f.key]} placeholder={f.placeholder} spellcheck="false" />
            {#if unknownIn(tags[f.key]).length}
              <span class="err small">unknown {`{${unknownIn(tags[f.key])[0]}}`}</span>
            {:else if hint(tags[f.key])}
              <span class="muted small">→ {hint(tags[f.key])}</span>
            {/if}
          </div>
        {/each}
      </div>
      <p class="muted small">Fields can use the same placeholders as the file name. Empty fields are left out.</p>
    {/if}
  </section>
</div>

<style>
  .tpanel {
    display: grid;
    gap: 14px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 13px;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .row input {
    flex: 1;
  }
  .ext {
    white-space: nowrap;
  }
  .tokens {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
  }
  .token {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 999px;
  }
  .preview,
  .err {
    margin: 6px 0 0;
  }
  .err {
    color: var(--led-bad);
  }
  .small {
    font-size: 12px;
  }
  .fields {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 6px 10px;
    align-items: center;
  }
  .field {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .field input {
    flex: 1;
  }
</style>
