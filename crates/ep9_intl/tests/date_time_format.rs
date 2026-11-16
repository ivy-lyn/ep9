//! Intl.DateTimeFormat (ECMA-402 §11): INTL W4 DTF-OPT, with and without GC stress. The
//! expected lines are node v26.13.1's output for the same script (`print` = `console.log` of
//! the arguments, each through `String`, joined by a space), except where test262 asks for
//! another result than V8 gives (each named at its test).

mod common;
use common::{lines, run_both};

const SHAPE: &str = r#"const D = Intl.DateTimeFormat;
print(typeof D, D.length, D.name, Object.getPrototypeOf(D) === Function.prototype, Object.getOwnPropertyNames(D).join());
const pn = Object.getOwnPropertyNames(D.prototype);
print(["constructor", "resolvedOptions", "formatToParts", "format"].every(k => pn.includes(k)), D.supportedLocalesOf.length, D.prototype.formatToParts.length, D.prototype.resolvedOptions.length);
const g = Object.getOwnPropertyDescriptor(D.prototype, "format");
print(typeof g.get, g.set, g.enumerable, g.configurable, g.get.name, g.get.length);
const dtf = new D("en", {timeZone: "UTC"});
print(Object.prototype.toString.call(dtf), Object.isExtensible(dtf), Object.getPrototypeOf(dtf) === D.prototype, dtf.format === dtf.format, dtf.format.name === "", dtf.format.length);
print(JSON.stringify(D.supportedLocalesOf(["de-CH", "zz", "en-u-ca-japanese"])));
const ro = (l, o) => JSON.stringify(new D(l, o).resolvedOptions());
print(ro("en", {timeZone: "UTC"}));
print(ro("de-u-hc-h11-ca-islamicc-nu-arab", {timeZone: "asia/calcutta", hour: "numeric", minute: "2-digit"}));
print(ro("en-u-hc-h23", {timeZone: "Etc/UTC", hourCycle: "h11", hour: "2-digit"}), ro("fr-u-hc-h24", {timeZone: "GMT", hour12: true, timeStyle: "short"}));
print(ro("en", {timeZone: "-0530", dateStyle: "long"}), ro("en", {timeZone: "+00", weekday: "long", era: "short", fractionalSecondDigits: 2, timeZoneName: "shortOffset"}));
print(ro("en", {timeZone: "UTC", calendar: "ISO8601", numberingSystem: "thai", month: "long"}), ro("en", {timeZone: "UTC", calendar: "islamic"}).includes('"calendar":"gregory"'));
const r = [];
function t(fn) { try { fn(); r.push("ok"); } catch (e) { r.push(e.constructor.name); } }
t(() => new D("en", null));
t(() => new D("en", {timeZone: "PST"}));
t(() => new D("en", {timeZone: "+24"}));
t(() => new D("en", {timeZone: "−05"}));
t(() => new D("en", {dateStyle: "full", hour: "numeric"}));
t(() => new D("en", {hourCycle: "h25"}));
t(() => new D("en", {fractionalSecondDigits: 4}));
t(() => new D("en", {calendar: "İSO8601"}));
t(() => dtf.format(NaN));
t(() => dtf.format(8.64e15 + 1));
t(() => dtf.formatToParts(Infinity));
t(() => D.prototype.formatToParts.call({}));
t(() => D.prototype.resolvedOptions.call(1));
t(() => Object.getOwnPropertyDescriptor(D.prototype, "format").get.call({}));
t(() => new Date(0).toLocaleDateString("en", {timeStyle: "short"}));
t(() => new Date(0).toLocaleTimeString("en", {dateStyle: "short"}));
t(() => Date.prototype.toLocaleString.call({}));
print(r.join(" "));
const log = [];
const opts = new Proxy({}, { get(o, k) { if (typeof k === "string") log.push(k); return undefined; } });
new D("en", opts);
print(log.join());
const o = Object.create(D.prototype);
const back = D.call(o, "en", {timeZone: "UTC"});
const syms = Object.getOwnPropertySymbols(o);
const desc = Object.getOwnPropertyDescriptor(o, syms[0]);
print(back === o, syms.length, syms[0].description, desc.writable, desc.enumerable, desc.configurable, Object.prototype.toString.call(desc.value));
print(D.prototype.resolvedOptions.call(o).timeZone, o.format(0), new D("en") === D.call(new D("en")));
const n = Object.create(Intl.NumberFormat.prototype);
const nb = Intl.NumberFormat.call(n, "en");
print(nb === n, Object.getOwnPropertySymbols(n)[0] === syms[0], Intl.NumberFormat.prototype.resolvedOptions.call(n).locale);
class S extends D {}
print(new S("de", {timeZone: "UTC"}) instanceof S, new S("de", {timeZone: "UTC", year: "numeric"}).format(0));
"#;

/// The shape of %Intl.DateTimeFormat% and its prototype, resolvedOptions (the "ca", "hc" and
/// "nu" keywords and options, hour12, offset and named zones, the components and styles),
/// the errors, the order of the option reads (CreateDateTimeFormat), the legacy constructor
/// path (%Intl%.[[FallbackSymbol]], also for Intl.NumberFormat) and a subclass.
///
/// Not node's lines (node v26.13.1 differs from test262): "Etc/UTC" and "GMT" stay as the
/// user wrote them (`canonicalize-utc-timezone.js`); "PST" and an offset with U+2212 are
/// RangeErrors (`timezone-legacy-non-iana.js`, `offset-timezone-no-unicode-minus-sign.js`);
/// the calendar "islamic" falls back to "gregory" (`constructor-options-calendar-islamic-
/// fallback.js`).
#[test]
fn the_constructor_options_and_legacy_path_give_the_expected_lines() {
    assert_eq!(
        run_both(SHAPE),
        lines(&[
            "function 0 DateTimeFormat true length,name,prototype,supportedLocalesOf",
            "true 1 1 0",
            "function undefined false true get format 0",
            "[object Intl.DateTimeFormat] true true true true 1",
            "[\"de-CH\",\"en-u-ca-japanese\"]",
            "{\"locale\":\"en\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"year\":\"numeric\",\"month\":\"numeric\",\"day\":\"numeric\"}",
            "{\"locale\":\"de-u-ca-islamic-civil-hc-h11-nu-arab\",\"calendar\":\"islamic-civil\",\"numberingSystem\":\"arab\",\"timeZone\":\"Asia/Calcutta\",\"hourCycle\":\"h11\",\"hour12\":true,\"hour\":\"numeric\",\"minute\":\"2-digit\"}",
            "{\"locale\":\"en\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"Etc/UTC\",\"hourCycle\":\"h11\",\"hour12\":true,\"hour\":\"2-digit\"} {\"locale\":\"fr\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"GMT\",\"hourCycle\":\"h12\",\"hour12\":true,\"timeStyle\":\"short\"}",
            "{\"locale\":\"en\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"-05:30\",\"dateStyle\":\"long\"} {\"locale\":\"en\",\"calendar\":\"gregory\",\"numberingSystem\":\"latn\",\"timeZone\":\"+00:00\",\"weekday\":\"long\",\"era\":\"short\",\"fractionalSecondDigits\":2,\"timeZoneName\":\"shortOffset\"}",
            "{\"locale\":\"en\",\"calendar\":\"iso8601\",\"numberingSystem\":\"thai\",\"timeZone\":\"UTC\",\"month\":\"long\"} true",
            "TypeError RangeError RangeError RangeError TypeError RangeError RangeError RangeError RangeError RangeError RangeError TypeError TypeError TypeError TypeError TypeError TypeError",
            "localeMatcher,calendar,numberingSystem,hour12,hourCycle,timeZone,weekday,era,year,month,day,dayPeriod,hour,minute,second,fractionalSecondDigits,timeZoneName,formatMatcher,dateStyle,timeStyle",
            "true 1 IntlLegacyConstructedSymbol false false false [object Intl.DateTimeFormat]",
            "UTC 1/1/1970 false",
            "true true en",
            "true 1970",
        ])
    );
}

const FORMAT: &str = r#"const d = Date.UTC(2026, 0, 2, 15, 4, 5, 678);
const f = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).format(x);
const p = (l, o, x = d) => new Intl.DateTimeFormat(l, Object.assign({timeZone: "UTC"}, o)).formatToParts(x).map(q => q.type + ":" + q.value).join("/");
print(f("en"), "|", f("en", {year: "numeric", month: "long"}), "|", f("en", {weekday: "long"}), "|", f("en", {month: "short", day: "numeric"}));
print(f("en", {hour: "numeric", minute: "2-digit", hourCycle: "h23"}), "|", f("en-GB", {timeStyle: "medium"}), "|", f("en", {timeZone: "America/New_York", hour: "numeric", minute: "numeric", hour12: false, timeZoneName: "short"}));
print(f("en", {timeZone: "Asia/Kolkata", hourCycle: "h23", hour: "numeric", minute: "numeric"}), "|", f("en", {timeZone: "+05:30", hourCycle: "h23", hour: "numeric", minute: "numeric"}), "|", f("en", {}, -1), "|", f("en", {}, 8.64e15));
print(f("en-GB", {dateStyle: "full"}), "|", f("fr", {dateStyle: "long"}), "|", f("ja", {dateStyle: "medium"}), "|", f("zh", {year: "numeric", month: "long", day: "numeric"}));
print(f("en-u-ca-japanese", {dateStyle: "long"}), "|", f("en", {calendar: "hebrew", dateStyle: "long"}), "|", f("ar-EG", {dateStyle: "short"}));
print(p("en"), "|", p("en-GB", {timeStyle: "medium"}));
print(p("en", {weekday: "long", year: "numeric", month: "long", day: "numeric"}), "|", p("fr", {dateStyle: "long"}));
const dd = new Date(d);
print(dd.toLocaleDateString("en", {timeZone: "UTC"}), "|", dd.toLocaleString("en-GB", {timeZone: "UTC"}), "|", dd.toLocaleTimeString("en-GB", {timeZone: "UTC"}), "|", new Date(NaN).toLocaleTimeString("en", {timeZone: "bad"}));
print(dd.toLocaleDateString("en", {timeZone: "UTC", hour: "numeric", hourCycle: "h23"}), "|", Date.prototype.toLocaleString.length);
const saved = Intl.DateTimeFormat; Intl.DateTimeFormat = function () { throw new Error("taint"); };
print(dd.toLocaleDateString("en", {timeZone: "UTC"})); Intl.DateTimeFormat = saved;
"#;

/// format, formatToParts and Date.prototype.toLocale{,Date,Time}String: components, styles,
/// calendars, numbering systems, named and offset zones, the TimeClip range ends, "Invalid
/// Date", the required/defaults table, and no read of `globalThis.Intl`
/// (`taint-Intl-DateTimeFormat.js`). The cases have no day period (`pattern::adjust` writes
/// U+0020 before it, as V8 does; tests/date_time_format_pattern.rs has those cases).
#[test]
fn format_and_to_locale_string_give_the_node_lines() {
    assert_eq!(
        run_both(FORMAT),
        lines(&[
            "1/2/2026 | January 2026 | Friday | Jan 2",
            "15:04 | 15:04:05 | 10:04 EST",
            "20:34 | 20:34 | 12/31/1969 | 9/13/275760",
            "Friday, 2 January 2026 | 2 janvier 2026 | 2026/01/02 | 2026年1月2日",
            "January 2, 8 Reiwa | 13 Tevet 5786 | ٢‏/١‏/٢٠٢٦",
            "month:1/literal://day:2/literal://year:2026 | hour:15/literal::/minute:04/literal::/second:05",
            "weekday:Friday/literal:, /month:January/literal: /day:2/literal:, /year:2026 | day:2/literal: /month:janvier/literal: /year:2026",
            "1/2/2026 | 02/01/2026, 15:04:05 | 15:04:05 | Invalid Date",
            "1/2/2026, 15 | 0",
            "1/2/2026",
        ])
    );
}
