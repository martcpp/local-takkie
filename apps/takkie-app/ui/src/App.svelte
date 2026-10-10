<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app'
  import { onMount } from 'svelte'

  import { app, connect } from './lib/store.svelte'

  const version = getVersion()

  onMount(() => {
    const connected = connect()
    return () => {
      void connected.then((stop) => stop())
    }
  })
</script>

<main>
  <h1>local-takkie</h1>
  {#await version then number}
    <p class="version">version {number}</p>
  {/await}
  {#if app.problem}
    <p class="error">{app.problem}</p>
  {:else if app.snapshot}
    <p>
      Channel {app.snapshot.channel}{app.snapshot.private ? ' (private)' : ''},
      {app.snapshot.peers.length} others on the network.
    </p>
  {:else}
    <p>Starting…</p>
  {/if}
  <ul>
    {#each app.log.slice(-5) as line (line.at + line.text)}
      <li>{line.text}</li>
    {/each}
  </ul>
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

  ul {
    margin: 0.5rem 0 0;
    padding: 0;
    list-style: none;
    color: var(--muted);
    font-size: 0.875rem;
  }

  .version {
    color: var(--muted);
  }

  .error {
    color: var(--danger);
  }
</style>
