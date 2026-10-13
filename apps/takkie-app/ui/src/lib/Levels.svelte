<script lang="ts">
  import type { Snapshot } from './engine'
  import { meter, micWarning, nextStrength, outputLabel } from './state'
  import {
    headset,
    onPhone,
    screen,
    setAutoLevel,
    setBeeps,
    setClarity,
    setHeadsetButton,
    setKeepScreenOn,
    setMuted,
    setNoiseSuppression,
    setVolume,
  } from './store.svelte'

  let { snapshot }: { snapshot: Snapshot } = $props()

  const percent = $derived(Math.round(snapshot.volume * 100))
  const output = $derived(outputLabel(snapshot))
  const warning = $derived(micWarning(snapshot.micAdvice))
</script>

<section aria-labelledby="levels-title">
  <h2 id="levels-title">
    Sound {#if output}<span class="output">{output}</span>{/if}
  </h2>
  <div class="meters">
    <span>Mic</span>
    <meter class="mic" min="0" max="1" value={meter(snapshot.mic.peak)}></meter>
    <span>Speaker</span>
    <meter class="speaker" min="0" max="1" value={meter(snapshot.speaker.peak)}></meter>
  </div>
  {#if warning}<p class="warning" role="status">⚠️ {warning}</p>{/if}
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
    {#if onPhone}
      <button
        aria-pressed={screen.keptOn}
        class:on={screen.keptOn}
        onclick={() => setKeepScreenOn(!screen.keptOn)}
      >
        {screen.keptOn ? '☀️ Screen stays on' : '🌙 Screen can sleep'}
      </button>
      <button
        aria-pressed={headset.talks}
        class:on={headset.talks}
        title="Click the headset button to talk, and click again to stop"
        onclick={() => setHeadsetButton(!headset.talks)}
      >
        {headset.talks ? '🎧 Headset button talks' : '🎧 Headset button off'}
      </button>
    {/if}
    <span class="buffer">Buffer {snapshot.bufferMs} ms</span>
  </div>
  <div class="switches" role="group" aria-label="Your voice">
    <button
      class:on={snapshot.noiseSuppression !== 'off'}
      title="Removes background noise from your microphone"
      onclick={() => setNoiseSuppression(nextStrength(snapshot.noiseSuppression))}
    >
      🧹 Noise removal: {snapshot.noiseSuppression}
    </button>
    <button
      aria-pressed={snapshot.autoLevel}
      class:on={snapshot.autoLevel}
      title="Brings a quiet or loud voice to the same level"
      onclick={() => setAutoLevel(!snapshot.autoLevel)}
    >
      {snapshot.autoLevel ? '🎚️ Even volume on' : '🎚️ Even volume off'}
    </button>
    <button
      class:on={snapshot.clarity !== 'off'}
      title="Lifts the part of your voice that carries the words"
      onclick={() => setClarity(nextStrength(snapshot.clarity))}
    >
      ✨ Clarity: {snapshot.clarity}
    </button>
  </div>
  <details class="echo">
    <summary>Hearing an echo?</summary>
    <ul>
      <li>
        <strong>A voice sounds far away:</strong> the talker should hold the microphone close to
        their mouth, or use a headset.
      </li>
      <li>
        <strong>A voice twice, a moment apart:</strong> you are in the same room as the talker.
        Wear headphones, or mute that person in the list.
      </li>
    </ul>
  </details>
</section>

<style>
  .output {
    font-weight: 400;
    color: var(--muted);
    margin-left: 0.375rem;
  }

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

  .warning {
    margin: 0.375rem 0 0;
    color: var(--warning, #b45309);
    font-size: 0.875rem;
  }

  .echo {
    margin-top: 0.625rem;
    color: var(--muted);
    font-size: 0.875rem;
  }

  .echo summary {
    cursor: pointer;
    min-height: 2rem;
    display: flex;
    align-items: center;
  }

  .echo ul {
    margin: 0.25rem 0 0;
    padding-left: 1.125rem;
  }

  .echo li + li {
    margin-top: 0.375rem;
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
