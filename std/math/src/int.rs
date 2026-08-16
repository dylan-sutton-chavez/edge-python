use alloc::string::String;
use wasm_pdk::*;

// Results that exceed `i128` raise `ValueError`, since Edge Python ints cap at `i128`.
fn too_large(what: &str) -> Error {
    Error::Value(match what {
        "factorial" => String::from("factorial() result exceeds the i128 range"),
        "comb" => String::from("comb() result exceeds the i128 range"),
        "perm" => String::from("perm() result exceeds the i128 range"),
        _ => String::from("lcm() result exceeds the i128 range"),
    })
}

// Custom ABI error kind, the engine renders the carried message verbatim.
const CUSTOM_KIND: u32 = 6;

// Euclidean gcd on non-negative inputs.
fn gcd2(mut a: i128, mut b: i128) -> i128 {
    while b != 0 { let t = b; b = a % b; a = t; }
    a
}

// Unsigned variant, `unsigned_abs` keeps `i128::MIN` usable as an input.
fn gcd2u(mut a: u128, mut b: u128) -> u128 {
    while b != 0 { let t = b; b = a % b; a = t; }
    a
}

#[plugin_fn]
fn factorial(n: i128) -> Result<i128> {
    if n < 0 { return Err(Error::Value(String::from("factorial() not defined for negative values"))); }
    let mut acc: i128 = 1;
    let mut k: i128 = 2;
    while k <= n {
        acc = acc.checked_mul(k).ok_or_else(|| too_large("factorial"))?;
        k += 1;
    }
    Ok(acc)
}

// Variadic gcd, matching `math.gcd(*integers)`, `gcd()` is 0.
#[plugin_fn]
fn gcd(nums: Args) -> Result<i128> {
    let mut g: u128 = 0;
    for h in &nums.0 {
        let n = i128::from_handle(h.raw())?;
        g = gcd2u(g, n.unsigned_abs());
    }
    // A gcd is never negative and 2^127 does not fit `i128`, so that one case raises OverflowError.
    if g > i128::MAX as u128 {
        return Err(Error::Custom { kind: CUSTOM_KIND, message: String::from("OverflowError: gcd() result exceeds the i128 range") });
    }
    Ok(g as i128)
}

// Variadic lcm, matching `math.lcm(*integers)`, `lcm()` is 1, any zero yields 0.
#[plugin_fn]
fn lcm(nums: Args) -> Result<i128> {
    let mut l: u128 = 1;
    for h in &nums.0 {
        if i128::from_handle(h.raw())? == 0 { return Ok(0); }
    }
    for h in &nums.0 {
        let n = i128::from_handle(h.raw())?.unsigned_abs();
        let g = gcd2u(l, n);
        l = (l / g).checked_mul(n).ok_or_else(|| too_large("lcm"))?;
        if l > i128::MAX as u128 { return Err(too_large("lcm")); }
    }
    Ok(l as i128)
}

#[plugin_fn]
fn isqrt(n: i128) -> Result<i128> {
    if n < 0 { return Err(Error::Value(String::from("isqrt() argument must be nonnegative"))); }
    if n == 0 { return Ok(0); }
    // Newton's method on integers, division keeps every step within `i128`.
    let mut x = n;
    let mut y = x / 2 + x % 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    Ok(x)
}

#[plugin_fn]
fn comb(n: i128, k: i128) -> Result<i128> {
    if n < 0 || k < 0 { return Err(Error::Value(String::from("comb() arguments must be non-negative"))); }
    if k > n { return Ok(0); }
    let k = k.min(n - k);
    let mut acc: i128 = 1;
    let mut i: i128 = 1;
    // acc holds C(n-k+i, i) at each step, the gcd reduction keeps intermediates within the final result.
    while i <= k {
        let g = gcd2(acc, i);
        let t = (n - k + i) / (i / g);
        acc = (acc / g).checked_mul(t).ok_or_else(|| too_large("comb"))?;
        i += 1;
    }
    Ok(acc)
}

// `perm(n)` is `n!`, optional `k` gives the falling factorial `n*(n-1)*...*(n-k+1)`.
#[plugin_fn]
fn perm(n: i128, rest: Args) -> Result<i128> {
    if n < 0 { return Err(Error::Value(String::from("perm() arguments must be non-negative"))); }
    let k = match rest.len() {
        0 => n,
        1 => rest.get::<i128>(0).unwrap()?,
        _ => return Err(Error::Type(String::from("perm expected at most 2 arguments"))),
    };
    if k < 0 { return Err(Error::Value(String::from("perm() arguments must be non-negative"))); }
    if k > n { return Ok(0); }
    let mut acc: i128 = 1;
    let mut i: i128 = 0;
    while i < k {
        acc = acc.checked_mul(n - i).ok_or_else(|| too_large("perm"))?;
        i += 1;
    }
    Ok(acc)
}
