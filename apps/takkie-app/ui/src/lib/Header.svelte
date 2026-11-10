<script lang="ts">
  import type { Snapshot } from './engine'
  import { lamps } from './state'

  let { snapshot, onSettings }: { snapshot: Snapshot | null; onSettings: () => void } = $props()

  const lit = $derived(snapshot ? lamps(snapshot) : { tx: false, rx: false })
</script>

<header>
  <h1 class="label">Takkie</h1>
  <span class="lamps">
    <span class="lamp tx" class:on={lit.tx}>TX<span class="sr-only">{lit.tx ? ' on' : ' off'}</span></span>
    <span class="lamp rx" class:on={lit.rx}>RX<span class="sr-only">{lit.rx ? ' on' : ' off'}</span></span>
  </span>
  {#if snapshot}
    <dl>
      <div>
        <dt class="label">Station</dt>
        <dd class="mono">{snapshot.name}</dd>
      </div>
      <div>
        <dt class="label">Address</dt>
        {#if snapshot.address}
          <dd class="mono">{snapshot.address}</dd>
        {:else}
          <dd class="offline">No network</dd>
        {/if}
      </div>
    </dl>
  {/if}
  <button class="settings" aria-label="Settings" onclick={onSettings}>
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M3 6h8M16 6h1M3 14h1M8 14h9" />
      <circle cx="13.5" cy="6" r="2.25" />
      <circle cx="6" cy="14" r="2.25" />
    </svg>
    <span class="text">Settings</span>
  </button>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 1.5rem;
    min-height: 4.5rem;
    padding: env(safe-area-inset-top) max(1.875rem, env(safe-area-inset-right)) 0
      max(1.875rem, env(safe-area-inset-left));
    background: var(--bar);
    border-bottom: 1px solid var(--edge);
  }

  h1 {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 800;
    line-height: 1;
    letter-spacing: 0.01em;
    color: var(--text);
  }

  .lamps {
    display: flex;
    gap: 0.875rem;
  }

  .lamp {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--muted);
  }

  .lamp::before {
    content: '';
    width: 0.625rem;
    height: 0.625rem;
    border-radius: 2px;
    background: var(--line);
  }

  .tx.on::before {
    background: var(--signal);
  }

  .rx.on::before {
    background: var(--receive);
  }

  dl {
    display: flex;
    gap: 1.75rem;
    margin: 0 0 0 auto;
    min-width: 0;
  }

  dl div {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    min-width: 0;
  }

  dt {
    font-size: 0.6875rem;
    color: var(--muted);
  }

  dd {
    margin: 0;
    font-size: 0.9375rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .offline {
    color: var(--caution);
  }

  .settings {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-height: 2.75rem;
    padding: 0 1.125rem;
    font-weight: 600;
    background: transparent;
    border-color: var(--line);
    border-radius: 0.375rem;
  }

  .lamps + .settings {
    margin-left: auto;
  }

  svg {
    width: 1.125rem;
    height: 1.125rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }

  /* The phone design. */
  @media (max-width: 47.99rem) {
    header {
      gap: 1rem;
      padding-top: calc(1rem + env(safe-area-inset-top));
      padding-left: max(1rem, env(safe-area-inset-left));
      padding-right: max(1rem, env(safe-area-inset-right));
      background: transparent;
      border-bottom: none;
    }

    h1 {
      font-size: 1.875rem;
    }

    dl,
    .text {
      display: none;
    }

    .settings {
      margin-left: auto;
      width: 3.5rem;
      height: 3.5rem;
      padding: 0;
      justify-content: center;
      border-radius: 0.75rem;
    }

    svg {
      width: 1.375rem;
      height: 1.375rem;
    }
  }
</style>
