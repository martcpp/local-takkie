// What the screens show, and how engine messages change it. Plain functions
// with no Svelte or Tauri in them, so they can be tested by themselves.

import type { Access, EngineEvent, MicAdvice, Peer, Route, Snapshot, Strength } from './engine'

export type LogLine = { at: number; text: string }

export type AppState = {
  /** The latest engine state, or `null` before the first one arrives. */
  snapshot: Snapshot | null
  /** Why the engine isn't running, if it isn't. */
  problem: string | null
  /** What the user has allowed, or `null` before we have asked the system. */
  access: Access | null
  /** Newest last. */
  log: LogLine[]
}

export const MAX_LOG = 100

export const initial = (): AppState => ({ snapshot: null, problem: null, access: null, log: [] })

/** Takes the newest snapshot, and logs it when the network comes or goes. */
export function withSnapshot(state: AppState, snapshot: Snapshot, at = 0): AppState {
  const next = { ...state, snapshot, problem: null }
  const was = state.snapshot?.network ?? true
  if (was === snapshot.network) {
    return next
  }
  return withNote(next, snapshot.network ? 'Back on the network' : NO_NETWORK, at)
}

/** What to show while there is no network for other takkies to be on. */
export const NO_NETWORK = 'No Wi-Fi or local network. Others can’t be found until it is back.'

export function withProblem(state: AppState, problem: string): AppState {
  return { ...state, snapshot: null, problem }
}

export function withAccess(state: AppState, access: Access): AppState {
  return { ...state, access }
}

/**
 * What to do about the microphone: nothing, show the system prompt, or send
 * the user to the settings because the system won't prompt again.
 */
export function micGate(access: Access): 'ready' | 'ask' | 'settings' {
  switch (access.microphone) {
    case 'granted':
      return 'ready'
    case 'denied':
      return 'settings'
    default:
      return 'ask'
  }
}

/** A line of our own in the log, such as a command that failed. */
export function withNote(state: AppState, text: string, at: number): AppState {
  return { ...state, log: [...state.log, { at, text }].slice(-MAX_LOG) }
}

/** The peer's name, or its id while it has none. */
export function label(peers: Peer[], id: string): string {
  return peers.find((peer) => peer.id === id)?.name || id
}

/** One line for the event log, or `null` for events that aren't worth one. */
export function describe(event: EngineEvent, peers: Peer[]): string | null {
  switch (event.type) {
    case 'peerJoined':
      return `${event.peer.name || event.peer.id} joined (channel ${event.peer.channel})`
    case 'peerUpdated':
      return `${event.peer.name || event.peer.id} is on channel ${event.peer.channel}`
    case 'peerLeft':
      return `${label(peers, event.id)} left`
    case 'talkStarted':
    case 'talkStopped':
      return null
    case 'wrongPassphrase':
      return `${label(peers, event.id)} uses another passphrase. You can't hear each other.`
    case 'deviceStarted':
      return `${event.direction === 'input' ? 'Microphone' : 'Speaker'}: ${event.description}`
    case 'deviceLost':
      return `${event.direction === 'input' ? 'Microphone' : 'Speaker'} lost, retrying`
    case 'warning':
      return event.text
  }
}

function peersAfter(peers: Peer[], event: EngineEvent): Peer[] {
  const patch = (id: string, change: Partial<Peer>) =>
    peers.map((peer) => (peer.id === id ? { ...peer, ...change } : peer))
  switch (event.type) {
    case 'peerJoined':
    case 'peerUpdated':
      return [...peers.filter((peer) => peer.id !== event.peer.id), event.peer].sort((a, b) =>
        a.id.localeCompare(b.id),
      )
    case 'peerLeft':
      return peers.filter((peer) => peer.id !== event.id)
    case 'talkStarted':
      return patch(event.id, { talking: true })
    case 'talkStopped':
      return patch(event.id, { talking: false })
    case 'wrongPassphrase':
      return patch(event.id, { mismatch: true })
    default:
      return peers
  }
}

/**
 * Applies an event right away, so the screen doesn't wait for the next
 * snapshot, and adds its line to the log.
 */
export function withEvent(state: AppState, event: EngineEvent, at: number): AppState {
  const before = state.snapshot?.peers ?? []
  const text = describe(event, before)
  const log = text === null ? state.log : [...state.log, { at, text }].slice(-MAX_LOG)
  const snapshot = state.snapshot && { ...state.snapshot, peers: peersAfter(before, event) }
  return { ...state, snapshot, log }
}

export type ChannelState = { kind: 'free' | 'busy' | 'transmitting'; text: string }

/** Whether the channel is free, busy or ours, and who is talking. */
export function channelState(snapshot: Snapshot): ChannelState {
  const talkers = ordered(snapshot.peers, snapshot.channel)
    .filter((peer) => peer.talking && peer.channel === snapshot.channel)
    .map((peer) => peer.name || peer.id)
  const who =
    talkers.length === 0
      ? null
      : talkers.length === 1
        ? `${talkers[0]} is talking`
        : `${talkers[0]} and ${talkers.length - 1} more are talking`
  if (snapshot.transmitting) {
    return { kind: 'transmitting', text: who ? `Transmitting (${who} too)` : 'Transmitting' }
  }
  return who ? { kind: 'busy', text: `Busy: ${who}` } : { kind: 'free', text: 'Free' }
}

/** A peak level as a meter position from 0 to 1, on a 60 dB scale. */
export function meter(peak: number): number {
  if (!(peak > 0.001)) {
    return 0
  }
  return Math.min(1, Math.max(0, (20 * Math.log10(peak) + 60) / 60))
}

const ROUTES: Record<Route, string> = {
  speaker: 'Phone speaker',
  wired: 'Wired headset',
  bluetooth: 'Bluetooth',
}

/**
 * Where sound comes out, in words. A phone reports its route; a computer
 * names the device it opened, without the stream settings in brackets.
 */
const STRENGTHS: Strength[] = ['off', 'normal', 'strong']

/** The strength after `now`, for a button that steps through them. */
export function nextStrength(now: Strength): Strength {
  return STRENGTHS[(STRENGTHS.indexOf(now) + 1) % STRENGTHS.length]
}

/** A remembered strength, or `null` for anything else that was stored. */
export function asStrength(stored: string | null): Strength | null {
  return STRENGTHS.find((strength) => strength === stored) ?? null
}

/** What to tell the person about their microphone, if anything. */
export function micWarning(advice: MicAdvice): string | null {
  switch (advice) {
    case 'clipping':
      return 'Microphone too loud: move back a little'
    case 'quiet':
      return 'Microphone too quiet: move closer'
    default:
      return null
  }
}

export function outputLabel(snapshot: Snapshot): string | null {
  if (snapshot.route) {
    return ROUTES[snapshot.route]
  }
  return snapshot.outputDevice?.replace(/\s*\([^()]*\)\s*$/, '') || null
}

/** Passphrases shorter than this get a warning. */
export const SHORT_PASSPHRASE = 12

/** Your channel first, then by name, nameless ones last. */
export function ordered(peers: Peer[], channel: number): Peer[] {
  const rank = (peer: Peer) => [peer.channel === channel ? 0 : 1, peer.name ? 0 : 1] as const
  return [...peers].sort((a, b) => {
    const [aChannel, aNamed] = rank(a)
    const [bChannel, bNamed] = rank(b)
    return (
      aChannel - bChannel ||
      aNamed - bNamed ||
      a.name.toLowerCase().localeCompare(b.name.toLowerCase()) ||
      a.id.localeCompare(b.id)
    )
  })
}
