<script lang="ts">
  import type { LogLine } from './state'

  let { log }: { log: LogLine[] } = $props()

  const time = (at: number) =>
    new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  const newestFirst = $derived([...log].reverse())
</script>

<details>
  <summary>
    Events
    {#if log.length > 0}<span class="latest">{log[log.length - 1].text}</span>{/if}
  </summary>
  {#if log.length === 0}
    <p>Nothing has happened yet.</p>
  {:else}
    <ol>
      {#each newestFirst as line, index (index)}
        <li><time>{time(line.at)}</time> {line.text}</li>
      {/each}
    </ol>
  {/if}
</details>

<style>
  summary {
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .latest {
    margin-left: 0.5rem;
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
  }

  details[open] .latest {
    display: none;
  }

  ol {
    margin: 0.5rem 0 0;
    padding: 0;
    list-style: none;
    max-height: 12rem;
    overflow-y: auto;
    font-size: 0.875rem;
    display: grid;
    gap: 0.125rem;
  }

  time {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  p {
    margin: 0.5rem 0 0;
    color: var(--muted);
    font-size: 0.875rem;
  }
</style>
