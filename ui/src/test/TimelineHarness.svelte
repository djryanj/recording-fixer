<script lang="ts">
  // Holds the zoom the way the editor does (bound to the timeline), so tests can change `clips`
  // without resetting the scale and can read the scale back.
  import Timeline from '../lib/Timeline.svelte';
  import type { TClip } from '../lib/timeline';

  interface Props {
    clips: TClip[];
    selectedId?: number | null;
    playheadSecs?: number;
    playing?: boolean;
    fadeInSecs?: number;
    fadeOutSecs?: number;
    onselect: (id: number | null) => void;
    onseek: (secs: number) => void;
    onchange: (clips: TClip[]) => void;
  }
  let { clips, selectedId = null, playheadSecs = 0, playing = false, fadeInSecs = 0, fadeOutSecs = 0, onselect, onseek, onchange }: Props = $props();
  let px = $state(1);
</script>

<Timeline {clips} {selectedId} {playheadSecs} {playing} {fadeInSecs} {fadeOutSecs} bind:pxPerSec={px} {onselect} {onseek} {onchange} />
<output data-testid="px">{px}</output>
