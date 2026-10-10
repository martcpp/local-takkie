// What the screens show, and how engine messages change it. Plain functions
// with no Svelte or Tauri in them, so they can be tested by themselves.

import type { EngineEvent, Peer, Snapshot } from './engine'

export type LogLine = { at: number; text: string }

export type AppState = {
  /** The latest engine state, or `null` before the first one arrives. */
  snapshot: Snapshot | null
  /** Why the engine isn't running, if it isn't. */
  problem: string | null
  /** Newest last. */
  log: LogLine[]
}

export const MAX_LOG = 100

export const initial = (): AppState => ({ snapshot: null, problem: null, log: [] })

export function withSnapshot(state: AppState, snapshot: Snapshot): AppState {
  return { ...state, snapshot, problem: null }
}

export function withProblem(state: AppState, problem: string): AppState {
  return { ...state, snapshot: null, problem }
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
