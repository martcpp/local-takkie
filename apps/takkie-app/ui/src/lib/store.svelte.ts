// The one place screens read engine state from and change it through. It
// follows the engine by itself, so no screen has to poll.

import * as engine from './engine'
import {
  SHORT_PASSPHRASE,
  initial,
  micGate,
  withAccess,
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
  get access() {
    return state.access
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
      state = withSnapshot(state, snapshot, Date.now())
    }),
    engine.onEvent((event) => {
      state = withEvent(state, event, Date.now())
    }),
    engine.onStopped((why) => {
      state = withProblem(state, why)
    }),
  ])
  if (keepsScreenOn) {
    await ask(engine.setKeepScreenOn(true))
  }
  if (!headsetTalks) {
    await ask(engine.setHeadsetButton(false))
  }
  await begin(engine.permissions())
  return () => stops.forEach((stop) => stop())
}

// Starts the radio if the microphone is allowed. Snapshots only flow while
// the engine runs, so one is asked for here to learn why it doesn't.
async function begin(asked: Promise<engine.Access>) {
  try {
    state = withAccess(state, await asked)
    if (state.access && micGate(state.access) === 'ready') {
      await engine.startRadio()
      state = withSnapshot(state, await engine.snapshot(), Date.now())
    }
  } catch (problem) {
    state = withProblem(state, String(problem))
  }
}

/** Shows the system's permission prompt, and starts the radio if allowed. */
export const allowMicrophone = () => begin(engine.requestPermissions())

/**
 * Tells the app whether the window is on screen. Coming back, it shows the
 * current state at once: a fresh snapshot, the reason the radio stopped
 * meanwhile, or the radio starting because the microphone is now allowed.
 */
export async function shown(visible: boolean) {
  await ask(engine.setOnScreen(visible))
  if (!visible) {
    return
  }
  if (state.access && micGate(state.access) !== 'ready') {
    await begin(engine.permissions())
  } else if (state.snapshot) {
    try {
      state = withSnapshot(state, await engine.snapshot(), Date.now())
    } catch (problem) {
      state = withProblem(state, String(problem))
    }
  }
}

export const openAppSettings = () => ask(engine.openAppSettings())

const KEEP_SCREEN_ON = 'keep-screen-on'

/** Phone-only options are shown when this is true. */
export const onPhone = /Android/i.test(navigator.userAgent)

let keepsScreenOn = $state(localStorage.getItem(KEEP_SCREEN_ON) === 'yes')

export const screen = {
  get keptOn() {
    return keepsScreenOn
  },
}

const HEADSET_BUTTON = 'headset-button'

// On unless turned off: it is what most people with a headset want.
let headsetTalks = $state(localStorage.getItem(HEADSET_BUTTON) !== 'no')

export const headset = {
  get talks() {
    return headsetTalks
  },
}

/**
 * Lets the headset button talk while it is held, or gives it back to
 * whatever plays music. Remembered.
 */
export async function setHeadsetButton(on: boolean) {
  headsetTalks = on
  localStorage.setItem(HEADSET_BUTTON, on ? 'yes' : 'no')
  await ask(engine.setHeadsetButton(on))
}

/** Keeps the screen from sleeping while the app is in front, and remembers it. */
export async function setKeepScreenOn(on: boolean) {
  keepsScreenOn = on
  localStorage.setItem(KEEP_SCREEN_ON, on ? 'yes' : 'no')
  await ask(engine.setKeepScreenOn(on))
}

/** Starts the radio again after it was turned off or failed to start. */
export const turnOn = () => begin(engine.permissions())

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

export const setTransmitting = (on: boolean) => ask(engine.setTransmitting(on))
export const setPeerMuted = (peer: string, muted: boolean) => ask(engine.setPeerMuted(peer, muted))
export const setMuted = (muted: boolean) => ask(engine.setMuted(muted))
export const setVolume = (volume: number) => ask(engine.setVolume(volume))
export const setBeeps = (on: boolean) => ask(engine.setBeeps(on))
