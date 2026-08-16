use crate::packages::NativeBinding;
use crate::vm::types::{HeapObj, HeapPool, Val, VmErr};
use std::sync::OnceLock;

use super::{num_arg, opt_str_arg, str_arg};

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_ABBR: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/* UTC only, the engine carries no timezone database, so localtime mirrors gmtime and tzname is UTC. */
pub(super) fn bindings() -> Vec<NativeBinding> {
    vec![
        NativeBinding::from_fn("time", |_, _, _| Ok(Val::float(crate::native::now_ns() as f64 / 1e9)), false),
        NativeBinding::from_fn("time_ns", |h, _, _| int_val(h, crate::native::now_ns() as i128), false),
        NativeBinding::from_fn("monotonic", |_, _, _| Ok(Val::float(mono_secs())), false),
        NativeBinding::from_fn("monotonic_ns", |h, _, _| int_val(h, (mono_secs() * 1e9) as i128), false),
        NativeBinding::from_fn("perf_counter", |_, _, _| Ok(Val::float(mono_secs())), false),
        NativeBinding::from_fn("perf_counter_ns", |h, _, _| int_val(h, (mono_secs() * 1e9) as i128), false),
        NativeBinding::from_fn("sleep", time_sleep, false),
        NativeBinding::from_fn("timezone", |_, _, _| Ok(Val::int(0)), false),
        NativeBinding::from_fn("altzone", |_, _, _| Ok(Val::int(0)), false),
        NativeBinding::from_fn("daylight", |_, _, _| Ok(Val::int(0)), false),
        NativeBinding::from_fn("tzname", |h, _, _| h.alloc(HeapObj::Str("UTC".into())), false),
        NativeBinding::from_fn("gmtime", to_tuple_fn, false),
        NativeBinding::from_fn("localtime", to_tuple_fn, false),
        NativeBinding::from_fn("mktime", time_mktime, false),
        NativeBinding::from_fn("strftime", time_strftime, false),
        NativeBinding::from_fn("strptime", time_strptime, false),
        NativeBinding::from_fn("asctime", time_asctime, false),
        NativeBinding::from_fn("ctime", time_ctime, false),
    ]
}

fn mono_secs() -> f64 {
    static BASE: OnceLock<std::time::Instant> = OnceLock::new();
    BASE.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

fn int_val(heap: &mut HeapPool, n: i128) -> Result<Val, VmErr> {
    match Val::int_checked(n as i64) {
        Some(v) if n >= i64::MIN as i128 && n <= i64::MAX as i128 => Ok(v),
        _ => heap.alloc(HeapObj::LongInt(n)),
    }
}

// Blocking by design, a parked native fetch or sleep stalls every coroutine, unlike the web host.
fn time_sleep(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let secs = num_arg(heap, args, 0, "time.sleep")?.unwrap_or(0.0);
    if secs > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(secs.min(3600.0)));
    }
    Ok(Val::none())
}

/* Days since epoch to civil (y, m, d), Hinnant's algorithm. */
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/* struct_time fields in the web tuple order (Y, m, d, H, M, S, wday, yday, isdst). */
struct Tm {
    y: i64,
    mo: u32,
    d: u32,
    h: u32,
    mi: u32,
    s: u32,
    wday: u32,
    yday: u32,
}

fn tm_from_secs(secs: f64) -> Tm {
    let total = secs.floor() as i64;
    let days = total.div_euclid(86_400);
    let rem = total.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    // Epoch day zero was a Thursday, js getDay counts from Sunday.
    let js_wday = (days.rem_euclid(7) + 4) % 7;
    Tm {
        y, mo, d,
        h: (rem / 3600) as u32,
        mi: (rem / 60 % 60) as u32,
        s: (rem % 60) as u32,
        wday: ((js_wday + 6) % 7) as u32,
        yday: (days - days_from_civil(y, 1, 1) + 1) as u32,
    }
}

fn tm_json(t: &Tm) -> String {
    format!("[{},{},{},{},{},{},{},{},-1]", t.y, t.mo, t.d, t.h, t.mi, t.s, t.wday, t.yday)
}

/* Parses the 9-tuple JSON string back into fields, recomputing wday and yday from the date. */
fn tm_parse(json: &str, who: &'static str) -> Result<Tm, VmErr> {
    let inner = json.trim().trim_start_matches('[').trim_end_matches(']');
    let nums: Vec<i64> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if nums.len() < 6 {
        return Err(VmErr::Raised(format!("RuntimeError: {who} expects a struct_time JSON tuple")));
    }
    let (y, mo, d) = (nums[0], nums[1].clamp(1, 12) as u32, nums[2].clamp(1, 31) as u32);
    let days = days_from_civil(y, mo, d);
    let js_wday = (days.rem_euclid(7) + 4) % 7;
    Ok(Tm {
        y, mo, d,
        h: nums[3].clamp(0, 23) as u32,
        mi: nums[4].clamp(0, 59) as u32,
        s: nums[5].clamp(0, 61) as u32,
        wday: ((js_wday + 6) % 7) as u32,
        yday: (days - days_from_civil(y, 1, 1) + 1) as u32,
    })
}

fn resolve_arg(heap: &HeapPool, args: &[Val], who: &'static str) -> Result<Tm, VmErr> {
    match opt_str_arg(heap, args, 0, who)? {
        Some(json) => tm_parse(&json, who),
        None => Ok(tm_from_secs(crate::native::now_ns() as f64 / 1e9)),
    }
}

fn to_tuple_fn(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let secs = num_arg(heap, args, 0, "time.gmtime")?.unwrap_or(crate::native::now_ns() as f64 / 1e9);
    heap.alloc(HeapObj::Str(tm_json(&tm_from_secs(secs))))
}

fn time_mktime(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let json = str_arg(heap, args, 0, "time.mktime")?;
    let t = tm_parse(&json, "time.mktime")?;
    let secs = days_from_civil(t.y, t.mo, t.d) * 86_400 + t.h as i64 * 3600 + t.mi as i64 * 60 + t.s as i64;
    Ok(Val::float(secs as f64))
}

fn js_wday(cpy: u32) -> usize { ((cpy + 1) % 7) as usize }

fn fmt_tuple(t: &Tm, fmt: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + 8);
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&t.y.to_string()),
            Some('y') => out.push_str(&format!("{:02}", t.y.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{:02}", t.mo)),
            Some('d') => out.push_str(&format!("{:02}", t.d)),
            Some('H') => out.push_str(&format!("{:02}", t.h)),
            Some('M') => out.push_str(&format!("{:02}", t.mi)),
            Some('S') => out.push_str(&format!("{:02}", t.s)),
            Some('I') => out.push_str(&format!("{:02}", if t.h.is_multiple_of(12) { 12 } else { t.h % 12 })),
            Some('p') => out.push_str(if t.h < 12 { "AM" } else { "PM" }),
            Some('j') => out.push_str(&format!("{:03}", t.yday)),
            Some('w') => out.push_str(&js_wday(t.wday).to_string()),
            Some('a') => out.push_str(DAYS_ABBR[js_wday(t.wday)]),
            Some('A') => out.push_str(DAYS[js_wday(t.wday)]),
            Some('b') => out.push_str(MONTHS_ABBR[(t.mo - 1) as usize]),
            Some('B') => out.push_str(MONTHS[(t.mo - 1) as usize]),
            Some('%') => out.push('%'),
            Some(other) => { out.push('%'); out.push(other); }
            None => out.push('%'),
        }
    }
    out
}

fn asc_form(t: &Tm) -> String {
    format!(
        "{} {} {:>2} {:02}:{:02}:{:02} {}",
        DAYS_ABBR[js_wday(t.wday)], MONTHS_ABBR[(t.mo - 1) as usize], t.d, t.h, t.mi, t.s, t.y,
    )
}

fn time_strftime(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let fmt = str_arg(heap, args, 0, "time.strftime")?;
    let t = match opt_str_arg(heap, args, 1, "time.strftime")? {
        Some(json) => tm_parse(&json, "time.strftime")?,
        None => tm_from_secs(crate::native::now_ns() as f64 / 1e9),
    };
    heap.alloc(HeapObj::Str(fmt_tuple(&t, &fmt)))
}

fn time_asctime(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let t = resolve_arg(heap, args, "time.asctime")?;
    heap.alloc(HeapObj::Str(asc_form(&t)))
}

fn time_ctime(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let secs = num_arg(heap, args, 0, "time.ctime")?.unwrap_or(crate::native::now_ns() as f64 / 1e9);
    heap.alloc(HeapObj::Str(asc_form(&tm_from_secs(secs))))
}

fn month_index(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    MONTHS.iter().position(|m| m.to_lowercase() == lower)
        .or_else(|| MONTHS_ABBR.iter().position(|m| m.to_lowercase() == lower))
        .map(|i| i as u32 + 1)
}

/* Mirrors the web strptime directives, unknown ones match literally, names bind months only. */
fn time_strptime(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let s = str_arg(heap, args, 0, "time.strptime")?;
    let fmt = str_arg(heap, args, 1, "time.strptime")?;
    let no_match = || VmErr::Raised(format!("RuntimeError: time data '{s}' does not match format '{fmt}'"));
    let (mut y, mut mo, mut d, mut h, mut mi, mut sec) = (1900i64, 1u32, 1u32, 0u32, 0u32, 0u32);
    let input: Vec<char> = s.chars().collect();
    let mut pos = 0usize;
    let mut fchars = fmt.chars().peekable();
    while let Some(c) = fchars.next() {
        if c != '%' {
            if input.get(pos) != Some(&c) { return Err(no_match()) }
            pos += 1;
            continue;
        }
        let Some(dir) = fchars.next() else { return Err(no_match()) };
        match dir {
            'Y' | 'y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'j' => {
                let width = if dir == 'Y' { 4 } else if dir == 'j' { 3 } else { 2 };
                let start = pos;
                while pos < input.len() && pos - start < width && input[pos].is_ascii_digit() { pos += 1; }
                if pos == start { return Err(no_match()) }
                let v: i64 = input[start..pos].iter().collect::<String>().parse().map_err(|_| no_match())?;
                match dir {
                    'Y' => y = v,
                    'y' => y = if v <= 68 { 2000 + v } else { 1900 + v },
                    'm' => mo = v.clamp(1, 12) as u32,
                    'd' => d = v.clamp(1, 31) as u32,
                    'H' => h = v as u32,
                    'M' => mi = v as u32,
                    'S' => sec = v as u32,
                    _ => {}
                }
            }
            'b' | 'B' | 'a' | 'A' => {
                let start = pos;
                while pos < input.len() && input[pos].is_alphabetic() { pos += 1; }
                if pos == start { return Err(no_match()) }
                let word: String = input[start..pos].iter().collect();
                if matches!(dir, 'b' | 'B') {
                    mo = month_index(&word).ok_or_else(no_match)?;
                }
            }
            '%' => {
                if input.get(pos) != Some(&'%') { return Err(no_match()) }
                pos += 1;
            }
            other => {
                if input.get(pos) != Some(&'%') || input.get(pos + 1) != Some(&other) { return Err(no_match()) }
                pos += 2;
            }
        }
    }
    if pos != input.len() { return Err(no_match()) }
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + sec as i64;
    heap.alloc(HeapObj::Str(tm_json(&tm_from_secs(secs as f64))))
}
