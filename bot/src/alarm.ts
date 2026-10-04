import { TICK_MS } from './config'

// A healthy alarm is never more than a few ticks away, so one further off on either side was stranded and is armed again.
export const stale = (at: number | null, now: number) => at === null || Math.abs(at - now) > TICK_MS * 6
