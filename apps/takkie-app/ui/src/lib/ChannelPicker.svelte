<script lang="ts">
  import type { Snapshot } from './engine'
  import { switchChannel } from './store.svelte'

  let { snapshot, onLock }: { snapshot: Snapshot; onLock: () => void } = $props()

  const channels = Array.from({ length: 10 }, (_, index) => index + 1)
  const here = (channel: number) =>
    snapshot.peers.filter((peer) => peer.channel === channel).length
</script>

<section aria-labelledby="channels-title">
  <header>
    <h2 id="channels-title">Channel</h2>
    <button class="lock" class:private={snapshot.private} onclick={onLock}>
      {snapshot.private ? '🔒 Private' : '🔓 Open'}
    </button>
  </header>
  <div class="grid" role="group" aria-label="Channels 1 to 10">
    {#each channels as channel (channel)}
      {@const current = channel === snapshot.channel}
      <button
        class="channel"
        class:current
        aria-pressed={current}
        aria-label={`Channel ${channel}, ${here(channel)} others`}
        onclick={() => switchChannel(channel)}
      >
        <span class="number">{channel}</span>
        <span class="count">{here(channel) || ''}</span>
      </button>
    {/each}
  </div>
</section>

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 0.375rem;
  }

  .channel {
    position: relative;
    min-width: 0;
    min-height: 2.75rem;
    font-size: 1.125rem;
    font-weight: 600;
  }

  .channel.current {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }

  .count {
    position: absolute;
    top: 0.125rem;
    right: 0.375rem;
    font-size: 0.6875rem;
    font-weight: 500;
    opacity: 0.75;
  }

  .lock {
    font-size: 0.875rem;
    padding: 0.25rem 0.625rem;
  }

  .lock.private {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
