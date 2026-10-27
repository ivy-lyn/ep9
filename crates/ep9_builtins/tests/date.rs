//! Date (§21.4): the date math at the ±8.64e15 ms edges, TimeClip, the local time zone (the
//! offset read from the platform at two instants, and a DST change in a child process with a
//! fixed TZ), Date.parse round trips of every output format, and the constructor, getters,
//! setters and string forms. Expected values are node v24.22.1 output (the DST test: node
//! with the same TZ, without the time-zone name that ep9 leaves out). One difference is on
//! purpose: node's Date.parse gives NaN for a negative year in the toString and toUTCString
//! forms, and §21.4.3.2 asks for the round trip, which ep9 gives. Every script runs with and
//! without GC stress (`common::run_both`).

mod common;

use common::{run, run_both};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// The largest and smallest time values, one ms past them, and the fields there.
#[test]
fn date_math_at_the_time_value_edges() {
    assert_eq!(
        run_both(
            r#"
var hi = new Date(8.64e15), lo = new Date(-8.64e15);
print(hi.toISOString(), lo.toISOString());
print(hi.toUTCString(), lo.toUTCString());
print(hi.getUTCFullYear(), hi.getUTCMonth(), hi.getUTCDate(), hi.getUTCDay());
print(lo.getUTCFullYear(), lo.getUTCMonth(), lo.getUTCDate(), lo.getUTCDay());
print(new Date(8.64e15 + 1).getTime(), new Date(-8.64e15 - 1).getTime());
print(Date.UTC(275760, 8, 13), Date.UTC(275760, 8, 13, 0, 0, 0, 1));
print(Date.UTC(-271821, 3, 20), Date.UTC(-271821, 3, 19, 23, 59, 59, 999));
var d = new Date(8.64e15);
print(d.setUTCMilliseconds(1), d.getTime());
var e = new Date(0);
print(e.setUTCFullYear(275760, 8, 13), e.setUTCHours(24));
print(new Date(-1).toISOString(), new Date(-62198755200000).toISOString());
print(new Date(253402300800000).toISOString(), Date.UTC(-1, 0), Date.UTC(1e300));
"#
        ),
        lines(&[
            "+275760-09-13T00:00:00.000Z -271821-04-20T00:00:00.000Z",
            "Sat, 13 Sep 275760 00:00:00 GMT Tue, 20 Apr -271821 00:00:00 GMT",
            "275760 8 13 6",
            "-271821 3 20 2",
            "NaN NaN",
            "8640000000000000 NaN",
            "-8640000000000000 NaN",
            "NaN NaN",
            "8640000000000000 NaN",
            "1969-12-31T23:59:59.999Z -000001-01-01T00:00:00.000Z",
            "+010000-01-01T00:00:00.000Z -62198755200000 NaN",
        ])
    );
}

/// TimeClip (§21.4.1.31): integral part, -0 to +0, NaN outside the range or not finite.
#[test]
fn time_clip() {
    assert_eq!(
        run_both(
            r#"
var z = new Date(-0.5).getTime();
print(z, 1 / z, new Date(1.9).getTime(), new Date(-1.9).getTime());
print(new Date(NaN).getTime(), new Date(Infinity).getTime(), new Date(-Infinity).getTime());
var d = new Date(0);
print(d.setTime(-0), 1 / d.getTime(), d.setTime(8.64e15 + 0.5), d.setTime('12.7'));
print(Date.UTC(1970, 0, 1, 0, 0, 0, -0.9), 1 / Date.UTC(1970, 0, 1, 0, 0, 0, -0.9));
print(new Date(undefined).getTime(), new Date(null).getTime(), new Date(true).getTime());
"#
        ),
        lines(&[
            "0 Infinity 1 -1",
            "NaN NaN NaN",
            "0 Infinity 8640000000000000 12",
            "0 Infinity",
            "NaN 0 1",
        ])
    );
}

/// Date.parse of every output form gives the time value back (toString, toUTCString and
/// toISOString exactly; toDateString the local midnight; toTimeString is not a date).
#[test]
fn parse_round_trips_every_output_format() {
    assert_eq!(
        run_both(
            r#"
var times = [0, -1000, 1.6e12, 8.64e15, -8.64e15, -62198755200000, 951782400000, 1e14];
var bad = [];
for (var i = 0; i < times.length; i++) {
  var d = new Date(times[i]);
  // toString has whole-minute offsets: an offset with seconds (local mean time before
  // about 1900 in most zones) cannot round-trip.
  var whole = d.getTimezoneOffset() % 1 === 0;
  if (whole && Date.parse(d.toString()) !== times[i]) bad.push('toString ' + d.toString());
  if (Date.parse(d.toUTCString()) !== times[i]) bad.push('toUTCString ' + d.toUTCString());
  if (Date.parse(d.toISOString()) !== times[i]) bad.push('toISOString ' + d.toISOString());
  if (Date.parse(d.toGMTString()) !== times[i]) bad.push('toGMTString');
  if (whole && Date.parse(d.toLocaleString()) !== times[i]) bad.push('toLocaleString');
  var mid = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  // (Both NaN when the local midnight is before the first time value.)
  if (!Object.is(Date.parse(d.toDateString()), mid)) bad.push('toDateString ' + d.toDateString());
  if (whole && new Date(d.toString()).getTime() !== times[i]) bad.push('new Date(toString)');
}
var ms = new Date(1.6e12 + 123);
print(bad.length, bad.join('; '), Date.parse(ms.toISOString()) - 1.6e12);
print(Date.parse(new Date(1.6e12).toTimeString()), Date.parse('Invalid Date'));
print(Date.parse('2021-01-01'), Date.parse('2021-01-01T10:00+01:00'), Date.parse('+002021-02'));
print(Date.parse('-000000-03-31T00:45Z'), Date.parse('2021-13-01'), Date.parse('2021-01-01T24:00Z'));
print(Date.parse('Thu, 01 Jan 1970 00:00:00 GMT'), Date.parse('Thu Jan 01 1970 07:30:00 GMT+0730 (X)'));
print(Date.parse('1970-01-01T00:00:00') === new Date(1970, 0, 1).getTime());
"#
        ),
        lines(&[
            "0  123",
            "NaN NaN",
            "1609459200000 1609491600000 1612137600000",
            "NaN NaN 1609545600000",
            "0 0",
            "true",
        ])
    );
}

/// The local offset of Date (getTimezoneOffset, getHours, toString's GMT+hhmm) is the
/// platform offset (`Cx::local_tz_offset_ms`) at two instants half a year apart, so a zone
/// with DST shows both of its offsets.
#[test]
fn local_time_follows_the_platform_offset_at_two_instants() {
    // 2021-01-15T12:00Z and 2021-07-15T12:00Z.
    let instants = [1_610_712_000_000.0_f64, 1_626_350_400_000.0];
    let mut iso = common::isolate(false);
    let offsets: Vec<f64> = instants
        .iter()
        .map(|&t| {
            iso.with_cx(|cx| Ok(cx.local_tz_offset_ms(t, true)))
                .unwrap()
        })
        .collect();
    for stress in [false, true] {
        let mut src = String::new();
        for t in instants {
            src.push_str(&format!(
                "var d = new Date({t}); print(d.getTimezoneOffset(), d.getHours(), d.getMinutes(), \
                 d.toString().slice(25, 33));\n"
            ));
        }
        let out = common::on_big_stack(move || run(&src, stress)).unwrap();
        let want: Vec<String> = instants
            .iter()
            .zip(&offsets)
            .map(|(&t, &o)| {
                let local = t + o;
                // Whole minutes of the local time, in i64 (no float remainder, H36).
                let local_min = (local / 60_000.0).floor() as i64;
                let hours = local_min.div_euclid(60).rem_euclid(24);
                let minutes = local_min.rem_euclid(60);
                let abs = o.abs() / 60_000.0;
                let sign = if o >= 0.0 { '+' } else { '-' };
                format!(
                    "{} {hours} {minutes} GMT{sign}{:02}{:02}",
                    -o / 60_000.0 + 0.0,
                    (abs / 60.0).floor(),
                    (abs % 60.0).floor()
                )
            })
            .collect();
        assert_eq!(out, want, "offsets {offsets:?}");
    }
}

/// A DST change in a child process with `TZ` set to US Eastern time (POSIX rules, so no
/// tz database is needed): the two offsets, a skipped local hour (the offset before the
/// change) and a repeated one (the earlier instant), as node prints them.
#[test]
#[cfg(unix)]
fn dst_change_in_us_eastern_time() {
    const CHILD: &str = "EP9_DATE_TZ_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["dst_change_in_us_eastern_time", "--exact", "--nocapture"])
            .env(CHILD, "1")
            .env("TZ", "EST5EDT,M3.2.0,M11.1.0")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && text.contains("1 passed"),
            "child failed:\n{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    assert_eq!(
        run_both(
            r#"
var jan = new Date(Date.UTC(2021, 0, 15, 12)), jul = new Date(Date.UTC(2021, 6, 15, 12));
print(jan.getTimezoneOffset(), jul.getTimezoneOffset(), jan.getHours(), jul.getHours());
print(jan.toString(), '|', jul.toTimeString());
var skipped = new Date(2021, 2, 14, 2, 30), repeated = new Date(2021, 10, 7, 1, 30);
print(skipped.toISOString(), skipped.getHours(), repeated.toISOString(), repeated.getTimezoneOffset());
print(new Date(2021, 2, 14, 1, 59).toISOString(), new Date(2021, 10, 7, 2, 0).toISOString());
var d = new Date(2021, 2, 13, 12); d.setDate(14); print(d.toISOString(), d.getHours());
print(Date.parse('2021-07-15T08:00'), new Date(2021, 6, 15).toDateString());
"#
        ),
        lines(&[
            "300 240 7 8",
            "Fri Jan 15 2021 07:00:00 GMT-0500 | 08:00:00 GMT-0400",
            "2021-03-14T07:30:00.000Z 3 2021-11-07T05:30:00.000Z 240",
            "2021-03-14T06:59:00.000Z 2021-11-07T07:00:00.000Z",
            "2021-03-14T16:00:00.000Z 12",
            "1626350400000 Thu Jul 15 2021",
        ])
    );
}

/// The constructor forms, Date called as a function, Date.now, Date.UTC and MakeFullYear.
#[test]
fn constructor_now_and_utc() {
    assert_eq!(
        run_both(
            r#"
var t0 = Date.now();
var a = new Date(), b = Date.now();
print(typeof t0, t0 > 1.5e12, a.getTime() >= t0 && a.getTime() <= b, Number.isInteger(t0));
print(typeof Date(), typeof Date(0), Date(1, 2) === Date(), Date.length, Date.name);
print(new Date(new Date(5)).getTime(), new Date('1970-01-01T00:00:00.007Z').getTime());
print(new Date({ valueOf: function () { return 9; } }).getTime());
print(new Date({ [Symbol.toPrimitive]: function () { return '1970-01-01T00:00:01Z'; } }).getTime());
print(Date.UTC(99, 11, 31), Date.UTC(100, 0), Date.UTC(2020), Date.UTC(), Date.UTC(2020, 1, 30));
print(new Date(99, 0).getFullYear(), new Date(2020, 0, 1, 25).getDate(), new Date(2020, 12).getMonth());
var order = [];
function v(n) { return { valueOf: function () { order.push(n); return n; } }; }
Date.UTC(v(2000), v(1), v(2), v(3), v(4), v(5), v(6));
new Date(v(2000), v(1), v(2));
print(order.join());
class D extends Date { tag() { return 'D'; } }
var sub = new D(42);
print(sub.getTime(), sub.tag(), sub instanceof Date, Object.getPrototypeOf(D.prototype) === Date.prototype);
print(Object.prototype.toString.call(new Date(0)), Object.prototype.toString.call(Date.prototype));
"#
        ),
        lines(&[
            "number true true true",
            "string string true 7 Date",
            "5 7",
            "9",
            "1000",
            "946598400000 -59011459200000 1577836800000 NaN 1583020800000",
            "1999 2 0",
            "2000,1,2,3,4,5,6,2000,1,2",
            "42 D true true",
            "[object Date] [object Object]",
        ])
    );
}

/// The UTC getters and setters, setters' argument order and NaN rules, Annex B getYear and
/// setYear, and the receiver checks.
#[test]
fn getters_setters_and_receivers() {
    assert_eq!(
        run_both(
            r#"
var d = new Date(Date.UTC(2020, 1, 29, 13, 14, 15, 167));
print(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), d.getUTCDay(), d.getUTCHours(),
      d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds(), d.valueOf() === d.getTime());
print(d.setUTCFullYear(2021), d.toISOString());
print(d.setUTCMonth(13, 31), d.toISOString());
print(d.setUTCDate(0), d.setUTCHours(1, 2, 3, 4), d.toISOString());
print(d.setUTCMinutes(70), d.setUTCSeconds(-1, 5), d.toISOString());
var n = new Date(NaN);
print(n.setUTCHours(1), n.setUTCMonth(1), n.setUTCFullYear(2000), n.toISOString());
var calls = [];
var m = new Date(NaN);
m.setUTCMinutes({ valueOf: function () { calls.push('m'); return 1; } },
                { valueOf: function () { calls.push('s'); return 2; } });
print(calls.join(), m.getTime());
var y = new Date(2000, 5, 15, 10);
print(y.getYear(), y.setYear(99) === new Date(1999, 5, 15, 10).getTime(), y.getFullYear());
var e = [];
var fns = ['getTime', 'setTime', 'getUTCDay', 'setUTCHours', 'toISOString', 'toString', 'valueOf',
           'getYear', 'setYear', 'toUTCString'];
for (var i = 0; i < fns.length; i++) {
  try { Date.prototype[fns[i]].call({}); e.push('none'); } catch (x) { e.push(x.constructor.name); }
}
print(e.join());
var touched = false;
try { Date.prototype.setUTCDate.call({}, { valueOf: function () { touched = true; } }); } catch (x) {}
print(touched, Date.prototype.toGMTString === Date.prototype.toUTCString);
"#
        ),
        lines(&[
            "2020 1 29 6 13 14 15 167 true",
            "1614604455167 2021-03-01T13:14:15.167Z",
            "1646313255167 2022-03-03T13:14:15.167Z",
            "1646054055167 1646010123004 2022-02-28T01:02:03.004Z",
            "1646014203004 1646014199005 2022-02-28T02:09:59.005Z",
            "NaN NaN 946684800000 2000-01-01T00:00:00.000Z",
            "m,s NaN",
            "100 true 1999",
            "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError",
            "false true",
        ])
    );
}

/// toISOString's RangeError, toJSON, [Symbol.toPrimitive], "Invalid Date" strings and the
/// property attributes.
#[test]
fn strings_to_json_and_to_primitive() {
    assert_eq!(
        run_both(
            r#"
var bad = new Date(NaN);
var r; try { bad.toISOString(); } catch (x) { r = x.constructor.name; }
print(r, bad.toString(), bad.toUTCString(), bad.toDateString(), bad.toTimeString(), String(bad));
print(bad.toJSON(), new Date(0).toJSON(), Date.prototype.toJSON.call({ toISOString: function () { return 'iso'; } }));
var p = new Date(0);
print(p[Symbol.toPrimitive]('number'), typeof p[Symbol.toPrimitive]('default'), p + 1 === p.toString() + '1', p - 1);
var t; try { p[Symbol.toPrimitive]('nope'); } catch (x) { t = x.constructor.name; }
var u; try { Date.prototype[Symbol.toPrimitive].call(1, 'number'); } catch (x) { u = x.constructor.name; }
print(t, u, Date.prototype[Symbol.toPrimitive].name, Date.prototype[Symbol.toPrimitive].length);
var desc = Object.getOwnPropertyDescriptor(Date.prototype, Symbol.toPrimitive);
print(desc.writable, desc.enumerable, desc.configurable);
var pd = Object.getOwnPropertyDescriptor(Date, 'prototype');
print(pd.writable, pd.enumerable, pd.configurable, Date.prototype.constructor === Date);
print(Object.getOwnPropertyNames(Date).join(), Object.getOwnPropertyNames(Date.prototype).length);
print(new Date(0) == new Date(0).toString(), new Date(0) < new Date(1));
"#
        ),
        lines(&[
            "RangeError Invalid Date Invalid Date Invalid Date Invalid Date Invalid Date",
            "null 1970-01-01T00:00:00.000Z iso",
            "0 string true -1",
            "TypeError TypeError [Symbol.toPrimitive] 1",
            "false false true",
            "false false false true",
            "length,name,prototype,now,parse,UTC 47",
            "true true",
        ])
    );
}

/// A legacy-form offset word of 4 bytes that holds a multibyte char ('+1€', a lone
/// surrogate, which becomes U+FFFD): NaN, not a panic (X-DATE1).
#[test]
fn parse_offset_word_with_a_multibyte_char() {
    assert_eq!(
        run_both(
            r#"
print(Date.parse('Jan 01 1970 GMT+1€'), Date.parse('Jan 01 1970 GMT+1\ud800'), new Date('Jan 01 1970 UTC+1é1').getTime());
print(Date.parse('Jan 01 1970 GMT+€1'), Date.parse('Jan 01 1970 GMT-1é1'), Date.parse('Jan 01 1970 GMT+1\u0800'));
"#
        ),
        lines(&["NaN NaN NaN", "NaN NaN NaN"])
    );
}

/// A local time far outside the Date range (an hour of -1e100) reaches the time-zone offset
/// before TimeClip: NaN, not an i64 overflow in the equivalent-year fallback (X-DATE2).
#[test]
fn enormous_local_time_is_nan() {
    assert_eq!(
        run_both(
            r#"
print(new Date(2020, 0, 1, -1e100).getTime(), new Date(2020, 0, 1, 0, 0, 0, -1.7e308).getTime());
print(new Date(2020, 0, 1, 1e100).getTime(), new Date(2020, 0, 1, -9.3e18).getTime());
var d = new Date(0);
print(d.setHours(-1e100), new Date(0).setMinutes(-1e100), new Date(0).setSeconds(-1e100));
print(new Date(0).setMilliseconds(-1e100), new Date(0).setDate(-1e100), new Date(0).setHours(0, 0, 0, -Number.MAX_VALUE));
"#
        ),
        lines(&["NaN NaN", "NaN NaN", "NaN NaN NaN", "NaN NaN NaN"])
    );
}
