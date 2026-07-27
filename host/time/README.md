# Edge Python time

Wall and monotonic clocks, sleep, and calendar formatting shipped as a plain ESM module, a subset of CPython's `time`. Register it with `createWorker({ mainThreadModules: { time } })` (or the `host` field of `packages.json`); see [`host/README.md`](../README.md) for the setup boilerplate.

```python
from time import time, sleep, strftime, gmtime

print(time()) # float seconds since the Unix epoch
sleep(0.1) # yields, resumes after 100ms
print(strftime("%Y-%m-%d %H:%M:%S", gmtime(0))) # 1970-01-01 00:00:00
```

## Testing

```bash
deno run -A npm:playwright install chromium # one-time
HOSTCAP=time deno test --allow-all tests/ # from repo root
```

See [`tests/README.md`](../tests/README.md) for the corpus shape.

## API

**Conventions:**

- Clock reads are sync. They return numbers, no `await` and no `receive()`.
- `sleep(secs)` yields, so it composes with the concurrency builtins (`gather`, `with_timeout`, `run`) exactly like `fetch`.
- Nanosecond counts that exceed JS's safe-integer range cross as strings. `time_ns()` returns a numeric string, parse with `int()`. `monotonic_ns()` and `perf_counter_ns()` fit and return integers.
- A `struct_time` is a JSON nine-tuple string in CPython order: `[tm_year, tm_mon, tm_mday, tm_hour, tm_min, tm_sec, tm_wday, tm_yday, tm_isdst]`. `gmtime`, `localtime`, `strptime` produce it; `strftime`, `asctime`, `mktime` consume it. Read fields by decoding it with [`json`](https://github.com/dylan-sutton-chavez/edge-python/tree/main/std/json).
- `tm_wday` is Monday=0 through Sunday=6, `tm_yday` is 1-based, `tm_isdst` is always -1 (unknown).

### Clocks

`time`, `time_ns`, `monotonic`, `monotonic_ns`, `perf_counter`, `perf_counter_ns`.

```python
print(time()) # float seconds since the epoch, UTC wall clock
print(int(time_ns())) # nanoseconds since the epoch (crosses as a string)
start = perf_counter() # highest-resolution timer, for benchmarking a block
print(monotonic() >= 0) # seconds, never steps backward
```

In a browser `monotonic` and `perf_counter` share one source (`performance.now()`); reach for `perf_counter` when timing a block, `monotonic` for general elapsed-time checks.

### Sleep

`sleep` yields, so it composes with the scheduler:

```python
sleep(0.05)

async def slow():
    sleep(10)
    return "never"
try:
    with_timeout(0.1, slow()) # deadline enforced by the scheduler
except TimeoutError:
    print("timed out")
```

### Calendar

`gmtime([secs])`, `localtime([secs])`, `mktime(t)`. Omitting `secs` uses the current time. `gmtime` is UTC, `localtime` is the host's local zone, `mktime` is the inverse of `localtime`.

```python
t = loads(gmtime(0)) # decode the struct_time tuple
print(t[0], t[1], t[2]) # 1970 1 1
print(mktime(localtime(1000000)) == 1000000) # round-trips
```

### Formatting

`strftime(fmt[, t])`, `strptime(s, fmt)`, `asctime([t])`, `ctime([secs])`. `strftime`/`asctime` default `t` to local now; `ctime` defaults `secs` to now.

```python
print(strftime("%a %b %d", gmtime(0))) # Thu Jan 01
print(asctime(gmtime(0))) # Thu Jan  1 00:00:00 1970
print(strptime("2021-06-15", "%Y-%m-%d"))
```

Supported directives: `%Y %y %m %d %H %M %S %I %p %j %w %a %A %b %B %%`. Any literal that is not a directive passes through unchanged.

### Timezone

`timezone`, `altzone`, `daylight`, `tzname`. CPython exposes these as module constants; here they are zero-argument calls because host modules export callables, not values. `tzname` returns the IANA zone name (e.g. `"America/Bogota"`) rather than CPython's `(std, dst)` tuple.

```python
print(timezone()) # seconds west of UTC, standard (non-DST) offset
print(daylight()) # 1 if the zone observes DST, else 0
```

## Not supported

By design, a sandboxed WASM runtime has no process, thread, or POSIX clock to expose:

- CPU and thread clocks: `process_time(_ns)`, `thread_time(_ns)`.
- POSIX clock syscalls and ids: `clock_gettime/settime/getres` (and `_ns`), `pthread_getcpuclockid`, the `CLOCK_*` constants.
- `tzset` (no environment; the zone comes from `Intl`).
- `struct_time` as a named tuple. Tuples cross as JSON strings, so `t[0]` after `json.loads` works but `t.tm_year` does not.

## How it works

`src/index.js` is a factory `() => handlers`. Two slices in `src/` merge with `Object.assign`. `clock.js` returns the sync reads (`Date.now`, `performance.now`, `Intl`) plus the yielding `sleep`. `fmt.js` returns the pure calendar helpers, building struct_time tuples and formatting through `Date` without shipping locale tables, on CPython's C locale names.

## License

Apache-2.0
