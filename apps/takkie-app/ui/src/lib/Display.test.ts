import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe as suite, expect, test, vi } from 'vitest'

import Display from './Display.svelte'
import type { Snapshot } from './engine'
import { peer, snapshot } from './fixtures'
import { METER_SEGMENTS, NO_MIC } from './state'

let shown: ReturnType<typeof mount> | undefined

afterEach(() => {
  if (shown) {
    unmount(shown)
  }
  document.body.innerHTML = ''
})

function render(state: Snapshot, onLock = () => {}) {
  const target = document.body.appendChild(document.createElement('div'))
  shown = mount(Display, { target, props: { snapshot: state, onLock } })
  flushSync()
  const text = (selector: string) => target.querySelector(selector)?.textContent?.trim()
  return {
    target,
    tag: text('.state'),
    lock: text('.lock'),
    here: text('.tag:last-child'),
    line: text('.line'),
    warning: target.querySelector('.line svg') !== null,
    lit: target.querySelectorAll('.segments .on').length,
  }
}

const talking = (name: string, channel = 2) => peer(name, name, channel, { talking: true })

suite('the display shows every state of the main screen', () => {
  test('standby on an empty open channel', () => {
    expect(render(snapshot())).toMatchObject({
      tag: 'STANDBY',
      lock: 'OPEN',
      here: '0 HERE',
      line: 'Nobody else here yet',
      warning: false,
    })
  })

  test('standby on a locked channel with people', () => {
    const view = render({ ...snapshot([peer('01', 'pixel-7a'), peer('02', 'away', 5)]), private: true })
    expect(view).toMatchObject({ lock: 'LOCKED', here: '1 HERE', line: 'Channel 2 is free' })
    expect(view.target.querySelector('.number')?.textContent).toContain('02')
  })

  test('receiving', () => {
    expect(render(snapshot([talking('mira-laptop')]))).toMatchObject({
      tag: 'RX',
      line: 'mira-laptop is talking',
    })
  })

  test('transmitting, with the meter lit', () => {
    const view = render({ ...snapshot(), transmitting: true, channel: 4, mic: { rms: 0.1, peak: 1 } })
    expect(view).toMatchObject({ tag: 'TX', line: 'You are talking on channel 4', lit: METER_SEGMENTS })
    expect(view.target.querySelector('.number')?.textContent).toContain('04')
  })

  test('each problem is a warning with its own line', () => {
    expect(render({ ...snapshot(), network: false })).toMatchObject({
      line: 'No Wi-Fi or local network',
      warning: true,
    })
    expect(render({ ...snapshot(), inputDevice: null })).toMatchObject({ line: NO_MIC, warning: true })
    expect(render({ ...snapshot(), micAdvice: 'clipping' }).line).toContain('too loud')
    expect(render(snapshot([peer('01', 'dock-tablet', 2, { mismatch: true })]))).toMatchObject({
      line: 'dock-tablet uses a different passphrase',
      warning: true,
    })
  })

  test('the lock tag opens the passphrase editor', () => {
    const onLock = vi.fn()
    const view = render(snapshot(), onLock)
    view.target.querySelector<HTMLButtonElement>('.lock')?.click()
    expect(onLock).toHaveBeenCalledOnce()
  })

  test('the meter and channel can be read without seeing them', () => {
    const view = render({ ...snapshot(), mic: { rms: 0, peak: 0.1 } })
    const level = view.target.querySelector('[role="meter"]')
    expect(level?.getAttribute('aria-valuenow')).toBe(String(view.lit))
    expect(view.target.textContent).toContain('Channel 2')
  })
})
