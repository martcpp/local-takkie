<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app'
  import { onMount } from 'svelte'

  import ChannelPicker from './lib/ChannelPicker.svelte'
  import EventLog from './lib/EventLog.svelte'
  import Levels from './lib/Levels.svelte'
  import PassphraseDialog from './lib/PassphraseDialog.svelte'
  import PeerList from './lib/PeerList.svelte'
  import PermissionGate from './lib/PermissionGate.svelte'
  import PttButton from './lib/PttButton.svelte'
  import { channelState, micGate } from './lib/state'
  import { app, connect, recheck } from './lib/store.svelte'

  const version = getVersion()
  let passphrase: PassphraseDialog | undefined = $state()

  onMount(() => {
    const connected = connect()
    return () => {
      void connected.then((stop) => stop())
    }
  })
</script>

<svelte:document
  onvisibilitychange={() => {
    if (!document.hidden) {
      void recheck()
    }
  }}
/>

<main>
  <header class="top">
    <h1>local-takkie</h1>
    {#if app.snapshot}
      {@const status = channelState(app.snapshot)}
      <p class="status {status.kind}" role="status">{status.text}</p>
    {/if}
  </header>

  {#if app.problem}
    <section class="problem" role="alert">
      <h2>The radio isn't running</h2>
      <p>{app.problem}</p>
    </section>
  {:else if app.snapshot}
    <ChannelPicker snapshot={app.snapshot} onLock={() => passphrase?.open()} />
    <PeerList snapshot={app.snapshot} />
    <PttButton snapshot={app.snapshot} />
    <Levels snapshot={app.snapshot} />
    <PassphraseDialog
      bind:this={passphrase}
      channel={app.snapshot.channel}
      isPrivate={app.snapshot.private}
    />
  {:else if app.access && micGate(app.access) !== 'ready'}
    <PermissionGate access={app.access} />
  {:else}
    <p class="starting">Starting…</p>
  {/if}

  <EventLog log={app.log} />

  <footer>
    {#await version then number}version {number}{/await}
  </footer>
</main>

<style>
  main {
    max-width: 34rem;
    min-height: 100vh;
    margin: 0 auto;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  h1 {
    margin: 0;
    font-size: 1.25rem;
  }

  .status {
    margin: 0;
    padding: 0.25rem 0.75rem;
    border-radius: 999px;
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--on-accent);
    background: var(--talking);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status.busy {
    background: var(--warning);
  }

  .status.transmitting {
    background: var(--danger);
  }

  .problem {
    padding: 0.75rem;
    border: 1px solid var(--danger);
    border-radius: 0.75rem;
  }

  .problem h2 {
    color: var(--danger);
  }

  .problem p,
  .starting {
    margin: 0;
  }

  footer {
    margin-top: auto;
    text-align: center;
    color: var(--muted);
    font-size: 0.75rem;
  }
</style>
