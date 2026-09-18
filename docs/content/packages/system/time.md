---
title: "time (js, cli)"
description: "Clocks, calendar functions, and a suspending sleep."
---

`time` is clocks and calendar functions. Declare it with `edge add time`, a `system` entry, and import it by bare name. It runs in the JS host and in the CLI, which builds it in and resolves the entry by name, see [The CLI](/reference/modules#the-cli).

The surface is `time`, `time_ns`, `monotonic`, `monotonic_ns`, `perf_counter`, `perf_counter_ns`, `sleep`, `gmtime`, `localtime`, `mktime`, `strftime`, `strptime`, `asctime`, `ctime`, `timezone`, `altzone`, `daylight`, `tzname`. `sleep` suspends the coroutine. `gmtime` and `localtime` return the nine fields as a JSON string in `struct_time` order, decode them with `json.loads`. `tm_wday` is Monday=0, `tm_yday` is 1-based, and `tm_isdst` is always -1. `time_ns` returns an int on both hosts. `timezone`, `altzone`, `daylight`, and `tzname` are calls, not constants, and `tzname` returns the IANA zone name.

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

Known limitations: the CLI is always UTC. There is no timezone database, so `tzname()` is `"UTC"` there and `localtime` equals `gmtime`. CPU and POSIX clocks are out of scope on both hosts.
