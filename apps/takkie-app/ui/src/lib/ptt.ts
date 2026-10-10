// When the SPACE key means "talk". Kept apart from the button so it can be
// tested without a browser.

export type KeyTarget = {
  tagName?: string
  isContentEditable?: boolean
  closest?: (selector: string) => unknown
}

const TYPING = new Set(['INPUT', 'TEXTAREA', 'SELECT'])

/**
 * SPACE talks anywhere in the window, except while typing or with a dialog
 * open. That includes when a button has focus: Enter still presses it.
 */
export function spaceTalks(code: string, target: KeyTarget | null): boolean {
  if (code !== 'Space') {
    return false
  }
  if (!target) {
    return true
  }
  const typing = TYPING.has(target.tagName ?? '') || target.isContentEditable === true
  const inDialog = Boolean(target.closest?.('dialog[open]'))
  return !typing && !inDialog
}

/** Why talking isn't possible right now, or `null` when it is. */
export function blocked(inputDevice: string | null): string | null {
  return inputDevice === null ? 'The microphone isn’t running' : null
}
