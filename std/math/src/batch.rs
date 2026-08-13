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

// Apply `f` element-wise over `bytes` of little-endian f64, crossing the host boundary once per call instead of once per element. Chunked decode avoids the alignment UB of casting `&[u8]` to `&[f64]`.
fn map(data: &[u8], f: impl Fn(f64) -> f64) -> Result<Vec<u8>> {
    check_len(data)?;
    let mut out = vec![0u8; data.len()];
    for (src, dst) in data.chunks_exact(8).zip(out.chunks_exact_mut(8)) {
        let x = f64::from_le_bytes(src.try_into().unwrap());
        dst.copy_from_slice(&f(x).to_le_bytes());
    }
    Ok(out)
}

/* Element-wise transforms, `bytes` -> `bytes`, same length. */

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

/* Reductions, `bytes` -> scalar, the largest boundary win (N values in, one out). */

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

/* Binary kernels over two same-length `bytes` operand buffers. */

fn check_pair(a: &[u8], b: &[u8]) -> Result<()> {
    check_len(a)?;
    check_len(b)?;
    if a.len() != b.len() {
        return Err(Error::Value(String::from("buffers must have the same length")));
    }
    Ok(())
}

// Element-wise over two zipped operand buffers.
fn zip_map(a: &[u8], b: &[u8], f: impl Fn(f64, f64) -> f64) -> Result<Vec<u8>> {
    check_pair(a, b)?;
    let mut out = Vec::with_capacity(a.len());
    for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        let xv = f64::from_le_bytes(x.try_into().unwrap());
        let yv = f64::from_le_bytes(y.try_into().unwrap());
        out.extend_from_slice(&f(xv, yv).to_le_bytes());
    }
    Ok(out)
}

macro_rules! bin_all { ($($f:ident => $op:tt),* $(,)?) => { $(
    #[plugin_fn]
    fn $f(a: Bytes, b: Bytes) -> Result<Bytes> { Ok(Bytes(zip_map(&a, &b, |x, y| x $op y)?)) }
)* } }

bin_all!(
    add_all => +,
    sub_all => -,
    mul_all => *,
);

// Scalar broadcast, every element times `k`.
#[plugin_fn]
fn scale_all(data: Bytes, k: f64) -> Result<Bytes> {
    Ok(Bytes(map(&data, |x| x * k)?))
}

// Neumaier compensated dot product.
#[plugin_fn]
fn dot_all(a: Bytes, b: Bytes) -> Result<f64> {
    check_pair(&a, &b)?;
    let mut sum = 0.0_f64;
    let mut c = 0.0_f64;
    for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        let p = f64::from_le_bytes(x.try_into().unwrap()) * f64::from_le_bytes(y.try_into().unwrap());
        let t = sum + p;
        if sum.abs() >= p.abs() { c += (sum - t) + p; } else { c += (p - t) + sum; }
        sum = t;
    }
    Ok(sum + c)
}

/* Row-major (rows x cols) matrix times vector, returns `rows` packed values. */
#[plugin_fn]
fn matvec(m: Bytes, x: Bytes, cols: i64) -> Result<Bytes> {
    check_len(&m)?;
    check_len(&x)?;
    if cols <= 0 || x.len() != (cols as usize) * 8 {
        return Err(Error::Value(String::from("vector length must equal cols")));
    }
    let row_bytes = (cols as usize) * 8;
    if !m.len().is_multiple_of(row_bytes) {
        return Err(Error::Value(String::from("matrix length must be a multiple of cols")));
    }
    let xs: Vec<f64> = x.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
    let mut out = Vec::with_capacity(m.len() / (cols as usize));
    for row in m.chunks_exact(row_bytes) {
        let mut acc = 0.0_f64;
        for (c, xv) in row.chunks_exact(8).zip(&xs) {
            acc += f64::from_le_bytes(c.try_into().unwrap()) * xv;
        }
        out.extend_from_slice(&acc.to_le_bytes());
    }
    Ok(Bytes(out))
}
