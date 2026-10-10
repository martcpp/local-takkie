<script lang="ts">
  import type { Access } from './engine'
  import { micGate } from './state'
  import { allowMicrophone, openAppSettings } from './store.svelte'

  let { access }: { access: Access } = $props()

  const gate = $derived(micGate(access))
  let asking = $state(false)

  async function allow() {
    asking = true
    await allowMicrophone()
    asking = false
  }
</script>

<section class="gate" aria-labelledby="gate-title">
  <h2 id="gate-title">Allow the microphone</h2>
  <p>
    local-takkie needs the microphone so others can hear you. Your voice goes only to the people
    on your channel, on this network. Nothing is recorded.
  </p>
  {#if gate === 'settings'}
    <p class="refused">
      The microphone is turned off for this app. Turn it on under Permissions in the app's
      settings, then come back.
    </p>
    <button class="primary" onclick={() => openAppSettings()}>Open settings</button>
  {:else}
    <button class="primary" disabled={asking} onclick={allow}>Allow microphone</button>
  {/if}
</section>

<style>
  .gate {
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.75rem;
    background: var(--surface);
    display: grid;
    gap: 0.75rem;
  }

  h2 {
    margin: 0;
    font-size: 1.0625rem;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text);
  }

  p {
    margin: 0;
  }

  .refused {
    color: var(--warning);
  }

  .primary {
    min-height: 3rem;
    font-weight: 600;
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
</style>
