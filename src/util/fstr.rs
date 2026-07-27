/* f64 -> string: shortest round-trip digits, fixed for exponents in (-4, 16], else scientific with sign and ≥2 exp digits; always includes `.0` or exponent so floats never read as ints. */
pub fn format_f64(f: f64) -> alloc::string::String {
    use alloc::string::String;
    use core::fmt::Write;
    if f.is_nan() { return String::from("nan"); }
    if f == f64::INFINITY { return String::from("inf"); }
    if f == f64::NEG_INFINITY { return String::from("-inf"); }
    if f == 0.0 {
        return if f.is_sign_negative() { String::from("-0.0") } else { String::from("0.0") };
    }

    // Rust's `{:e}` yields the same unique shortest mantissa dtoa does: "d[.ddd]eN".
    let neg = f < 0.0;
    let mut sci = String::with_capacity(32);
    let _ = write!(&mut sci, "{:e}", f.abs());
    let (mant, exp_str) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exp: i32 = exp_str.parse().unwrap_or(0);
    // Significant digits with the point removed; `decpt` = count of digits left of the point.
    let mut digits = String::with_capacity(mant.len());
    for c in mant.chars() { if c != '.' { digits.push(c); } }
    let decpt = exp + 1;
    let ndig = digits.len() as i32;

    let mut out = String::with_capacity(digits.len() + 8);
    if neg { out.push('-'); }
    if decpt > -4 && decpt <= 16 {
        if decpt <= 0 {
            out.push_str("0.");
            for _ in 0..(-decpt) { out.push('0'); }
            out.push_str(&digits);
        } else if decpt >= ndig {
            out.push_str(&digits);
            for _ in 0..(decpt - ndig) { out.push('0'); }
            out.push_str(".0");
        } else {
            let (l, r) = digits.split_at(decpt as usize);
            out.push_str(l);
            out.push('.');
            out.push_str(r);
        }
    } else {
        let mut chars = digits.chars();
        out.push(chars.next().unwrap_or('0'));
        let rest: String = chars.collect();
        if !rest.is_empty() { out.push('.'); out.push_str(&rest); }
        out.push('e');
        let e = decpt - 1;
        out.push(if e < 0 { '-' } else { '+' });
        let ea = e.unsigned_abs();
        if ea < 10 { out.push('0'); }
        let mut nb = itoa::Buffer::new();
        out.push_str(nb.format(ea));
    }
    out
}

#[macro_export]
macro_rules! s {
    (@b $s:ident;) => {};
    (@b $s:ident; $l:literal $(, $($r:tt)*)?) => { $s.push_str($l); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; str $v:expr $(, $($r:tt)*)?) => { $s.push_str($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; int $v:expr $(, $($r:tt)*)?) => {{ let mut _b = itoa::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; float $v:expr $(, $($r:tt)*)?) => { $s.push_str(&$crate::util::fstr::format_f64($v)); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; char $v:expr $(, $($r:tt)*)?) => { $s.push($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; bool $v:expr $(, $($r:tt)*)?) => { $s.push_str(if $v { "true" } else { "false" }); $($crate::s!(@b $s; $($r)*);)? };
    (cap: $c:expr; $($t:tt)*) => {{ let mut _s = alloc::string::String::with_capacity($c); $crate::s!(@b _s; $($t)*); _s }};
    ($($t:tt)*) => {{ let mut _s = alloc::string::String::new(); $crate::s!(@b _s; $($t)*); _s }};
}

pub enum E {
    Custom { msg: alloc::string::String },
}

impl E {
    pub fn message(&self) -> alloc::string::String {
        match self {
            Self::Custom { msg } => msg.clone(),
        }
    }
}

impl From<E> for alloc::string::String { fn from(e: E) -> Self { e.message() } }

#[macro_export]
macro_rules! err {
    ($($t:tt)*) => { $crate::util::fstr::E::Custom { msg: $crate::s!($($t)*) } };
}
