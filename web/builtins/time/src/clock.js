/* Clock reads (sync) plus sleep, which yields so the worker parks the coro on the Promise, like fetch. */

// Timezone is the standard (non-DST) offset, so sample both halves of the year and take the larger.
const janOff = () => new Date(new Date().getFullYear(), 0, 1).getTimezoneOffset();
const julOff = () => new Date(new Date().getFullYear(), 6, 1).getTimezoneOffset();

export default () => ({
    time: () => Date.now() / 1000,
    time_ns: () => (BigInt(Date.now()) * 1_000_000n).toString(), // string: epoch ns overflows Number
    monotonic: () => performance.now() / 1000,
    monotonic_ns: () => Math.round(performance.now() * 1e6),
    perf_counter: () => performance.now() / 1000,
    perf_counter_ns: () => Math.round(performance.now() * 1e6),
    sleep: (secs) => new Promise((resolve) => setTimeout(resolve, secs * 1000)),
    // Constants, exposed as callables since host modules export functions, not values.
    timezone: () => Math.max(janOff(), julOff()) * 60,
    altzone: () => Math.min(janOff(), julOff()) * 60,
    daylight: () => (janOff() !== julOff() ? 1 : 0),
    tzname: () => Intl.DateTimeFormat().resolvedOptions().timeZone,
});
