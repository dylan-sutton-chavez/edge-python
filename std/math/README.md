# Edge Python Math

`math` package for [Edge Python](https://edgepython.com), compiled to `wasm32-unknown-unknown` over the [WASM module ABI](https://edgepython.com/reference/wasm-abi). Scalar transcendentals run on `libm` (no platform libc), and a packed-f64 batch path keeps bulk work fast by crossing the host boundary once per call instead of once per element.

```python
from math import sqrt, pi, factorial, hypot

print(sqrt(2)) # 1.4142135623730951
print(pi) # 3.141592653589793  (a value, not a call)
print(factorial(5)) # 120
print(hypot(3, 4, 12)) # 13.0
```

## Surface

| Group | Names |
|-------|-------|
| Constants | `pi`, `e`, `tau`, `inf`, `nan` |
| Power / log | `sqrt`, `cbrt`, `exp`, `exp2`, `expm1`, `pow`, `log` (optional base), `log2`, `log10`, `log1p` |
| Trigonometric | `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `hypot` (variadic), `dist` |
| Hyperbolic | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` |
| Angular | `degrees`, `radians` |
| Special | `erf`, `erfc`, `gamma`, `lgamma` |
| Float ops | `fabs`, `fmod`, `remainder`, `copysign`, `ldexp`, `modf`, `frexp`, `floor`, `ceil`, `trunc` |
| Classification | `isnan`, `isinf`, `isfinite` |
| Reductions | `fsum`, `prod` |
| Integer | `factorial`, `gcd` (variadic), `lcm` (variadic), `isqrt`, `comb`, `perm` |

`floor`, `ceil`, and `trunc` return an `int`; `modf` and `frexp` return a tuple. Out-of-domain inputs raise `ValueError("math domain error")`, matching CPython.

## Batch fast path

For array workloads, the `*_all` functions take and return `bytes` holding little-endian f64 values. The whole buffer crosses the boundary once, so `n` elements cost two crossings instead of `2n`.

```python
from math import sqrt_all, fsum_all, dot_all
from struct import pack

buf = pack("4d", 1.0, 4.0, 9.0, 16.0)
roots = sqrt_all(buf) # bytes -> bytes, element-wise sqrt
print(fsum_all(buf)) # 30.0, compensated sum over the buffer
print(dot_all(buf, buf)) # 354.0, compensated dot product
```

Element-wise (`bytes -> bytes`): `sqrt_all`, `abs_all`, `exp_all`, `log_all`, `sin_all`, `cos_all`; same-length pairs: `add_all`, `sub_all`, `mul_all`; scalar broadcast: `scale_all(buf, k)`. Reductions (`bytes -> float`): `fsum_all`, `prod_all`, `dot_all`. Row-major matrix times vector: `matvec(m, x, cols)`. A buffer length that is not a multiple of 8, or mismatched operand lengths, raises `ValueError`.

Together with [`struct`](../struct) this covers small numeric training loops: Python orchestrates, the kernels run native, `pack`/`unpack` move and persist the weights.

## Limitations

These come from the Edge Python VM, not this package:

- Integers cap at `i128`. `factorial`, `comb`, `perm`, and `lcm` raise `ValueError` when a result exceeds that range.
- No complex numbers, so there is no `cmath`-style surface.
- `prod` returns a `float`; integer inputs lose their int-ness.

## Build

```bash
cargo build --release --target wasm32-unknown-unknown
```

License: MIT OR Apache-2.0
