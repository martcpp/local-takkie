<script lang="ts">
  import type { Snapshot } from './engine'
  import { ordered } from './state'
  import { setPeerMuted } from './store.svelte'

  let { snapshot }: { snapshot: Snapshot } = $props()

  const peers = $derived(ordered(snapshot.peers, snapshot.channel))
  const here = $derived(peers.filter((peer) => peer.channel === snapshot.channel).length)
</script>

<section aria-labelledby="peers-title">
  <h2 id="peers-title">
    People <span class="count">{here} on your channel, {peers.length} in all</span>
  </h2>
  {#if peers.length === 0}
    <p class="empty">Nobody else yet. Others on this network appear here by themselves.</p>
  {:else}
    <ul>
      {#each peers as peer (peer.id)}
        <li class:elsewhere={peer.channel !== snapshot.channel}>
          <span class="dot" class:talking={peer.talking} aria-hidden="true"></span>
          <span class="name">{peer.name || peer.id}</span>
          {#if peer.talking}<span class="sr-only">is talking</span>{/if}
          {#if peer.mismatch}<span class="mark mismatch">other passphrase</span>{/if}
          <span class="channel">ch {peer.channel}</span>
          <button
            class="mute"
            class:on={peer.muted}
            aria-pressed={peer.muted}
            aria-label={`${peer.muted ? 'Unmute' : 'Mute'} ${peer.name || peer.id}`}
            title={peer.muted ? 'Muted for you. Click to hear them again.' : 'Mute for you only'}
            onclick={() => setPeerMuted(peer.id, !peer.muted)}
          >
            {peer.muted ? '🔇' : '🔈'}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .count {
    font-weight: 400;
    color: var(--muted);
    margin-left: 0.375rem;
  }

  .empty {
    margin: 0;
    color: var(--muted);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.25rem;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.375rem 0.5rem 0.375rem 0.625rem;
    border-radius: 0.5rem;
    background: var(--surface);
  }

  li.elsewhere {
    opacity: 0.55;
  }

  .dot {
    flex: none;
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 50%;
    border: 2px solid var(--muted);
  }

  .dot.talking {
    border-color: var(--talking);
    background: var(--talking);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--talking) 30%, transparent);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mark {
    flex: none;
    font-size: 0.75rem;
    padding: 0.0625rem 0.375rem;
    border-radius: 999px;
    border: 1px solid currentColor;
  }

  .mute {
    flex: none;
    padding: 0.125rem 0.5rem;
    background: transparent;
  }

  .mute.on {
    border-color: var(--warning);
  }

  .mismatch {
    color: var(--danger);
  }

  .channel {
    flex: none;
    color: var(--muted);
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
  }
</style>
