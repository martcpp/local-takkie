<script lang="ts">
  import type { Snapshot } from './engine'
  import { blocked, spaceTalks } from './ptt'
  import { setTransmitting } from './store.svelte'

  let { snapshot }: { snapshot: Snapshot } = $props()

  // What we asked for, so the button reacts before the next snapshot.
  let held = $state(false)
  const reason = $derived(blocked(snapshot.inputDevice))
  const live = $derived(held || snapshot.transmitting)

  function talk(on: boolean) {
    if (on && reason) {
      return
    }
    if (on !== held) {
      held = on
      void setTransmitting(on)
    }
  }

  function press(event: PointerEvent) {
    if (event.button !== 0) {
      return
    }
    // Keeps the release coming here even if the pointer slides off.
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    talk(true)
  }

  function keydown(event: KeyboardEvent) {
    if (!spaceTalks(event.code, event.target as HTMLElement | null)) {
      return
    }
    event.preventDefault()
    if (!event.repeat) {
      talk(true)
    }
  }

  function keyup(event: KeyboardEvent) {
    if (event.code === 'Space' && held) {
      event.preventDefault()
      talk(false)
    }
  }

  $effect(() => {
    if (reason && held) {
      talk(false)
    }
  })

  $effect(() => () => {
    if (held) {
      void setTransmitting(false)
    }
  })
</script>

<svelte:window onkeydown={keydown} onkeyup={keyup} onblur={() => talk(false)} />
<svelte:document
  onvisibilitychange={() => {
    if (document.hidden) {
      talk(false)
    }
  }}
/>

<button
  class="ptt"
  class:live
  disabled={reason !== null}
  aria-pressed={live}
  onpointerdown={press}
  onpointerup={() => talk(false)}
  onpointercancel={() => talk(false)}
  onlostpointercapture={() => talk(false)}
  oncontextmenu={(event) => event.preventDefault()}
>
  <span class="label">{live ? 'Talking' : 'Hold to talk'}</span>
  {#if reason}
    <span class="hint">{reason}</span>
  {:else if live}
    <span class="hint">Release to stop</span>
  {:else}
    <span class="hint keyboard">or hold SPACE</span>
  {/if}
</button>

<style>
  .ptt {
    width: 100%;
    min-height: 7rem;
    display: grid;
    place-content: center;
    gap: 0.25rem;
    border-radius: 1.25rem;
    border: 2px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
  }

  .ptt.live {
    background: var(--danger);
    border-color: var(--danger);
    color: var(--on-accent);
  }

  .ptt:disabled {
    border-color: var(--border);
    background: var(--surface);
  }

  .label {
    font-size: 1.5rem;
    font-weight: 700;
  }

  .hint {
    font-size: 0.875rem;
    opacity: 0.8;
  }

  /* A phone has no SPACE key to hold. */
  @media (hover: none) and (pointer: coarse) {
    .keyboard {
      display: none;
    }
  }

  /* Landscape on a phone: leave room for the list above. */
  @media (max-height: 30rem) {
    .ptt {
      min-height: 4rem;
    }
  }
</style>
