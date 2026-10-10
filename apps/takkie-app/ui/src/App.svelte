<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app'
  import { invoke } from '@tauri-apps/api/core'

  type DeviceCount = { inputs: number; outputs: number }

  const version = getVersion()
  const devices = invoke<DeviceCount>('device_count')
</script>

<main>
  <h1>local-takkie</h1>
  {#await version then number}
    <p class="version">version {number}</p>
  {/await}
  {#await devices}
    <p>Looking for audio devices…</p>
  {:then found}
    <p>{found.inputs} microphones and {found.outputs} speakers found.</p>
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
