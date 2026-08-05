---
title: "time (web, native)"
description: "Clocks, calendar functions, and a suspending sleep."
---

`time` is clocks and calendar functions. Import it by bare name or declare it in the `host` field of `packages.json`. The native engine builds it in, see [the native engine](/reference/modules#the-native-engine).

The surface is `time`, `time_ns`, `monotonic`, `monotonic_ns`, `perf_counter`, `perf_counter_ns`, `sleep`, `gmtime`, `localtime`, `mktime`, `strftime`, `strptime`, `asctime`, `ctime`, `timezone`, `altzone`, `daylight`, `tzname`. `sleep` suspends the coroutine. `gmtime` and `localtime` return the nine fields as a JSON string in `struct_time` order, decode them with `json.loads`. `tm_wday` is Monday=0, `tm_yday` is 1-based, and `tm_isdst` is always -1. `time_ns` returns a decimal string, because epoch nanoseconds exceed what a JS number can hold. `timezone`, `altzone`, `daylight`, and `tzname` are calls, not constants, and `tzname` returns the IANA zone name.

`gmtime` takes epoch seconds and returns UTC fields, which read the same on every host:

```python
import json
import time

fields = json.loads(time.gmtime(0))
print(fields)
print(time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(0)))
```

```text Output
[1970, 1, 1, 0, 0, 0, 3, 1, -1]
1970-01-01 00:00:00
```

Known limitations: the native engine is always UTC. There is no timezone database, so `tzname()` is `"UTC"` there. CPU and POSIX clocks are out of scope on both runtimes.
