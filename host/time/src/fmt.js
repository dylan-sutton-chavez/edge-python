/* Pure calendar helpers. struct_time crosses as a JSON 9-tuple string, like storage's returns. */

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_ABBR = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_ABBR = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const pad = (n, w = 2) => String(n).padStart(w, "0");
// CPython tm_wday is Mon=0..Sun=6; JS getDay is Sun=0. Convert both ways.
const cpyWday = (d) => (d + 6) % 7;
const jsDay = (cpy) => (cpy + 1) % 7;

// Date -> struct_time JSON, CPython order (Y, m, d, H, M, S, wday, yday, isdst).
const toTuple = (dt, utc) => {
    const g = (m) => (utc ? dt["getUTC" + m]() : dt["get" + m]());
    const yday = Math.floor((Date.UTC(g("FullYear"), g("Month"), g("Date")) - Date.UTC(g("FullYear"), 0, 1)) / 86400000) + 1;
    return JSON.stringify([g("FullYear"), g("Month") + 1, g("Date"), g("Hours"), g("Minutes"), g("Seconds"), cpyWday(g("Day")), yday, -1]);
};

// A JSON tuple string, or localtime(now) when the arg is omitted.
const resolve = (t) => (t !== undefined ? JSON.parse(t) : JSON.parse(toTuple(new Date(), false)));

const fmtTuple = (t, fmt) => {
    const [Y, mo, d, H, Mi, S, w, j] = t;
    const map = {
        Y, y: pad(Y % 100), m: pad(mo), d: pad(d), H: pad(H), M: pad(Mi), S: pad(S),
        I: pad((H % 12) || 12), p: H < 12 ? "AM" : "PM", j: pad(j, 3), w: jsDay(w),
        a: DAYS_ABBR[jsDay(w)], A: DAYS[jsDay(w)], b: MONTHS_ABBR[mo - 1], B: MONTHS[mo - 1], "%": "%",
    };
    return fmt.replace(/%(.)/g, (_, c) => (c in map ? String(map[c]) : "%" + c));
};

// "Thu Jan  1 00:00:00 1970", day-of-month space-padded to width 2.
const ascForm = (t) => `${DAYS_ABBR[jsDay(t[6])]} ${MONTHS_ABBR[t[1] - 1]} ${String(t[2]).padStart(2, " ")} ${pad(t[3])}:${pad(t[4])}:${pad(t[5])} ${t[0]}`;

const monthIndex = (name) => {
    const full = MONTHS.findIndex((m) => m.toLowerCase() === name.toLowerCase());
    return full >= 0 ? full : MONTHS_ABBR.findIndex((m) => m.toLowerCase() === name.toLowerCase());
};

// Compile the format into a regex, capture fields, assemble a struct_time JSON.
const STRP = { Y: "(\\d{4})", y: "(\\d{2})", m: "(\\d{1,2})", d: "(\\d{1,2})", H: "(\\d{1,2})", M: "(\\d{1,2})", S: "(\\d{1,2})", j: "(\\d{1,3})", b: "([A-Za-z]+)", B: "([A-Za-z]+)", a: "([A-Za-z]+)", A: "([A-Za-z]+)" };
const strptime = (s, fmt) => {
    const fields = [];
    const src = fmt.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/%(.)/g, (_, c) => {
        if (c === "%") return "%";
        if (!(c in STRP)) return "%" + c;
        fields.push(c);
        return STRP[c];
    });
    const m = new RegExp("^" + src + "$").exec(s);
    if (!m) throw new Error(`time data '${s}' does not match format '${fmt}'`);
    let Y = 1900, mo = 1, d = 1, H = 0, Mi = 0, S = 0;
    fields.forEach((f, i) => {
        const v = m[i + 1];
        if (f === "Y") Y = +v;
        else if (f === "y") Y = 2000 + +v;
        else if (f === "m") mo = +v;
        else if (f === "d") d = +v;
        else if (f === "H") H = +v;
        else if (f === "M") Mi = +v;
        else if (f === "S") S = +v;
        else if (f === "b" || f === "B") mo = monthIndex(v) + 1;
    });
    return toTuple(new Date(Date.UTC(Y, mo - 1, d, H, Mi, S)), true);
};

export default () => ({
    gmtime: (secs) => toTuple(new Date((secs ?? Date.now() / 1000) * 1000), true),
    localtime: (secs) => toTuple(new Date((secs ?? Date.now() / 1000) * 1000), false),
    mktime: (t) => { const a = JSON.parse(t); return new Date(a[0], a[1] - 1, a[2], a[3], a[4], a[5]).getTime() / 1000; },
    strftime: (fmt, t) => fmtTuple(resolve(t), fmt),
    strptime,
    asctime: (t) => ascForm(resolve(t)),
    ctime: (secs) => ascForm(JSON.parse(toTuple(new Date((secs ?? Date.now() / 1000) * 1000), false))),
});
