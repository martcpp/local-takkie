<script lang="ts">
  import type { Snapshot } from './engine'
  import { METER_SEGMENTS, display } from './state'

  let { snapshot, onLock }: { snapshot: Snapshot; onLock: () => void } = $props()

  const TAGS = { standby: 'STANDBY', receiving: 'RX', transmitting: 'TX' }
  const segments = Array.from({ length: METER_SEGMENTS }, (_, index) => index)
  const shown = $derived(display(snapshot))
  const number = $derived(String(snapshot.channel).padStart(2, '0'))
</script>

<section class="lcd" aria-label="Display">
  <div class="channel">
    <span class="ch" aria-hidden="true">CH</span>
    <span class="number" aria-hidden="true">
      <span class="ghost">88</span>
      <span class="digits">{number}</span>
    </span>
    <span class="sr-only">Channel {snapshot.channel}</span>
  </div>

  <div class="tags">
    <span class="tag state">{TAGS[shown.state]}</span>
    <button
      class="tag lock"
      aria-label={snapshot.private ? 'Locked. Change the passphrase' : 'Open. Set a passphrase'}
      onclick={onLock}
    >
      {#if snapshot.private}
        <svg viewBox="0 0 12 12" aria-hidden="true">
          <rect x="2.5" y="5.5" width="7" height="5" rx="1" />
          <path d="M4 5.5V4a2 2 0 0 1 4 0v1.5" />
        </svg>
      {/if}
      {snapshot.private ? 'LOCKED' : 'OPEN'}
    </button>
    <span class="tag">{shown.here} HERE</span>
  </div>

  <p class="line" role="status">
    {#if shown.warning}
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M8 1.75 15 14.25H1Z" />
        <path d="M8 6.5v3.5M8 12v.25" />
      </svg>
    {/if}
    {shown.line}
  </p>

  <div class="mic">
    <span class="mic-label" aria-hidden="true">MIC</span>
    <span
      class="segments"
      role="meter"
      aria-label="Microphone level"
      aria-valuemin={0}
      aria-valuemax={METER_SEGMENTS}
      aria-valuenow={shown.lit}
    >
      {#each segments as index (index)}
        <span class:on={index < shown.lit}></span>
      {/each}
    </span>
  </div>
</section>

<style>
  .lcd {
    --faint: color-mix(in srgb, var(--lcd-ink) 12%, var(--lcd));

    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-areas:
      'channel tags'
      'channel line'
      'channel mic';
    align-items: center;
    column-gap: 2rem;
    padding: 1.25rem 1.5rem;
    color: var(--lcd-ink);
    background: var(--lcd);
    border: 2px solid color-mix(in srgb, var(--lcd-ink) 18%, var(--lcd));
    border-radius: 0.375rem;
    box-shadow: inset 0 2px 0 color-mix(in srgb, var(--lcd-ink) 8%, transparent);
  }

  .channel {
    grid-area: channel;
    display: flex;
    align-items: flex-end;
    gap: 0.625rem;
  }

  .ch {
    margin-bottom: 0.75rem;
    font-family: var(--font-mono);
    font-size: 1.125rem;
    font-weight: 600;
  }

  .number {
    position: relative;
    font-family: var(--font-mono);
    font-size: 6.5rem;
    font-weight: 800;
    line-height: 0.8;
    letter-spacing: -0.02em;
  }

  .ghost {
    position: absolute;
    inset: 0 auto auto 0;
    transform: translateX(0.14em);
    color: color-mix(in srgb, var(--lcd-ink) 7%, var(--lcd));
  }

  .digits {
    position: relative;
  }

  .tags {
    grid-area: tags;
    display: flex;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    min-height: 0;
    padding: 0.25rem 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.875rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 1.2;
    color: inherit;
    background: transparent;
    border: 1.5px solid var(--lcd-ink);
    border-radius: 3px;
  }

  .tag.state {
    color: var(--lcd);
    background: var(--lcd-ink);
  }

  .lock {
    position: relative;
    min-width: 0;
    cursor: pointer;
  }

  /* A finger-sized target around a small tag. */
  .lock::after {
    content: '';
    position: absolute;
    inset: -0.625rem -0.375rem;
  }

  .lock:focus-visible {
    outline-color: var(--lcd-ink);
  }

  .tag svg {
    width: 0.8125rem;
    height: 0.8125rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.3;
  }

  .line {
    grid-area: line;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    margin: 0.5rem 0;
    font-size: 1.625rem;
    font-weight: 650;
    line-height: 1.15;
    text-align: right;
  }

  .line svg {
    flex: none;
    width: 1.25rem;
    height: 1.25rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .mic {
    grid-area: mic;
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .mic-label {
    font-family: var(--font-mono);
    font-size: 0.8125rem;
    font-weight: 600;
  }

  .segments {
    flex: 1;
    display: flex;
    gap: 3px;
  }

  .segments span {
    flex: 1;
    height: 0.875rem;
    border-radius: 1px;
    background: var(--faint);
  }

  .segments .on {
    background: var(--lcd-ink);
  }

  /* The phone design: tags stacked beside the number, the line under it. */
  @media (max-width: 47.99rem) {
    .lcd {
      grid-template-columns: 1fr auto;
      grid-template-areas:
        'channel tags'
        'line line'
        'mic mic';
      padding: 1.25rem 1.25rem 1.125rem;
      border-radius: 0.75rem;
    }

    .number {
      font-size: 6rem;
    }

    .tags {
      flex-direction: column;
      align-items: flex-end;
      align-self: start;
    }

    .line {
      justify-content: flex-start;
      margin: 1rem 0 0.875rem;
      font-size: 1.5rem;
      text-align: left;
    }
  }
</style>
