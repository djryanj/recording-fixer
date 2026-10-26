<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type AudioDevice, type DeviceSelection } from './api';
  import { audio, channelPairs, defaultSelection } from './audio.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let pick = $state<DeviceSelection>({ ...audio.selection });
  let busy = $state(false);
  let refreshing = $state(false);
  let error = $state<string | null>(null);
  /** Channel counts learned on demand (ASIO drivers are not probed when listing). */
  let channels = $state(new Map<string, number>());

  const HOST_ORDER = ['asio', 'coreaudio', 'wasapi'];
  const HOST_TITLE: Record<string, string> = {
    asio: 'ASIO: low latency, usually one program at a time',
    wasapi: 'Windows audio (WASAPI): shared with other programs',
    coreaudio: 'CoreAudio',
  };
  const BUFFERS = [64, 128, 256, 512, 1024, 2048];

  const groups = $derived(
    [...new Set(audio.devices.map((d) => d.host))]
      .sort((a, b) => (HOST_ORDER.indexOf(a) + 99) % 99 - (HOST_ORDER.indexOf(b) + 99) % 99 || a.localeCompare(b))
      .map((host) => ({ host, devices: audio.devices.filter((d) => d.host === host) })),
  );
  const current = $derived(audio.devices.find((d) => d.id === pick.id) ?? null);
  const channelCount = $derived(pick.id ? (channels.get(pick.id) ?? current?.channels ?? 0) : (audio.opened?.channels ?? 2));
  const pairs = $derived(channelPairs(channelCount || 2));
  const rate = $derived(audio.opened?.rate ?? 44100);

  async function learnChannels(d: AudioDevice) {
    if (d.channels > 0 || channels.has(d.id)) return;
    const details = await api.audioDeviceDetails(d.id).catch(() => null);
    if (details) channels = new Map(channels).set(d.id, details.channels);
  }

  function choose(d: AudioDevice | null) {
    pick = { ...pick, id: d?.id ?? null, out_pair: 0 };
    if (d) void learnChannels(d);
  }

  async function refresh() {
    refreshing = true;
    await audio.refreshDevices();
    refreshing = false;
  }

  async function apply() {
    busy = true;
    error = null;
    try {
      await audio.applySelection(pick);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refresh();
    const current = audio.devices.find((d) => d.id === pick.id);
    if (current) void learnChannels(current);
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div class="backdrop" role="presentation" onclick={onclose}>
  <div class="dialog panel" role="dialog" aria-modal="true" aria-label="Audio output" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <header>
      <h2>Audio output</h2>
      <button class="ghost" onclick={refresh} disabled={refreshing}>{refreshing ? 'Scanning…' : 'Rescan'}</button>
    </header>

    <div class="list" role="radiogroup" aria-label="Output device">
      <label class="row" class:sel={pick.id === null}>
        <input type="radio" name="dev" checked={pick.id === null} onchange={() => choose(null)} />
        <span class="nm">System default output</span>
        <span class="tag">Windows chooses</span>
      </label>
      {#each groups as g (g.host)}
        <div class="group label">{HOST_TITLE[g.host] ?? g.host}</div>
        {#each g.devices as d (d.id)}
          <label class="row" class:sel={pick.id === d.id}>
            <input type="radio" name="dev" checked={pick.id === d.id} onchange={() => choose(d)} />
            <span class="nm">{d.name}</span>
            {#if d.is_default}<span class="tag">default</span>{/if}
            {#if d.channels > 0}<span class="tag mono">{d.channels} ch</span>{/if}
          </label>
        {/each}
      {/each}
      {#if !audio.devices.length}
        <p class="muted none">No output devices were found.</p>
      {/if}
    </div>

    <div class="opts">
      <label for="pair" class="label">Output channels</label>
      <select id="pair" bind:value={pick.out_pair} disabled={pairs.length < 2}>
        {#each pairs as p}<option value={p.index}>Outputs {p.label}</option>{/each}
      </select>

      <label for="buf" class="label">Buffer size</label>
      <select id="buf" bind:value={pick.buffer_frames}>
        <option value={null}>Driver default</option>
        {#each BUFFERS as b}<option value={b}>{b} samples · {((b / rate) * 1000).toFixed(1)} ms</option>{/each}
      </select>
    </div>
    <p class="note muted">
      Smaller buffers respond faster but may crackle. ASIO drivers normally serve one program at a time, so close Serato (or pick another output) if the device will not open.
    </p>

    {#if error}<p class="err" role="alert">{error}</p>{/if}

    <footer>
      <button class="ghost" onclick={() => (pick = defaultSelection())}>Reset</button>
      <span class="spacer"></span>
      <button onclick={onclose}>Cancel</button>
      <button class="primary" onclick={apply} disabled={busy}>{busy ? 'Opening…' : 'Apply'}</button>
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
    z-index: 30;
  }
  .dialog {
    width: min(620px, 94vw);
    max-height: 90vh;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto auto auto auto;
    gap: 12px;
    padding: 18px 20px;
    box-shadow: var(--shadow);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  header h2 {
    flex: 1;
    font-size: 16px;
  }
  .spacer {
    flex: 1;
  }
  .list {
    overflow: auto;
    border: 1px solid var(--line);
    background: var(--inset);
    min-height: 120px;
  }
  .group {
    padding: 8px 10px 4px;
    border-top: 1px solid var(--line);
    font-size: 11px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    cursor: pointer;
    border-left: 3px solid transparent;
  }
  .row:hover {
    background: var(--panel-2);
  }
  .row.sel {
    border-left-color: var(--amber);
    background: var(--amber-wash);
  }
  .nm {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    font-size: 11px;
    color: var(--muted);
    border: 1px solid var(--line-strong);
    padding: 0 6px;
    border-radius: 2px;
    white-space: nowrap;
  }
  .none {
    padding: 14px;
    margin: 0;
  }
  .opts {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 8px 12px;
    align-items: center;
  }
  .note {
    margin: 0;
    font-size: 12.5px;
  }
  .err {
    margin: 0;
    color: var(--led-bad);
  }
</style>
