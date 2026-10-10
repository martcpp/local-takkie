// The app's Rust side, as the frontend sees it. The shapes mirror
// `src-tauri/src/view.rs` and the commands mirror `commands.rs`.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type Level = { rms: number; peak: number }

export type Peer = {
  /** Hex text: a 64-bit id doesn't fit a JavaScript number. */
  id: string
  name: string
  channel: number
  addr: string
  talking: boolean
  seenMs: number
  mismatch: boolean
  muted: boolean
}

export type Snapshot = {
  id: string
  channel: number
  private: boolean
  transmitting: boolean
  muted: boolean
  beeps: boolean
  volume: number
  mic: Level
  speaker: Level
  bufferMs: number
  /** What is running, or `null` when that side isn't. */
  inputDevice: string | null
  outputDevice: string | null
  peers: Peer[]
}

export type Direction = 'input' | 'output'

export type EngineEvent =
  | { type: 'peerJoined'; peer: Peer }
  | { type: 'peerUpdated'; peer: Peer }
  | { type: 'peerLeft'; id: string }
  | { type: 'talkStarted'; id: string }
  | { type: 'talkStopped'; id: string }
  | { type: 'wrongPassphrase'; id: string }
  | { type: 'deviceStarted'; direction: Direction; description: string }
  | { type: 'deviceLost'; direction: Direction }
  | { type: 'warning'; text: string }

export type Device = { name: string; isDefault: boolean }
export type Devices = { inputs: Device[]; outputs: Device[] }

export type Grant = 'granted' | 'denied' | 'prompt' | 'prompt-with-rationale'

/** Runtime permissions. On a computer both are always granted. */
export type Access = { microphone: Grant; notifications: Grant }

export const permissions = () => invoke<Access>('permissions')
export const requestPermissions = () => invoke<Access>('request_permissions')
export const openAppSettings = () => invoke<void>('open_app_settings')
/** Does nothing if the engine already runs. */
export const startRadio = () => invoke<void>('start_radio')

export const snapshot = () => invoke<Snapshot>('snapshot')
export const listDevices = () => invoke<Devices>('list_devices')
export const setTransmitting = (on: boolean) => invoke<void>('set_transmitting', { on })
export const setMuted = (muted: boolean) => invoke<void>('set_muted', { muted })
export const setVolume = (volume: number) => invoke<void>('set_volume', { volume })
export const setBeeps = (on: boolean) => invoke<void>('set_beeps', { on })
export const setPeerMuted = (peer: string, muted: boolean) =>
  invoke<void>('set_peer_muted', { peer, muted })

/** An empty or missing passphrase makes the channel open. */
export const setChannel = (channel: number, passphrase?: string) =>
  invoke<void>('set_channel', { channel, passphrase: passphrase || null })

export const onSnapshot = (handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> =>
  listen<Snapshot>('engine-snapshot', (message) => handler(message.payload))

export const onEvent = (handler: (event: EngineEvent) => void): Promise<UnlistenFn> =>
  listen<EngineEvent>('engine-event', (message) => handler(message.payload))
