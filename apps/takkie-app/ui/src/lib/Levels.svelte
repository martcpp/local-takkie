<script lang="ts">
  import type { Snapshot } from './engine'
  import { meter } from './state'
  import { setBeeps, setMuted, setVolume } from './store.svelte'

  let { snapshot }: { snapshot: Snapshot } = $props()

  const percent = $derived(Math.round(snapshot.volume * 100))
</script>

<section aria-labelledby="levels-title">
  <h2 id="levels-title">Sound</h2>
  <div class="meters">
    <span>Mic</span>
    <meter class="mic" min="0" max="1" value={meter(snapshot.mic.peak)}></meter>
    <span>Speaker</span>
    <meter class="speaker" min="0" max="1" value={meter(snapshot.speaker.peak)}></meter>
  </div>
  <label class="volume">
    <span>Volume</span>
    <input
      type="range"
      min="0"
      max="200"
      step="10"
      value={percent}
      oninput={(event) => setVolume(event.currentTarget.valueAsNumber / 100)}
    />
    <output>{percent}%</output>
  </label>
  <div class="switches">
    <button aria-pressed={snapshot.muted} class:on={snapshot.muted} onclick={() => setMuted(!snapshot.muted)}>
      {snapshot.muted ? '🔇 Muted' : '🔊 Sound on'}
    </button>
    <button aria-pressed={snapshot.beeps} class:on={snapshot.beeps} onclick={() => setBeeps(!snapshot.beeps)}>
      {snapshot.beeps ? '🔔 Beeps on' : '🔕 Beeps off'}
    </button>
    <span class="buffer">Buffer {snapshot.bufferMs} ms</span>
  </div>
</section>

<style>
  .meters {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 0.375rem 0.75rem;
  }

  meter {
    width: 100%;
    height: 0.75rem;
  }

  .volume {
    display: grid;
    grid-template-columns: auto 1fr 3.25rem;
    align-items: center;
    gap: 0.75rem;
    margin-top: 0.625rem;
  }

  .volume input {
    width: 100%;
    min-height: 2rem;
    accent-color: var(--accent);
  }

  output {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .switches {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.625rem;
  }

  .switches button.on {
    border-color: var(--accent);
    color: var(--accent);
  }

  .buffer {
    margin-left: auto;
    color: var(--muted);
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
  }
</style>
