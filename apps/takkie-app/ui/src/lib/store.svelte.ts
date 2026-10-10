// The one place screens read engine state from. It follows the engine by
// itself, so no screen has to poll.

import * as engine from './engine'
import { initial, withEvent, withProblem, withSnapshot, type AppState } from './state'

let state = $state<AppState>(initial())

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
