import { describe as suite, expect, test } from 'vitest'

import type { Access, Peer, Snapshot } from './engine'
import {
  MAX_LOG,
  NO_NETWORK,
  asStrength,
  channelState,
  describe,
  initial,
  label,
  meter,
  micGate,
  micWarning,
  nextStrength,
  ordered,
  outputLabel,
  withAccess,
  withEvent,
  withNote,
  withProblem,
  withSnapshot,
} from './state'

const peer = (id: string, name: string, channel = 2, extra: Partial<Peer> = {}): Peer => ({
  id,
  name,
  channel,
  addr: '192.168.1.5:40000',
  talking: false,
  seenMs: 0,
  mismatch: false,
  muted: false,
  ...extra,
})

const snapshot = (peers: Peer[] = []): Snapshot => ({
  id: '00000000000000aa',
  channel: 2,
  private: false,
  transmitting: false,
  muted: false,
  beeps: false,
  noiseSuppression: 'normal',
  autoLevel: true,
  clarity: 'normal',
  micAdvice: 'fine',
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

const running = (peers: Peer[] = []) => withSnapshot(initial(), snapshot(peers))

suite('snapshots and problems', () => {
  test('nothing is known at the start', () => {
    expect(initial()).toEqual({ snapshot: null, problem: null, access: null, log: [] })
  })

  test('a snapshot replaces the last one and clears a problem', () => {
    const broken = withProblem(initial(), 'the port is taken')
    expect(broken.problem).toBe('the port is taken')
    const fixed = withSnapshot(broken, snapshot([peer('01', 'Kitchen')]))
    expect(fixed.problem).toBeNull()
    expect(fixed.snapshot?.peers).toHaveLength(1)
  })

  test('a problem hides the stale snapshot and keeps the log', () => {
    const state = withEvent(running(), { type: 'warning', text: 'careful' }, 5)
    const broken = withProblem(state, 'stopped')
    expect(broken.snapshot).toBeNull()
    expect(broken.log).toEqual([{ at: 5, text: 'careful' }])
  })
})

suite('the network coming and going', () => {
  const offline = { ...snapshot(), network: false }

  test('losing it is logged once, not on every snapshot', () => {
    let state = withSnapshot(running(), offline, 5)
    expect(state.log).toEqual([{ at: 5, text: NO_NETWORK }])
    state = withSnapshot(state, offline, 6)
    expect(state.log).toHaveLength(1)
  })

  test('getting it back is logged too', () => {
    const state = withSnapshot(withSnapshot(running(), offline, 5), snapshot(), 9)
    expect(state.log.map((line) => line.text)).toEqual([NO_NETWORK, 'Back on the network'])
    expect(state.log[1].at).toBe(9)
  })

  test('starting without a network says so, starting with one says nothing', () => {
    expect(withSnapshot(initial(), offline, 1).log).toHaveLength(1)
    expect(withSnapshot(initial(), snapshot(), 1).log).toEqual([])
  })
})

suite('events change the peer list at once', () => {
  test('a peer joins, talks, stops and leaves', () => {
    const kitchen = peer('02', 'Kitchen')
    let state = withEvent(running([peer('03', 'Attic')]), { type: 'peerJoined', peer: kitchen }, 1)
    expect(state.snapshot?.peers.map((p) => p.id)).toEqual(['02', '03'])

    state = withEvent(state, { type: 'talkStarted', id: '02' }, 2)
    expect(state.snapshot?.peers[0].talking).toBe(true)
    expect(state.snapshot?.peers[1].talking).toBe(false)

    state = withEvent(state, { type: 'talkStopped', id: '02' }, 3)
    expect(state.snapshot?.peers[0].talking).toBe(false)

    state = withEvent(state, { type: 'peerLeft', id: '02' }, 4)
    expect(state.snapshot?.peers.map((p) => p.id)).toEqual(['03'])
    expect(state.log.map((line) => line.text)).toEqual(['Kitchen joined (channel 2)', 'Kitchen left'])
  })

  test('an update replaces the peer instead of adding a second one', () => {
    const state = withEvent(
      running([peer('02', 'Kitchen', 2)]),
      { type: 'peerUpdated', peer: peer('02', 'Kitchen', 5) },
      1,
    )
    expect(state.snapshot?.peers).toHaveLength(1)
    expect(state.snapshot?.peers[0].channel).toBe(5)
    expect(state.log[0].text).toBe('Kitchen is on channel 5')
  })

  test('a wrong passphrase marks the peer and explains', () => {
    const state = withEvent(running([peer('02', 'Garage')]), { type: 'wrongPassphrase', id: '02' }, 1)
    expect(state.snapshot?.peers[0].mismatch).toBe(true)
    expect(state.log[0].text).toContain('Garage uses another passphrase')
  })

  test('events before the first snapshot are logged and nothing breaks', () => {
    const state = withEvent(initial(), { type: 'peerLeft', id: '09' }, 1)
    expect(state.snapshot).toBeNull()
    expect(state.log).toEqual([{ at: 1, text: '09 left' }])
  })

  test('the earlier state is not changed', () => {
    const before = running([peer('02', 'Kitchen')])
    withEvent(before, { type: 'talkStarted', id: '02' }, 1)
    expect(before.snapshot?.peers[0].talking).toBe(false)
    expect(before.log).toEqual([])
  })
})

suite('the event log', () => {
  test('each kind of event reads naturally', () => {
    const peers = [peer('02', 'Kitchen'), peer('03', '')]
    expect(describe({ type: 'peerJoined', peer: peers[1] }, peers)).toBe('03 joined (channel 2)')
    expect(describe({ type: 'talkStarted', id: '02' }, peers)).toBeNull()
    expect(describe({ type: 'talkStopped', id: '02' }, peers)).toBeNull()
    expect(
      describe({ type: 'deviceStarted', direction: 'input', description: 'Headset (48000 Hz)' }, peers),
    ).toBe('Microphone: Headset (48000 Hz)')
    expect(describe({ type: 'deviceLost', direction: 'output' }, peers)).toBe('Speaker lost, retrying')
    expect(describe({ type: 'warning', text: 'no such microphone' }, peers)).toBe('no such microphone')
  })

  test('a peer is called by name, or by id while it has none', () => {
    const peers = [peer('02', 'Kitchen'), peer('03', '')]
    expect(label(peers, '02')).toBe('Kitchen')
    expect(label(peers, '03')).toBe('03')
    expect(label(peers, '99')).toBe('99')
  })

  test('only the newest lines are kept', () => {
    let state = running()
    for (let n = 0; n < MAX_LOG + 5; n += 1) {
      state = withEvent(state, { type: 'warning', text: `line ${n}` }, n)
    }
    expect(state.log).toHaveLength(MAX_LOG)
    expect(state.log[0].text).toBe('line 5')
    expect(state.log.at(-1)?.text).toBe(`line ${MAX_LOG + 4}`)
  })
})

suite('microphone permission', () => {
  const access = (microphone: Access['microphone']): Access => ({
    microphone,
    notifications: 'granted',
  })

  test('granted is ready, whatever notifications say', () => {
    expect(micGate(access('granted'))).toBe('ready')
    expect(micGate({ microphone: 'granted', notifications: 'denied' })).toBe('ready')
  })

  test('not asked yet, or refused once, can be asked', () => {
    expect(micGate(access('prompt'))).toBe('ask')
    expect(micGate(access('prompt-with-rationale'))).toBe('ask')
  })

  test('refused for good needs the settings', () => {
    expect(micGate(access('denied'))).toBe('settings')
  })

  test('what is allowed is remembered next to everything else', () => {
    const state = withAccess(running([peer('01', 'Kitchen')]), access('prompt'))
    expect(state.access?.microphone).toBe('prompt')
    expect(state.snapshot?.peers).toHaveLength(1)
  })
})

suite('channel state', () => {
  const talking = (id: string, name: string, channel = 2) => peer(id, name, channel, { talking: true })

  test('free when nobody on the channel talks', () => {
    const quiet = snapshot([peer('01', 'Bedroom'), talking('02', 'Office', 5)])
    expect(channelState(quiet)).toEqual({ kind: 'free', text: 'Free' })
  })

  test('busy names who is talking', () => {
    expect(channelState(snapshot([talking('01', 'Bedroom')]))).toEqual({
      kind: 'busy',
      text: 'Busy: Bedroom is talking',
    })
    const crowd = snapshot([talking('01', 'Bedroom'), talking('03', 'Attic'), talking('04', '')])
    expect(channelState(crowd).text).toBe('Busy: Attic and 2 more are talking')
  })

  test('transmitting wins, and says who else talks', () => {
    const alone = { ...snapshot(), transmitting: true }
    expect(channelState(alone)).toEqual({ kind: 'transmitting', text: 'Transmitting' })
    const both = { ...snapshot([talking('01', 'Bedroom')]), transmitting: true }
    expect(channelState(both).text).toBe('Transmitting (Bedroom is talking too)')
  })
})

suite('meters and notes', () => {
  test('meters use a decibel scale', () => {
    expect(meter(0)).toBe(0)
    expect(meter(0.0005)).toBe(0)
    expect(meter(Number.NaN)).toBe(0)
    expect(meter(1)).toBe(1)
    expect(meter(4)).toBe(1)
    expect(meter(0.1)).toBeCloseTo(40 / 60)
    expect(meter(0.01)).toBeCloseTo(20 / 60)
  })

  test('a note is added to the log like an event line', () => {
    const state = withNote(running(), 'Channel 3', 7)
    expect(state.log).toEqual([{ at: 7, text: 'Channel 3' }])
    expect(state.snapshot).not.toBeNull()
  })
})

suite('where sound comes out', () => {
  test('a phone names its route', () => {
    expect(outputLabel({ ...snapshot(), route: 'speaker' })).toBe('Phone speaker')
    expect(outputLabel({ ...snapshot(), route: 'wired' })).toBe('Wired headset')
    expect(outputLabel({ ...snapshot(), route: 'bluetooth' })).toBe('Bluetooth')
  })

  test('a computer names its device, without the stream settings', () => {
    const device = { ...snapshot(), outputDevice: 'Headphones (48000 Hz, 2 ch, f32)' }
    expect(outputLabel(device)).toBe('Headphones')
    const brackets = { ...snapshot(), outputDevice: 'Speakers (Realtek) (44100 Hz, 2 ch, i16)' }
    expect(outputLabel(brackets)).toBe('Speakers (Realtek)')
  })

  test('nothing is shown while no speaker runs', () => {
    expect(outputLabel({ ...snapshot(), outputDevice: null })).toBeNull()
  })
})

suite('peer order', () => {
  test('your channel first, then names, nameless last', () => {
    const peers = [
      peer('01', 'zed', 2),
      peer('02', 'Amy', 5),
      peer('04', '', 2),
      peer('03', 'bob', 2),
    ]
    expect(ordered(peers, 2).map((p) => p.name)).toEqual(['bob', 'zed', '', 'Amy'])
    expect(peers[0].name).toBe('zed')
  })
})

suite('voice processing', () => {
  test('a strength button steps off, normal, strong and round again', () => {
    expect(nextStrength('off')).toBe('normal')
    expect(nextStrength('normal')).toBe('strong')
    expect(nextStrength('strong')).toBe('off')
  })

  test('only a real strength is taken from storage', () => {
    expect(asStrength('strong')).toBe('strong')
    expect(asStrength('loud')).toBeNull()
    expect(asStrength(null)).toBeNull()
  })

  test('the microphone warning says what to do', () => {
    expect(micWarning('fine')).toBeNull()
    expect(micWarning('clipping')).toContain('move back')
    expect(micWarning('quiet')).toContain('move closer')
  })
})
