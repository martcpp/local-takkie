<script lang="ts">
  import { SHORT_PASSPHRASE } from './state'
  import { setPassphrase } from './store.svelte'

  let { channel, isPrivate }: { channel: number; isPrivate: boolean } = $props()

  let dialog: HTMLDialogElement
  let passphrase = $state('')
  const short = $derived(passphrase.length > 0 && [...passphrase].length < SHORT_PASSPHRASE)

  export function open() {
    passphrase = ''
    dialog.showModal()
    // An entry for the system back gesture to undo, so it closes the dialog
    // and doesn't leave the app.
    history.pushState({ dialog: true }, '')
  }

  function closed() {
    passphrase = ''
    if (history.state?.dialog) {
      history.back()
    }
  }

  function apply(secret: string) {
    void setPassphrase(secret)
    passphrase = ''
    dialog.close()
  }
</script>

<svelte:window
  onpopstate={() => {
    if (dialog.open) {
      dialog.close()
    }
  }}
/>

<dialog bind:this={dialog} aria-labelledby="passphrase-title" onclose={closed}>
  <form
    onsubmit={(event) => {
      event.preventDefault()
      apply(passphrase)
    }}
  >
    <h2 id="passphrase-title">Passphrase for channel {channel}</h2>
    <p>
      Everyone who should hear you sets the same passphrase on this channel. It is never saved.
    </p>
    <input
      type="password"
      autocomplete="off"
      maxlength="128"
      placeholder="A few random words"
      aria-label="Passphrase"
      bind:value={passphrase}
    />
    <p class="hint" class:warn={short}>
      {short ? 'Short passphrases are easy to guess. A few words are safer.' : ' '}
    </p>
    <div class="actions">
      {#if isPrivate}
        <button type="button" onclick={() => apply('')}>Make it open</button>
      {/if}
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary" disabled={passphrase.length === 0}>Set</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(24rem, calc(100vw - 2rem));
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.75rem;
    background: var(--bg);
    color: var(--text);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
  }

  h2 {
    margin: 0 0 0.5rem;
    font-size: 1.0625rem;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text);
  }

  p {
    margin: 0 0 0.75rem;
    color: var(--muted);
    font-size: 0.875rem;
  }

  input {
    width: 100%;
    padding: 0.5rem 0.625rem;
    font: inherit;
    color: inherit;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
  }

  .hint {
    min-height: 1.25rem;
    margin: 0.375rem 0 0.5rem;
  }

  .hint.warn {
    color: var(--warning);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
</style>
