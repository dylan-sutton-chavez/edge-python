// Pure-Rust f64 math (no libm, works under no_std / WASM).

#[inline]
pub fn fpowi(mut base: f64, exp: i32) -> f64 {
    if exp == 0 { return 1.0; }
    let neg = exp < 0;
    let mut e = (exp as i64).unsigned_abs() as u32;
    let mut r = 1.0;
    while e > 0 { if e & 1 != 0 { r *= base; } base *= base; e >>= 1; }
    if neg { 1.0 / r } else { r }
}

#[inline]
pub fn fround(x: f64) -> f64 {
    // Round half to even, no i64 cast so large/huge values are exact.
    if !x.is_finite() { return x; }
    let fl = libm::floor(x);
    let diff = x - fl;
    if diff < 0.5 { fl }
    else if diff > 0.5 { fl + 1.0 }
    // Exactly .5: pick the even neighbour. `fl` is integral here, so test evenness without a cast.
    else if libm::floor(fl / 2.0) * 2.0 == fl { fl }
    else { fl + 1.0 }
}

#[inline]
pub fn fpowf(base: f64, exp: f64) -> f64 {
    let ei = exp as i32;
    // Exact integer exponents stay on the squaring path; everything else uses libm `pow`.
    if (ei as f64) == exp && exp.abs() < 1024.0 { return fpowi(base, ei); }
    libm::pow(base, exp)
}

#[inline]
pub fn ffloor(x: f64) -> f64 { libm::floor(x) }

#[inline]
pub fn fabs(x: f64) -> f64 {
    f64::from_bits(f64::to_bits(x) & 0x7FFF_FFFF_FFFF_FFFF)
}

#[inline]
pub fn ftrunc(x: f64) -> f64 { libm::trunc(x) }

#[inline]
pub fn fsignum(x: f64) -> f64 {
    if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }
}

#[inline]
pub fn flog10(x: f64) -> f64 { libm::log10(x) }
