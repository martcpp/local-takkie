// The one place screens read engine state from and change it through. It
// follows the engine by itself, so no screen has to poll.

import * as engine from './engine'
import {
  SHORT_PASSPHRASE,
  initial,
  withEvent,
  withNote,
  withProblem,
  withSnapshot,
  type AppState,
} from './state'

let state = $state<AppState>(initial())

// This session's passphrases, by channel. Kept in memory only.
const secrets = new Map<number, string>()

export const app = {
  get snapshot() {
    return state.snapshot
  },
  get problem() {
    return state.problem
  },
  get log() {
    return state.log
  },
}

function note(text: string) {
  state = withNote(state, text, Date.now())
}

async function ask(action: Promise<void>) {
  try {
    await action
  } catch (problem) {
    note(String(problem))
  }
}

/** Starts following the engine. Call the result to stop. */
export async function connect(): Promise<() => void> {
  const stops = await Promise.all([
    engine.onSnapshot((snapshot) => {
      state = withSnapshot(state, snapshot)
    }),
    engine.onEvent((event) => {
      state = withEvent(state, event, Date.now())
    }),
  ])
  // Snapshots only flow while the engine runs, so ask once to learn why not.
  try {
    state = withSnapshot(state, await engine.snapshot())
  } catch (problem) {
    state = withProblem(state, String(problem))
  }
  return () => stops.forEach((stop) => stop())
}

/** Joins `channel`, with the passphrase it had earlier in this session. */
export async function switchChannel(channel: number) {
  if (channel === state.snapshot?.channel) {
    return
  }
  const secret = secrets.get(channel)
  note(`Channel ${channel}${secret ? ' (private)' : ''}`)
  await ask(engine.setChannel(channel, secret))
}

/** Makes the current channel private, or open again with an empty passphrase. */
export async function setPassphrase(passphrase: string) {
  const channel = state.snapshot?.channel
  if (channel === undefined) {
    return
  }
  if (passphrase) {
    secrets.set(channel, passphrase)
    if ([...passphrase].length < SHORT_PASSPHRASE) {
      note('That passphrase is short and easy to guess, a few words are safer')
    }
    note(`Channel ${channel} is now private`)
  } else {
    secrets.delete(channel)
    note(`Channel ${channel} is open again`)
  }
  await ask(engine.setChannel(channel, passphrase))
}

export const setMuted = (muted: boolean) => ask(engine.setMuted(muted))
export const setVolume = (volume: number) => ask(engine.setVolume(volume))
export const setBeeps = (on: boolean) => ask(engine.setBeeps(on))
