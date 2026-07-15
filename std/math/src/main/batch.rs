/*
Vectorized fast path. Operates on `bytes` of little-endian f64, crossing the host boundary once per call instead of once per element.
*/

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use wasm_pdk::*;

fn check_len(data: &[u8]) -> Result<()> {
    if !data.len().is_multiple_of(8) {
        return Err(Error::Value(String::from("buffer length must be a multiple of 8")));
    }
    Ok(())
}

// Apply `f` element-wise; chunked decode avoids the alignment UB of casting `&[u8]` to `&[f64]`.
fn map(data: &[u8], f: impl Fn(f64) -> f64) -> Result<Vec<u8>> {
    check_len(data)?;
    let mut out = vec![0u8; data.len()];
    for (src, dst) in data.chunks_exact(8).zip(out.chunks_exact_mut(8)) {
        let x = f64::from_le_bytes(src.try_into().unwrap());
        dst.copy_from_slice(&f(x).to_le_bytes());
    }
    Ok(out)
}

/* Element-wise transforms: `bytes` -> `bytes`, same length. */

macro_rules! map_all { ($($f:ident => $l:ident),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(data: Bytes) -> Result<Bytes> { Ok(Bytes(map(&data, libm::$l)?)) }
)* } }

map_all!(
    sqrt_all => sqrt,
    abs_all => fabs,
    exp_all => exp,
    log_all => log,
    sin_all => sin,
    cos_all => cos,
);

/* Reductions: `bytes` -> scalar, the largest boundary win (N values in, one out). */

// Neumaier compensated sum over the packed buffer.
#[plugin_fn]
fn fsum_all(data: Bytes) -> Result<f64> {
    check_len(&data)?;
    let mut sum = 0.0_f64;
    let mut c = 0.0_f64;
    for src in data.chunks_exact(8) {
        let x = f64::from_le_bytes(src.try_into().unwrap());
        let t = sum + x;
        if sum.abs() >= x.abs() { c += (sum - t) + x; } else { c += (x - t) + sum; }
        sum = t;
    }
    Ok(sum + c)
}

#[plugin_fn]
fn prod_all(data: Bytes) -> Result<f64> {
    check_len(&data)?;
    let mut acc = 1.0_f64;
    for src in data.chunks_exact(8) {
        acc *= f64::from_le_bytes(src.try_into().unwrap());
    }
    Ok(acc)
}
