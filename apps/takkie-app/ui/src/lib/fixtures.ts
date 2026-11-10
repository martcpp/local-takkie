import type { Peer, Snapshot } from './engine'

export const peer = (id: string, name: string, channel = 2, extra: Partial<Peer> = {}): Peer => ({
  id,
  name,
  channel,
  addr: '192.168.1.5:40000',
  talking: false,
  seenMs: 0,
  mismatch: false,
  nearby: false,
  muted: false,
  ...extra,
})

export const snapshot = (peers: Peer[] = []): Snapshot => ({
  id: '00000000000000aa',
  name: 'studio-pc',
  address: '192.168.1.24:50211',
  channel: 2,
  private: false,
  transmitting: false,
  muted: false,
  beeps: false,
  noiseSuppression: 'normal',
  autoLevel: true,
  clarity: 'normal',
  micAdvice: 'fine',
  muteNearby: true,
  volume: 1,
  mic: { rms: 0, peak: 0 },
  speaker: { rms: 0, peak: 0 },
  bufferMs: 0,
  inputDevice: 'Microphone',
  outputDevice: 'Speakers',
  network: true,
  route: null,
  peers,
})
