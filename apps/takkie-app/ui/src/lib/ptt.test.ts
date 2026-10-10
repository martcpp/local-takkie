import { expect, test } from 'vitest'

import { blocked, spaceTalks } from './ptt'

const element = (tagName: string, inDialog = false, isContentEditable = false) => ({
  tagName,
  isContentEditable,
  closest: (selector: string) => (inDialog && selector === 'dialog[open]' ? {} : null),
})

test('space talks on the page and on buttons', () => {
  expect(spaceTalks('Space', null)).toBe(true)
  expect(spaceTalks('Space', element('BODY'))).toBe(true)
  expect(spaceTalks('Space', element('BUTTON'))).toBe(true)
  expect(spaceTalks('Space', element('SUMMARY'))).toBe(true)
  expect(spaceTalks('Space', {})).toBe(true)
})

test('other keys never talk', () => {
  expect(spaceTalks('Enter', element('BODY'))).toBe(false)
  expect(spaceTalks('KeyM', null)).toBe(false)
})

test('space types while typing', () => {
  expect(spaceTalks('Space', element('INPUT'))).toBe(false)
  expect(spaceTalks('Space', element('TEXTAREA'))).toBe(false)
  expect(spaceTalks('Space', element('SELECT'))).toBe(false)
  expect(spaceTalks('Space', element('DIV', false, true))).toBe(false)
})

test('space does nothing to the radio while a dialog is open', () => {
  expect(spaceTalks('Space', element('BUTTON', true))).toBe(false)
  expect(spaceTalks('Space', element('INPUT', true))).toBe(false)
})

test('talking is blocked without a microphone', () => {
  expect(blocked(null)).toContain('microphone')
  expect(blocked('Headset (48000 Hz)')).toBeNull()
})
