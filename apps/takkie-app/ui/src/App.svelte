<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app'
  import { invoke } from '@tauri-apps/api/core'

  type Snapshot = { channel: number; private: boolean; peers: unknown[] }
  type Devices = { inputs: unknown[]; outputs: unknown[] }

  const version = getVersion()
  const snapshot = invoke<Snapshot>('snapshot')
  const devices = invoke<Devices>('list_devices')
</script>

<main>
  <h1>local-takkie</h1>
  {#await version then number}
    <p class="version">version {number}</p>
  {/await}
  {#await snapshot}
    <p>Starting…</p>
  {:then now}
    <p>
      Channel {now.channel}{now.private ? ' (private)' : ''}, {now.peers.length} others on the
      network.
    </p>
  {:catch error}
    <p class="error">{error}</p>
  {/await}
  {#await devices then found}
    <p>{found.inputs.length} microphones and {found.outputs.length} speakers found.</p>
  {:catch error}
    <p class="error">Audio devices can't be listed: {error}</p>
  {/await}
</main>

<style>
  main {
    min-height: 100vh;
    display: grid;
    place-content: center;
    gap: 0.5rem;
    text-align: center;
    padding: 1rem;
  }

  h1 {
    margin: 0;
    font-size: 2rem;
  }

  p {
    margin: 0;
  }

  .version {
    color: var(--muted);
  }

  .error {
    color: var(--danger);
  }
</style>
