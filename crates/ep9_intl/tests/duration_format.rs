//! Intl.DurationFormat (ECMA-402 §13): INTL W4 DURATION, with and without GC stress.
//!
//! [`CASES`]: the expected lines are node v26.13.1's output for the same script (`print` =
//! `console.log` of the arguments, each through `String`, joined by a space), except one:
//! node reads a duration String through Temporal (`format("PT1H")` is "1 hr"); `ep9_intl` has
//! no Temporal reader yet, so ep9 gives the RangeError of ECMA-402 ToDurationRecord.
//!
//! [`FP4`]: DL INTL-V2-FP4. The sub-second sums are exact (an `i128` sum and one `Decimal`, no
//! `f64` sum). The expected text is the exact decimal of the sum, as the BigInt helper of the
//! test262 harness (`durationToFractional`) makes it; node (V8) is not the reference here: it
//! gives 9007200.256743992 for the second duration and an `i64` overflow for the third.

mod common;
use common::{lines, run_both};

const CASES: &str = r#"const D = Intl.DurationFormat;
print(typeof D, D.length, D.name, Object.getPrototypeOf(D) === Function.prototype);
print(Object.getOwnPropertyNames(D).join(), Object.getOwnPropertyNames(D.prototype).join());
print(D.supportedLocalesOf.length, D.prototype.format.length, D.prototype.formatToParts.length, D.prototype.resolvedOptions.length);
const full = {years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10};
for (const style of ["long", "short", "narrow", "digital"]) {
  for (const loc of ["en", "de", "fr", "ja", "ar", "da", "pl"]) {
    const df = new D(loc, {style});
    print(loc, style, df.format(full), "|", df.format({hours: -1, minutes: 0, seconds: -5}), "|", df.format({days: 2, seconds: 3, milliseconds: 40}));
  }
}
print(JSON.stringify(new D("en").resolvedOptions()));
print(JSON.stringify(new D("de-u-nu-arab", {style: "digital", fractionalDigits: 3}).resolvedOptions()));
print(JSON.stringify(new D("en", {style: "long", seconds: "numeric"}).resolvedOptions()));
print(JSON.stringify(new D("en", {hours: "numeric"}).resolvedOptions()));
print(JSON.stringify(new D("en", {milliseconds: "numeric"}).resolvedOptions()));
const d = new D("en", {style: "digital", fractionalDigits: 2});
print(d.format({hours: 1, minutes: 2, seconds: 3, milliseconds: 456}), d.format({minutes: -5, seconds: -1, milliseconds: -999}));
print(new D("en", {seconds: "long", milliseconds: "numeric"}).format({seconds: 1, milliseconds: 500}));
print(new D("en", {minutes: "numeric"}).format({hours: 2, minutes: 3, seconds: 4}));
print(new D("en", {style: "narrow", hoursDisplay: "always"}).format({days: 1}));
print(new D("en").format({seconds: 9007199254740991, milliseconds: 975, microseconds: 424}));
print(new D("en", {style: "digital"}).format({seconds: 9007199254740991, milliseconds: 975, microseconds: 424}));
print(new D("en", {style: "digital"}).format({hours: 0}), JSON.stringify(new D("en").format({years: 0})));
print(new D("en", {yearsDisplay: "always"}).format({years: -0, days: -1}));
print(new D("en", {style: "digital"}).format({days: -1, hours: 0, minutes: -1, seconds: -2}));
print(JSON.stringify(new D("en", {style: "digital"}).formatToParts({hours: 1, minutes: 2, seconds: 3, milliseconds: 5})));
print(JSON.stringify(new D("en").formatToParts({years: -2, hours: -1})));
print(new D("en-u-nu-thai", {style: "digital"}).format({hours: 12, minutes: 34, seconds: 56}));
print(new D("en", {numberingSystem: "arab"}).format({hours: 12}));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => D());
t(() => new D("en", null));
t(() => new D("en", "x"));
t(() => new D("en", {style: "x"}));
t(() => new D("en", {years: "numeric"}));
t(() => new D("en", {hours: "numeric", minutes: "long"}));
t(() => new D("en", {milliseconds: "numeric", microseconds: "long"}));
t(() => new D("en", {style: "digital", millisecondsDisplay: "always"}));
t(() => new D("en", {fractionalDigits: 10}));
t(() => new D("en", {fractionalDigits: -1}));
t(() => new D("en", {hoursDisplay: "x"}));
t(() => new D("en").format({years: 1.5}));
t(() => new D("en").format({years: 1, months: -1}));
t(() => new D("en").format({}));
t(() => new D("en").format("PT1H"));
t(() => new D("en").format(5));
t(() => new D("en").format({weeks: 2 ** 32}));
t(() => new D("en").format({seconds: Infinity}));
t(() => D.prototype.format.call({}, {years: 1}));
t(() => D.prototype.resolvedOptions.call(new Intl.NumberFormat()));
print(r.join(" "));
const log = [];
new D("en").format({ get years() { log.push("years"); return 1; }, get days() { log.push("days"); return 1; }, get hours() { log.push("hours"); return undefined; }, get nanoseconds() { log.push("ns"); return { valueOf() { log.push("ns.valueOf"); return 3; } }; } });
print(log.join());
class S extends D {}
print(new S("de") instanceof S, new S("de").format({hours: 2}));
"#;

/// The shape of %Intl.DurationFormat% and its prototype, the styles of 7 locales, the
/// numeric (digital) units, the fraction of the sub-second units, the sign, the -0 of a
/// zero unit, formatToParts with the `unit` of each number part, resolvedOptions, the
/// numbering systems, the errors and the order of the field reads.
#[test]
fn the_cases_give_the_node_lines() {
    assert_eq!(
        run_both(CASES),
        lines(&[
            r#"function 0 DurationFormat true"#,
            r#"length,name,prototype,supportedLocalesOf constructor,resolvedOptions,format,formatToParts"#,
            r#"1 1 1 0"#,
            r#"en long 1 year, 2 months, 3 weeks, 4 days, 5 hours, 6 minutes, 7 seconds, 8 milliseconds, 9 microseconds, 10 nanoseconds | -1 hour, 5 seconds | 2 days, 3 seconds, 40 milliseconds"#,
            r#"de long 1 Jahr, 2 Monate, 3 Wochen, 4 Tage, 5 Stunden, 6 Minuten, 7 Sekunden, 8 Millisekunden, 9 Mikrosekunden und 10 Nanosekunden | -1 Stunde, 5 Sekunden | 2 Tage, 3 Sekunden und 40 Millisekunden"#,
            r#"fr long 1 an, 2 mois, 3 semaines, 4 jours, 5 heures, 6 minutes, 7 secondes, 8 millisecondes, 9 microsecondes et 10 nanosecondes | -1 heure et 5 secondes | 2 jours, 3 secondes et 40 millisecondes"#,
            r#"ja long 1 年 2 か月 3 週間 4 日 5 時間 6 分 7 秒 8 ミリ秒 9 マイクロ秒 10 ナノ秒 | -1 時間 5 秒 | 2 日 3 秒 40 ミリ秒"#,
            r#"ar long سنة، وشهران، و3 أسابيع، و4 أيام، و5 ساعات، و6 دقائق، و7 ثوان، و8 ملي ثانية، و9 ميكروثانية، و10 نانو ثانية | ساعة و5 ثوان | يومان، و3 ثوان، و40 ملي ثانية"#,
            r#"da long 1 år, 2 måneder, 3 uger, 4 dage, 5 timer, 6 minutter, 7 sekunder, 8 millisekunder, 9 mikrosekunder og 10 nanosekunder | -1 time og 5 sekunder | 2 dage, 3 sekunder og 40 millisekunder"#,
            r#"pl long 1 rok, 2 miesiące, 3 tygodnie, 4 dni, 5 godzin, 6 minut, 7 sekund, 8 milisekund, 9 mikrosekund i 10 nanosekund | -1 godzina i 5 sekund | 2 dni, 3 sekundy i 40 milisekund"#,
            r#"en short 1 yr, 2 mths, 3 wks, 4 days, 5 hr, 6 min, 7 sec, 8 ms, 9 μs, 10 ns | -1 hr, 5 sec | 2 days, 3 sec, 40 ms"#,
            r#"de short 1 J, 2 Mon., 3 Wo., 4 Tg., 5 Std., 6 Min., 7 Sek., 8 ms, 9 μs und 10 ns | -1 Std., 5 Sek. | 2 Tg., 3 Sek. und 40 ms"#,
            r#"fr short 1 an, 2 m., 3 sem., 4 j, 5 h, 6 min, 7 s, 8 ms, 9 μs et 10 ns | -1 h et 5 s | 2 j, 3 s et 40 ms"#,
            r#"ja short 1 年 2 か月 3 週間 4 日 5 時間 6 分 7 秒 8 ms 9 μs 10 ns | -1 時間 5 秒 | 2 日 3 秒 40 ms"#,
            r#"ar short سنة واحدة، وشهران، و3 أسابيع، و4 أيام، و5 س، و6 د، و7 ث، و8 ملي ث، و9 م.ث.، و10 ن.ث. | ‎-1 س و5 ث | يومان، و3 ث، و40 ملي ث"#,
            r#"da short 1 år, 2 mdr., 3 uger, 4 dage, 5 t., 6 min., 7 sek., 8 ms, 9 μs og 10 ns | -1 t. og 5 sek. | 2 dage, 3 sek. og 40 ms"#,
            r#"pl short 1 rok, 2 mies., 3 tyg., 4 dni, 5 godz., 6 min, 7 sek., 8 ms, 9 μs i 10 ns | -1 godz. i 5 sek. | 2 dni, 3 sek. i 40 ms"#,
            r#"en narrow 1y 2m 3w 4d 5h 6m 7s 8ms 9μs 10ns | -1h 5s | 2d 3s 40ms"#,
            r#"de narrow 1 J, 2 M, 3 W, 4 T, 5h, 6 Min., 7 Sek., 8 ms, 9 μs und 10 ns | -1h, 5 Sek. | 2 T, 3 Sek. und 40 ms"#,
            r#"fr narrow 1a 2m. 3sem. 4j 5h 6min 7s 8ms 9μs 10ns | -1h 5s | 2j 3s 40ms"#,
            r#"ja narrow 1y2m3w4d5h6m7s8ms9μs10ns | -1h5s | 2d3s40ms"#,
            r#"ar narrow 1 سنة وشهران و3 أ و4 ي و5 س و6 د و7 ث و8 ملي ث و9 م.ث. و10 ن.ث. | ‎-1 س و5 ث | 2 ي و3 ث و40 ملي ث"#,
            r#"da narrow 1 år, 2 m, 3 u, 4 d, 5 t, 6 m, 7 s, 8 ms, 9μs og 10ns | -1 t og 5 s | 2 d, 3 s og 40 ms"#,
            r#"pl narrow 1 r., 2 m-ce, 3 t., 4 d., 5 h, 6 min, 7 s, 8 ms, 9 μs i 10 ns | -1 h i 5 s | 2 d., 3 s i 40 ms"#,
            r#"en digital 1 yr, 2 mths, 3 wks, 4 days, 5:06:07.00800901 | -1:00:05 | 2 days, 0:00:03.04"#,
            r#"de digital 1 J, 2 Mon., 3 Wo., 4 Tg. und 5:06:07,00800901 | -1:00:05 | 2 Tg., 0:00:03,04"#,
            r#"fr digital 1 an, 2 m., 3 sem., 4 j et 5:06:07,00800901 | -1:00:05 | 2 j et 0:00:03,04"#,
            r#"ja digital 1 年 2 か月 3 週間 4 日 5:06:07.00800901 | -1:00:05 | 2 日 0:00:03.04"#,
            r#"ar digital سنة واحدة، وشهران، و3 أسابيع، و4 أيام، و5:06:07.00800901 | ‎-1:00:05 | يومان و0:00:03.04"#,
            r#"da digital 1 år, 2 mdr., 3 uger, 4 dage og 5.06.07,00800901 | -1.00.05 | 2 dage og 0.00.03,04"#,
            r#"pl digital 1 rok, 2 mies., 3 tyg., 4 dni i 5:06:07,00800901 | -1:00:05 | 2 dni i 0:00:03,04"#,
            r#"{"locale":"en","numberingSystem":"latn","style":"short","years":"short","yearsDisplay":"auto","months":"short","monthsDisplay":"auto","weeks":"short","weeksDisplay":"auto","days":"short","daysDisplay":"auto","hours":"short","hoursDisplay":"auto","minutes":"short","minutesDisplay":"auto","seconds":"short","secondsDisplay":"auto","milliseconds":"short","millisecondsDisplay":"auto","microseconds":"short","microsecondsDisplay":"auto","nanoseconds":"short","nanosecondsDisplay":"auto"}"#,
            r#"{"locale":"de-u-nu-arab","numberingSystem":"arab","style":"digital","years":"short","yearsDisplay":"auto","months":"short","monthsDisplay":"auto","weeks":"short","weeksDisplay":"auto","days":"short","daysDisplay":"auto","hours":"numeric","hoursDisplay":"always","minutes":"2-digit","minutesDisplay":"always","seconds":"2-digit","secondsDisplay":"always","milliseconds":"numeric","millisecondsDisplay":"auto","microseconds":"numeric","microsecondsDisplay":"auto","nanoseconds":"numeric","nanosecondsDisplay":"auto","fractionalDigits":3}"#,
            r#"{"locale":"en","numberingSystem":"latn","style":"long","years":"long","yearsDisplay":"auto","months":"long","monthsDisplay":"auto","weeks":"long","weeksDisplay":"auto","days":"long","daysDisplay":"auto","hours":"long","hoursDisplay":"auto","minutes":"long","minutesDisplay":"auto","seconds":"numeric","secondsDisplay":"always","milliseconds":"numeric","millisecondsDisplay":"auto","microseconds":"numeric","microsecondsDisplay":"auto","nanoseconds":"numeric","nanosecondsDisplay":"auto"}"#,
            r#"{"locale":"en","numberingSystem":"latn","style":"short","years":"short","yearsDisplay":"auto","months":"short","monthsDisplay":"auto","weeks":"short","weeksDisplay":"auto","days":"short","daysDisplay":"auto","hours":"numeric","hoursDisplay":"always","minutes":"2-digit","minutesDisplay":"always","seconds":"2-digit","secondsDisplay":"always","milliseconds":"numeric","millisecondsDisplay":"auto","microseconds":"numeric","microsecondsDisplay":"auto","nanoseconds":"numeric","nanosecondsDisplay":"auto"}"#,
            r#"{"locale":"en","numberingSystem":"latn","style":"short","years":"short","yearsDisplay":"auto","months":"short","monthsDisplay":"auto","weeks":"short","weeksDisplay":"auto","days":"short","daysDisplay":"auto","hours":"short","hoursDisplay":"auto","minutes":"short","minutesDisplay":"auto","seconds":"short","secondsDisplay":"auto","milliseconds":"numeric","millisecondsDisplay":"auto","microseconds":"numeric","microsecondsDisplay":"auto","nanoseconds":"numeric","nanosecondsDisplay":"auto"}"#,
            r#"1:02:03.45 -0:05:01.99"#,
            r#"1.5 seconds"#,
            r#"2 hr, 3:04"#,
            r#"1d 0h"#,
            r#"9,007,199,254,740,991 sec, 975 ms, 424 μs"#,
            r#"0:00:9007199254740991.975424"#,
            r#"0:00:00 """#,
            r#"-0 yrs, 1 day"#,
            r#"-1 day, 0:01:02"#,
            r#"[{"type":"integer","value":"1","unit":"hour"},{"type":"literal","value":":"},{"type":"integer","value":"02","unit":"minute"},{"type":"literal","value":":"},{"type":"integer","value":"03","unit":"second"},{"type":"decimal","value":".","unit":"second"},{"type":"fraction","value":"005","unit":"second"}]"#,
            r#"[{"type":"minusSign","value":"-","unit":"year"},{"type":"integer","value":"2","unit":"year"},{"type":"literal","value":" ","unit":"year"},{"type":"unit","value":"yrs","unit":"year"},{"type":"literal","value":", "},{"type":"integer","value":"1","unit":"hour"},{"type":"literal","value":" ","unit":"hour"},{"type":"unit","value":"hr","unit":"hour"}]"#,
            r#"๑๒:๓๔:๕๖"#,
            r#"١٢ hr"#,
            r#"TypeError TypeError TypeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError TypeError RangeError TypeError RangeError RangeError TypeError TypeError"#,
            r#"days,hours,ns,ns.valueOf,years"#,
            r#"true 2 Std."#,
        ])
    );
}

const FP4: &str = r#"const durations = [
  {seconds: 10_000_000, nanoseconds: 1},
  {seconds: 1, milliseconds: 2, microseconds: 3, nanoseconds: Number.MAX_SAFE_INTEGER},
  {milliseconds: 4503599627370497_000, microseconds: 4503599627370495_000000},
];
const digital = new Intl.DurationFormat("en", {style: "digital"});
const long = new Intl.DurationFormat("en", {style: "long", milliseconds: "numeric"});
const ms = new Intl.DurationFormat("en", {style: "narrow", microseconds: "numeric", fractionalDigits: 2});
for (const d of durations) {
  const neg = {};
  for (const k in d) neg[k] = -d[k];
  print(digital.format(d), "|", digital.format(neg), "|", long.format(d), "|", ms.format(neg));
}
const near = 9007199254740990926258176;
print(digital.format({nanoseconds: near}), digital.format({nanoseconds: -near}), digital.format({seconds: 9007199254740991, nanoseconds: 999999999}));
print(JSON.stringify(digital.formatToParts({nanoseconds: -near}).map(p => p.value)));
const r = [];
for (const d of [{seconds: 9007199254740991, nanoseconds: 1_000_000_000}, {nanoseconds: 9007199254740992000000000}, {days: 104249991375}, {days: 104249991374}, {days: -1969226660422097529995264, nanoseconds: -5140096494115884105728}]) {
  try { digital.format(d); r.push("ok"); } catch (e) { r.push(e.constructor.name); }
}
print(r.join(" "));
"#;

/// INTL-V2-FP4: the 3 durations of test262 `precision-exact-mathematical-values.js` (the third
/// is 9,007,199,254,740,991.975424 s), the same with a negative sign, in the digital style
/// (seconds), in a long style with numeric milliseconds (seconds) and a narrow style with
/// numeric microseconds (milliseconds, 2 digits, truncated); a nanoseconds field near 9e24
/// (the largest Number below 2**53 x 10^9: an `f64` division by 10^9 would give
/// 9007199254740991); the IsValidDuration bound, exact in nanoseconds.
#[test]
fn sub_second_sums_are_exact_near_9e24_ns() {
    assert_eq!(
        run_both(FP4),
        lines(&[
            r#"0:00:10000000.000000001 | -0:00:10000000.000000001 | 10,000,000.000000001 seconds | -10,000,000s 0.00ms"#,
            r#"0:00:9007200.256743991 | -0:00:9007200.256743991 | 9,007,200.256743991 seconds | -1s 9,007,199,256.74ms"#,
            r#"0:00:9007199254740991.975424 | -0:00:9007199254740991.975424 | 9,007,199,254,740,991.975424 seconds | -9,007,199,254,740,991,975.42ms"#,
            r#"0:00:9007199254740990.926258176 -0:00:9007199254740990.926258176 0:00:9007199254740991.999999999"#,
            r#"["-","0",":","00",":","9007199254740990",".","926258176"]"#,
            r#"RangeError RangeError RangeError ok RangeError"#,
        ])
    );
}
