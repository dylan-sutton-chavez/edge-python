/*
Scalar floating-point surface. `math`, raising `ValueError` ("math domain error") out of domain.
*/

use alloc::string::String;
use wasm_pdk::*;

// Raises ValueError with this exact text on out-of-domain inputs.
fn dom() -> Error { Error::Value(String::from("math domain error")) }

// One-arg libm pass-throughs.
macro_rules! libm1 { ($($f:ident),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(x: f64) -> f64 { libm::$f(x) }
)* } }

// One-arg with an out-of-domain predicate.
macro_rules! dom1 { ($($f:ident($x:ident) if $bad:expr),* $(,)?) => { $(
    #[plugin_fn]
    fn $f($x: f64) -> Result<f64> {
        if $bad { return Err(dom()); }
        Ok(libm::$f($x))
    }
)* } }

// NaN result from non-NaN input means domain error.
macro_rules! nan1 { ($($f:ident => $l:ident),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(x: f64) -> Result<f64> {
        let r = libm::$l(x);
        if r.is_nan() && !x.is_nan() { return Err(dom()); }
        Ok(r)
    }
)* } }

// Two-arg variant of `nan1`.
macro_rules! nan2 { ($($f:ident),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(x: f64, y: f64) -> Result<f64> {
        let r = libm::$f(x, y);
        if r.is_nan() && !x.is_nan() && !y.is_nan() { return Err(dom()); }
        Ok(r)
    }
)* } }

// Round with libm, then convert via `to_int`.
macro_rules! int1 { ($($f:ident),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(x: f64) -> Result<i128> { to_int(libm::$f(x)) }
)* } }

/* Constants */

#[plugin_const]
fn pi() -> f64 { core::f64::consts::PI }

#[plugin_const]
fn e() -> f64 { core::f64::consts::E }

#[plugin_const]
fn tau() -> f64 { core::f64::consts::TAU }

#[plugin_const]
fn inf() -> f64 { f64::INFINITY }

#[plugin_const]
fn nan() -> f64 { f64::NAN }

/* Power and logarithmic */

dom1!(sqrt(x) if x < 0.0);
libm1!(cbrt, exp, exp2, expm1);

// libm yields NaN on a domain error such as a negative base with a fractional exponent.
nan2!(pow);

// Optional second positional `base`, matching `math.log(x[, base])`.
#[plugin_fn]
fn log(x: f64, rest: Args) -> Result<f64> {
    if x <= 0.0 { return Err(dom()); }
    let ln = libm::log(x);
    match rest.len() {
        0 => Ok(ln),
        1 => {
            let base = rest.get::<f64>(0).unwrap()?;
            if base <= 0.0 { return Err(dom()); }
            Ok(ln / libm::log(base))
        }
        _ => Err(Error::Type(String::from("log expected at most 2 arguments"))),
    }
}

dom1!(
    log2(x) if x <= 0.0,
    log10(x) if x <= 0.0,
    log1p(x) if x <= -1.0,
);

/* Trigonometric */

libm1!(sin, cos, tan, atan);
dom1!(
    asin(x) if !(-1.0..=1.0).contains(&x),
    acos(x) if !(-1.0..=1.0).contains(&x),
);

#[plugin_fn]
fn atan2(y: f64, x: f64) -> f64 { libm::atan2(y, x) }

// Variadic Euclidean norm, matching `math.hypot(*coordinates)`.
#[plugin_fn]
fn hypot(coords: Args) -> Result<f64> {
    let mut sum = 0.0;
    for h in &coords.0 { let v = f64::from_handle(h.raw())?; sum += v * v; }
    Ok(libm::sqrt(sum))
}

// Euclidean distance between two equal-length coordinate sequences.
#[plugin_fn]
fn dist(p: Handle, q: Handle) -> Result<f64> {
    let (pi, qi) = (p.iter()?, q.iter()?);
    let mut sum = 0.0;
    loop {
        match (pi.iter_next()?, qi.iter_next()?) {
            (Some(a), Some(b)) => {
                let d = f64::from_handle(a.raw())? - f64::from_handle(b.raw())?;
                sum += d * d;
            }
            (None, None) => break,
            _ => return Err(Error::Value(String::from("both points must have the same number of dimensions"))),
        }
    }
    Ok(libm::sqrt(sum))
}

/* Hyperbolic */

libm1!(sinh, cosh, tanh, asinh);
dom1!(
    acosh(x) if x < 1.0,
    atanh(x) if x <= -1.0 || x >= 1.0,
);

/* Angular conversion */

#[plugin_fn]
fn degrees(x: f64) -> f64 { x * (180.0 / core::f64::consts::PI) }

#[plugin_fn]
fn radians(x: f64) -> f64 { x * (core::f64::consts::PI / 180.0) }

/* Special functions */

libm1!(erf, erfc);
nan1!(gamma => tgamma, lgamma => lgamma);

/* Floating-point manipulation and classification */

libm1!(fabs);
nan2!(fmod, remainder);

#[plugin_fn]
fn copysign(x: f64, y: f64) -> f64 { libm::copysign(x, y) }

#[plugin_fn]
fn ldexp(x: f64, i: i64) -> f64 { libm::ldexp(x, i as i32) }

#[plugin_fn]
fn isnan(x: f64) -> bool { x.is_nan() }

#[plugin_fn]
fn isinf(x: f64) -> bool { x.is_infinite() }

#[plugin_fn]
fn isfinite(x: f64) -> bool { x.is_finite() }

// `floor`/`ceil`/`trunc` return an int; reject non-finite or out-of-i128 values like CPython int conversion.
fn to_int(x: f64) -> Result<i128> {
    if !x.is_finite() {
        return Err(Error::Value(String::from("cannot convert float NaN or infinity to integer")));
    }
    // Values at or beyond 2^127 saturate on cast; reject so the result is never silently clamped.
    if x.abs() >= 170141183460469231731687303715884105728.0 {
        return Err(Error::Value(String::from("int too large to convert")));
    }
    Ok(x as i128)
}

int1!(floor, ceil, trunc);

// `modf(x) -> (fractional, integral)`, both with the sign of `x`.
#[plugin_fn]
fn modf(x: f64) -> Result<Handle> {
    let integral = libm::trunc(x);
    let frac = encode(Value::Float(x - integral))?;
    let ip = encode(Value::Float(integral))?;
    Handle::new_tuple(&[frac.raw(), ip.raw()])
}

// `frexp(x) -> (mantissa, exponent)` with `x == mantissa * 2**exponent`.
#[plugin_fn]
fn frexp(x: f64) -> Result<Handle> {
    let (m, exp) = libm::frexp(x);
    let mantissa = encode(Value::Float(m))?;
    let exponent = encode(Value::Int(exp as i128))?;
    Handle::new_tuple(&[mantissa.raw(), exponent.raw()])
}

/* Summation and products over an iterable */

// Neumaier compensated summation, matching `math.fsum` accuracy.
#[plugin_fn]
fn fsum(it: Handle) -> Result<f64> {
    let iter = it.iter()?;
    let mut sum = 0.0_f64;
    let mut c = 0.0_f64;
    while let Some(h) = iter.iter_next()? {
        let x = f64::from_handle(h.raw())?;
        let t = sum + x;
        if sum.abs() >= x.abs() { c += (sum - t) + x; } else { c += (x - t) + sum; }
        sum = t;
    }
    Ok(sum + c)
}

// `prod(iterable, *, start=1.0)`. Returns a float; integer inputs lose their int-ness.
#[plugin_fn]
fn prod(it: Handle, kw: Kwargs) -> Result<f64> {
    let mut acc = kw.get::<f64>("start")?.unwrap_or(1.0);
    let iter = it.iter()?;
    while let Some(h) = iter.iter_next()? { acc *= f64::from_handle(h.raw())?; }
    Ok(acc)
}
