---
title: "math (web, native)"
description: "Scalar math plus a batch path over bytes buffers."
---

`math` is scalar math on `libm`, with `ValueError: math domain error` for domain errors. Import it by bare name, both runtimes resolve it with no manifest. To pin a different version, import it by URL or through a `packages.json` alias, see [Modules](/reference/modules#packagesjson).

Module constants are `pi`, `e`, `tau`, `inf`, `nan` (values, not calls). Integer helpers are `factorial`, `gcd`, `lcm`, `isqrt`, `comb`, `perm`, bounded by the VM's 128-bit integers. `hypot` and `gcd` are variadic. `modf` and `frexp` return tuples, and `floor`, `ceil`, and `trunc` return `int`. The rest of the scalar surface, by group:

- Power and log: `sqrt`, `cbrt`, `exp`, `exp2`, `expm1`, `pow`, `log` (optional base), `log2`, `log10`, `log1p`.
- Trigonometric: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `hypot`, `dist`.
- Hyperbolic: `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`.
- Angular and special: `degrees`, `radians`, `erf`, `erfc`, `gamma`, `lgamma`.
- Float ops and classification: `fabs`, `fmod`, `remainder`, `copysign`, `ldexp`, `modf`, `frexp`, `floor`, `ceil`, `trunc`, `isnan`, `isinf`, `isfinite`.
- Reductions: `fsum`, `prod` (returns `float`, even for integer inputs).

There is no complex-number surface. A batch path processes a whole `bytes` buffer of little-endian f64 values in one call, and pairs with [struct](/packages/std/struct) for numeric work. Element-wise: `sqrt_all`, `abs_all`, `exp_all`, `log_all`, `sin_all`, `cos_all`. Same-length pairs: `add_all`, `sub_all`, `mul_all`. Scalar broadcast: `scale_all(buf, k)`. Reductions to a float: `fsum_all`, `prod_all`, `dot_all`. Row-major matrix times vector: `matvec(m, x, cols)`. A buffer length that is not a multiple of 8, or mismatched operand lengths, raises `ValueError`.

```python
from math import sqrt, pi, hypot, factorial

print(sqrt(2))
print(pi)
print(hypot(3, 4, 12))
print(factorial(5))
```

```text Output
1.4142135623730951
3.141592653589793
13.0
120
```

The integer helpers return `int`:

```python
from math import gcd, lcm, isqrt, comb, perm

print(gcd(12, 18, 24))
print(lcm(4, 6))
print(isqrt(99))
print(comb(5, 2), perm(5, 2))
```

```text Output
6
12
9
10 20
```

`modf` and `frexp` decompose a float into tuples, and `floor`, `ceil`, and `trunc` return `int`:

```python
from math import modf, frexp, floor, ceil, trunc

print(modf(3.75))
print(frexp(12.0))
print(floor(2.9), ceil(2.1), trunc(-2.9))
```

```text Output
(0.75, 3.0)
(0.75, 4)
2 3 -2
```

The batch functions work on a `bytes` buffer of little-endian f64 values. Pack it with [struct](/packages/std/struct):

```python
from math import fsum_all, scale_all, dot_all
from struct import pack, unpack

buf = pack("3d", 1.0, 2.0, 3.0)
print(fsum_all(buf))
print(unpack("3d", scale_all(buf, 10.0)))
print(dot_all(buf, buf))
```

```text Output
6.0
[10.0, 20.0, 30.0]
14.0
```
