// Dev only: `npm run dev`, then /preview.html?scene=rx. The real screens on a
// pretend engine, to look at them without Rust, a microphone or a network.

import { emit } from '@tauri-apps/api/event'
import { mockIPC } from '@tauri-apps/api/mocks'

import type { Peer, Snapshot, Strength } from './lib/engine'

const peer = (id: string, name: string, channel: number, change: Partial<Peer> = {}): Peer => ({
  id,
  name,
  channel,
  addr: `192.168.1.${parseInt(id, 16) % 200}:50211`,
  talking: false,
  seenMs: 400,
  mismatch: false,
  nearby: false,
  muted: false,
  ...change,
})

const everyone = [
  peer('a1', 'mira-laptop', 4, { talking: true }),
  peer('b2', 'pixel-7a', 4),
  peer('c3', 'dock-tablet', 4, { mismatch: true }),
  peer('d4', 'kitchen-mac', 1),
  peer('e5', 'field-phone', 7),
  peer('f6', 'warehouse-2', 7),
]

const base: Snapshot = {
  id: '00ff',
  channel: 4,
  private: true,
  transmitting: false,
  muted: false,
  beeps: false,
  noiseSuppression: 'normal',
  autoLevel: true,
  clarity: 'normal',
  micAdvice: 'fine',
  muteNearby: true,
  volume: 0.8,
  mic: { rms: 0, peak: 0 },
  speaker: { rms: 0, peak: 0 },
  bufferMs: 60,
  inputDevice: 'Built-in Microphone',
  outputDevice: 'Built-in Speakers',
  network: true,
  route: null,
  peers: everyone,
}

const scenes: Record<string, Partial<Snapshot>> = {
  rx: {},
  tx: { transmitting: true, peers: everyone.map((one) => ({ ...one, talking: false })) },
  standby: { channel: 2, private: false },
  alone: { channel: 2, private: false, peers: [] },
  offline: { network: false, peers: [] },
  nomic: { inputDevice: null },
  loud: { micAdvice: 'clipping' },
  room: { peers: everyone.map((one) => (one.id === 'b2' ? { ...one, nearby: true } : one)) },
}

const scene = new URLSearchParams(location.search).get('scene') ?? 'rx'
let now: Snapshot = { ...base, ...scenes[scene] }

function change(next: Partial<Snapshot>) {
  now = { ...now, ...next }
}

type Args = Record<string, unknown>

const commands: Record<string, (args: Args) => unknown> = {
  'plugin:app|version': () => '0.2.0-preview',
  permissions: () => ({ microphone: 'granted', notifications: 'granted' }),
  request_permissions: () => ({ microphone: 'granted', notifications: 'granted' }),
  snapshot: () => now,
  list_devices: () => ({
    inputs: [{ name: 'Built-in Microphone', isDefault: true }],
    outputs: [{ name: 'Built-in Speakers', isDefault: true }],
  }),
  set_transmitting: ({ on }) => change({ transmitting: Boolean(on) }),
  set_channel: ({ channel, passphrase }) =>
    change({ channel: Number(channel), private: Boolean(passphrase) }),
  set_muted: ({ muted }) => change({ muted: Boolean(muted) }),
  set_volume: ({ volume }) => change({ volume: Number(volume) }),
  set_beeps: ({ on }) => change({ beeps: Boolean(on) }),
  set_noise_suppression: ({ level }) => change({ noiseSuppression: level as Strength }),
  set_auto_level: ({ on }) => change({ autoLevel: Boolean(on) }),
  set_clarity: ({ level }) => change({ clarity: level as Strength }),
  set_mute_nearby: ({ on }) => change({ muteNearby: Boolean(on) }),
  set_peer_muted: ({ peer: id, muted }) =>
    change({ peers: now.peers.map((one) => (one.id === id ? { ...one, muted: Boolean(muted) } : one)) }),
}

mockIPC(
  (command, payload) => {
    const handle = commands[command]
    return handle ? handle((payload ?? {}) as Args) : null
  },
  { shouldMockEvents: true },
)

let tick = 0
setInterval(() => {
  tick += 1
  const voice = now.transmitting ? 0.25 + 0.2 * Math.abs(Math.sin(tick / 3)) : 0.01
  void emit('engine-snapshot', { ...now, mic: { rms: voice / 3, peak: voice } })
}, 100)

void import('./main')
