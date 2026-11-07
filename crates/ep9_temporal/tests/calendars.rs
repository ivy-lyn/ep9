//! Non-ISO calendars (INTL-I1-D2; SPEC-T-D17 as changed for ECMA-402): AvailableCalendars,
//! the ISO date in the slots, the era fields of a property bag, the calendar checks, with and
//! without GC stress. Expected lines come from node 26 (V8 uses temporal_rs with ICU4X too);
//! the doc comment of a test names any line where V8 does not follow ECMA-402.

mod common;
use common::{lines, run_both};

/// CanonicalizeCalendar on every path (constructors, a property bag, `withCalendar`, a string
/// annotation): the 16 ids of AvailableCalendars and the aliases `islamicc` and
/// `ethiopic-amete-alem` give the canonical id (any case); `islamic` and `islamic-rgsa` are
/// RangeErrors (INTL-I1-D2, test262 `*/from/islamic*.js`), as are ids that no calendar has.
/// node 26 gives the same lines, except that V8 accepts `islamic` in a string annotation.
#[test]
fn available_calendar_ids_on_every_path() {
    assert_eq!(
        run_both(
            r#"var ids = ["buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory", "hebrew", "indian", "islamic-civil", "islamic-tbla", "islamic-umalqura", "iso8601", "japanese", "persian", "roc", "islamicc", "ethiopic-amete-alem", "GREGORY", "islamic", "islamic-rgsa", "gregorian", "julian", ""];
            for (var id of ids) {
              var row = [];
              function t(f) { try { row.push(f()); } catch (e) { row.push(e.constructor.name); } }
              t(() => new Temporal.PlainDate(2020, 1, 1, id).calendarId);
              t(() => Temporal.PlainDate.from({ year: 2020, monthCode: "M01", day: 1, calendar: id }).calendarId);
              t(() => new Temporal.PlainDate(2020, 1, 1).withCalendar(id).calendarId);
              t(() => Temporal.PlainDate.from("2020-01-01[u-ca=" + id + "]").calendarId);
              t(() => new Temporal.PlainDateTime(2020, 1, 1, 0, 0, 0, 0, 0, 0, id).calendarId);
              t(() => new Temporal.PlainYearMonth(2020, 1, id).calendarId);
              t(() => new Temporal.PlainMonthDay(1, 1, id).calendarId);
              t(() => new Temporal.ZonedDateTime(0n, "UTC", id).calendarId);
              t(() => Temporal.ZonedDateTime.from("2020-01-01T00:00[UTC][u-ca=" + id + "]").calendarId);
              print(JSON.stringify(id), row.join(" "));
            }"#
        ),
        lines(&[
            "\"buddhist\" buddhist buddhist buddhist buddhist buddhist buddhist buddhist buddhist buddhist",
            "\"chinese\" chinese chinese chinese chinese chinese chinese chinese chinese chinese",
            "\"coptic\" coptic coptic coptic coptic coptic coptic coptic coptic coptic",
            "\"dangi\" dangi dangi dangi dangi dangi dangi dangi dangi dangi",
            "\"ethioaa\" ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa",
            "\"ethiopic\" ethiopic ethiopic ethiopic ethiopic ethiopic ethiopic ethiopic ethiopic ethiopic",
            "\"gregory\" gregory gregory gregory gregory gregory gregory gregory gregory gregory",
            "\"hebrew\" hebrew hebrew hebrew hebrew hebrew hebrew hebrew hebrew hebrew",
            "\"indian\" indian indian indian indian indian indian indian indian indian",
            "\"islamic-civil\" islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil",
            "\"islamic-tbla\" islamic-tbla islamic-tbla islamic-tbla islamic-tbla islamic-tbla islamic-tbla islamic-tbla islamic-tbla islamic-tbla",
            "\"islamic-umalqura\" islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura islamic-umalqura",
            "\"iso8601\" iso8601 iso8601 iso8601 iso8601 iso8601 iso8601 iso8601 iso8601 iso8601",
            "\"japanese\" japanese japanese japanese japanese japanese japanese japanese japanese japanese",
            "\"persian\" persian persian persian persian persian persian persian persian persian",
            "\"roc\" roc roc roc roc roc roc roc roc roc",
            "\"islamicc\" islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil islamic-civil",
            "\"ethiopic-amete-alem\" ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa ethioaa",
            "\"GREGORY\" gregory gregory gregory gregory gregory gregory gregory gregory gregory",
            "\"islamic\" RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError",
            "\"islamic-rgsa\" RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError",
            "\"gregorian\" RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError",
            "\"julian\" RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError",
            "\"\" RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError RangeError",
        ])
    );
}

/// The calendar fields of one ISO date in each calendar, and the ISO date the slots keep: the
/// ISO date comes back unchanged through `withCalendar("iso8601")`, PlainYearMonth and
/// PlainMonthDay keep their ISO reference date, and the fields round-trip through a property
/// bag. The same lines as node 26.
#[test]
fn calendar_fields_and_the_iso_date_slots() {
    assert_eq!(
        run_both(
            r#"var ids = ["buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory", "hebrew", "indian", "islamic-civil", "islamic-tbla", "islamic-umalqura", "iso8601", "japanese", "persian", "roc"];
            for (var id of ids) {
              var d = Temporal.PlainDate.from("2024-03-15[u-ca=" + id + "]");
              var ym = d.toPlainYearMonth();
              var md = d.toPlainMonthDay();
              var dt = d.toPlainDateTime("12:00");
              var z = d.toZonedDateTime("UTC");
              print(id, d.era, d.eraYear, d.year, d.month, d.monthCode, d.day, d.dayOfYear, d.daysInMonth, d.daysInYear, d.monthsInYear, d.inLeapYear, d.weekOfYear,
                d.withCalendar("iso8601").toString(), ym.toString(), md.toString(), dt.year, z.monthCode,
                Temporal.PlainDate.from({ year: d.year, monthCode: d.monthCode, day: d.day, calendar: id }).equals(d),
                d.add({ months: 13 }).toString(), d.until(d.add({ years: 1, days: 3 }), { largestUnit: "years" }).toString());
            }"#
        ),
        lines(&[
            "buddhist be 2567 2567 3 M03 15 75 31 366 12 true undefined 2024-03-15 2024-03-01[u-ca=buddhist] 1972-03-15[u-ca=buddhist] 2567 M03 true 2025-04-15[u-ca=buddhist] P1Y3D",
            "chinese undefined undefined 2024 2 M02 6 35 30 354 12 false undefined 2024-03-15 2024-03-10[u-ca=chinese] 1972-03-20[u-ca=chinese] 2024 M02 true 2025-04-03[u-ca=chinese] P1Y3D",
            "coptic am 1740 1740 7 M07 6 186 30 365 13 false undefined 2024-03-15 2024-03-10[u-ca=coptic] 1972-03-15[u-ca=coptic] 1740 M07 true 2025-03-15[u-ca=coptic] P1Y3D",
            "dangi undefined undefined 2024 2 M02 6 35 30 354 12 false undefined 2024-03-15 2024-03-10[u-ca=dangi] 1972-03-20[u-ca=dangi] 2024 M02 true 2025-04-03[u-ca=dangi] P1Y3D",
            "ethioaa aa 7516 7516 7 M07 6 186 30 365 13 false undefined 2024-03-15 2024-03-10[u-ca=ethioaa] 1972-03-15[u-ca=ethioaa] 7516 M07 true 2025-03-15[u-ca=ethioaa] P1Y3D",
            "ethiopic am 2016 2016 7 M07 6 186 30 365 13 false undefined 2024-03-15 2024-03-10[u-ca=ethiopic] 1972-03-15[u-ca=ethiopic] 2016 M07 true 2025-03-15[u-ca=ethiopic] P1Y3D",
            "gregory ce 2024 2024 3 M03 15 75 31 366 12 true undefined 2024-03-15 2024-03-01[u-ca=gregory] 1972-03-15[u-ca=gregory] 2024 M03 true 2025-04-15[u-ca=gregory] P1Y3D",
            "hebrew am 5784 5784 7 M06 5 182 29 383 13 true undefined 2024-03-15 2024-03-11[u-ca=hebrew] 1972-02-20[u-ca=hebrew] 5784 M06 true 2025-04-03[u-ca=hebrew] P1Y3D",
            "indian shaka 1945 1945 12 M12 25 360 30 365 12 false undefined 2024-03-15 2024-02-20[u-ca=indian] 1972-03-15[u-ca=indian] 1945 M12 true 2025-04-15[u-ca=indian] P1Y3D",
            "islamic-civil ah 1445 1445 9 M09 5 241 30 355 12 true undefined 2024-03-15 2024-03-11[u-ca=islamic-civil] 1972-10-13[u-ca=islamic-civil] 1445 M09 true 2025-04-04[u-ca=islamic-civil] P1Y3D",
            "islamic-tbla ah 1445 1445 9 M09 6 242 30 355 12 true undefined 2024-03-15 2024-03-10[u-ca=islamic-tbla] 1972-10-13[u-ca=islamic-tbla] 1445 M09 true 2025-04-04[u-ca=islamic-tbla] P1Y3D",
            "islamic-umalqura ah 1445 1445 9 M09 5 241 30 354 12 false undefined 2024-03-15 2024-03-11[u-ca=islamic-umalqura] 1972-10-13[u-ca=islamic-umalqura] 1445 M09 true 2025-04-03[u-ca=islamic-umalqura] P1Y3D",
            "iso8601 undefined undefined 2024 3 M03 15 75 31 366 12 true 11 2024-03-15 2024-03 03-15 2024 M03 true 2025-04-15 P1Y3D",
            "japanese reiwa 6 2024 3 M03 15 75 31 366 12 true undefined 2024-03-15 2024-03-01[u-ca=japanese] 1972-03-15[u-ca=japanese] 2024 M03 true 2025-04-15[u-ca=japanese] P1Y3D",
            "persian ap 1402 1402 12 M12 25 361 29 365 12 false undefined 2024-03-15 2024-02-20[u-ca=persian] 1972-03-15[u-ca=persian] 1402 M12 true 2025-04-14[u-ca=persian] P1Y3D",
            "roc roc 113 113 3 M03 15 75 31 366 12 true undefined 2024-03-15 2024-03-01[u-ca=roc] 1972-03-15[u-ca=roc] 113 M03 true 2025-04-15[u-ca=roc] P1Y3D",
        ])
    );
}

/// Chinese and Dangi dates across the Temporal range, inside and outside the baked year table
/// (1900-2102) of icu_calendar: its integer model gives each year (INTL-I1-D2; no astronomy
/// code runs, `tools/calprobe`). The same lines as node 26.
#[test]
fn chinese_and_dangi_across_the_temporal_range() {
    assert_eq!(
        run_both(
            r#"for (var y of [-271820, -10000, 1, 1800, 1899, 1900, 1912, 2050, 2101, 2200, 3000, 10000, 275759]) {
              for (var id of ["chinese", "dangi"]) {
                var iso = new Temporal.PlainDate(y, 6, 15);
                var d = iso.withCalendar(id);
                print(id, y, d.year, d.monthCode, d.day, d.monthsInYear, d.daysInYear, d.inLeapYear,
                  Temporal.PlainDate.from({ year: d.year, monthCode: d.monthCode, day: d.day, calendar: id }).withCalendar("iso8601").toString());
              }
            }"#
        ),
        lines(&[
            "chinese -271820 -271820 M04 26 13 384 true -271820-06-15",
            "dangi -271820 -271820 M04 26 13 384 true -271820-06-15",
            "chinese -10000 -10000 M05 4 12 354 false -010000-06-15",
            "dangi -10000 -10000 M05 4 12 354 false -010000-06-15",
            "chinese 1 1 M05 8 12 354 false 0001-06-15",
            "dangi 1 1 M05 8 12 354 false 0001-06-15",
            "chinese 1800 1800 M04L 23 13 384 true 1800-06-15",
            "dangi 1800 1800 M04L 23 13 384 true 1800-06-15",
            "chinese 1899 1899 M05 7 12 355 false 1899-06-15",
            "dangi 1899 1899 M05 7 12 355 false 1899-06-15",
            "chinese 1900 1900 M05 19 13 384 true 1900-06-15",
            "dangi 1900 1900 M05 19 13 384 true 1900-06-15",
            "chinese 1912 1912 M05 1 12 354 false 1912-06-15",
            "dangi 1912 1912 M05 1 12 354 false 1912-06-15",
            "chinese 2050 2050 M04 26 13 384 true 2050-06-15",
            "dangi 2050 2050 M04 26 13 384 true 2050-06-15",
            "chinese 2101 2101 M05 19 13 384 true 2101-06-15",
            "dangi 2101 2101 M05 19 13 384 true 2101-06-15",
            "chinese 2200 2200 M05 3 12 354 false 2200-06-15",
            "dangi 2200 2200 M05 3 12 354 false 2200-06-15",
            "chinese 3000 3000 M05 21 13 384 true 3000-06-15",
            "dangi 3000 3000 M05 21 13 384 true 3000-06-15",
            "chinese 10000 10000 M05 20 13 384 true +010000-06-15",
            "dangi 10000 10000 M05 20 13 384 true +010000-06-15",
            "chinese 275759 275759 M05 17 13 384 true +275759-06-15",
            "dangi 275759 275759 M05 17 13 384 true +275759-06-15",
        ])
    );
}

/// PrepareCalendarFields reads `era` and `eraYear` (CalendarExtraFields) in alphabetical order
/// for a calendar with eras, and not for `iso8601` or `chinese`; property bags with eras on
/// every type and in relativeTo. The same lines as node 26.
#[test]
fn era_fields_are_read_for_calendars_with_eras() {
    assert_eq!(
        run_both(
            r#"var log = [];
            var bag = new Proxy({ era: "bce", eraYear: 5, month: 3, day: 2, calendar: "gregory" }, {
              get(t, k) { if (typeof k === "string") log.push(k); return t[k]; },
              has(t, k) { return k in t; },
            });
            var d = Temporal.PlainDate.from(bag);
            print(d.toString(), d.year, d.era, d.eraYear, log.join(","));
            log.length = 0;
            var bag2 = new Proxy({ year: 2020, month: 3, day: 2 }, {
              get(t, k) { if (typeof k === "string") log.push(k); return t[k]; },
            });
            Temporal.PlainDate.from(bag2);
            print(log.join(","));
            log.length = 0;
            var cn = new Proxy({ year: 2020, month: 3, day: 2, calendar: "chinese" }, {
              get(t, k) { if (typeof k === "string") log.push(k); return t[k]; },
            });
            print(Temporal.PlainDate.from(cn).toString(), log.join(","));
            var r = [];
            function t(f) { try { r.push(String(f())); } catch (e) { r.push(e.constructor.name); } }
            t(() => Temporal.PlainDate.from({ era: "ce", eraYear: 2020, monthCode: "M02", day: 29, calendar: "gregory" }).toString());
            t(() => Temporal.PlainDate.from({ era: "ce", month: 2, day: 29, calendar: "gregory" }));
            t(() => Temporal.PlainDate.from({ era: "xyz", eraYear: 1, month: 2, day: 1, calendar: "gregory" }));
            t(() => Temporal.PlainDate.from({ era: "reiwa", eraYear: 2, month: 5, day: 1, calendar: "japanese" }).toString());
            t(() => new Temporal.PlainDate(2020, 5, 1, "japanese").with({ eraYear: 1 }).toString());
            t(() => new Temporal.PlainDate(2020, 5, 1, "japanese").with({ era: "heisei", eraYear: 1 }).toString());
            t(() => Temporal.PlainYearMonth.from({ era: "roc", eraYear: 100, month: 7, calendar: "roc" }).toString());
            t(() => new Temporal.PlainYearMonth(2020, 7, "roc").with({ eraYear: 1 }).toString());
            t(() => Temporal.PlainMonthDay.from({ monthCode: "M05L", day: 1, calendar: "chinese" }).toString());
            t(() => Temporal.PlainMonthDay.from({ era: "am", eraYear: 5784, monthCode: "M05L", day: 1, calendar: "hebrew" }).toString());
            t(() => Temporal.PlainMonthDay.from({ monthCode: "M02", day: 30, calendar: "hebrew" }).toPlainDate({ era: "am", eraYear: 5785 }).toString());
            t(() => Temporal.PlainDateTime.from({ era: "be", eraYear: 2563, month: 1, day: 1, hour: 5, calendar: "buddhist" }).toString());
            t(() => Temporal.ZonedDateTime.from({ era: "ah", eraYear: 1445, month: 1, day: 1, timeZone: "UTC", calendar: "islamic-civil" }).toString());
            t(() => new Temporal.ZonedDateTime(0n, "UTC", "persian").with({ eraYear: 1400 }).toString());
            t(() => Temporal.Duration.from("P1M").total({ unit: "days", relativeTo: { era: "am", eraYear: 5784, monthCode: "M05L", day: 1, calendar: "hebrew" } }));
            print(r.join(" | "));"#
        ),
        lines(&[
            "-000004-03-02[u-ca=gregory] -4 bce 5 calendar,day,era,eraYear,month,monthCode,year",
            "calendar,day,month,monthCode,year",
            "2020-03-25[u-ca=chinese] calendar,day,month,monthCode,year",
            "2020-02-29[u-ca=gregory] | TypeError | RangeError | 2020-05-01[u-ca=japanese] | TypeError | 1989-05-01[u-ca=japanese] | 2011-07-01[u-ca=roc] | TypeError | 1971-06-23[u-ca=chinese] | 1970-02-07[u-ca=hebrew] | 2024-12-01[u-ca=hebrew] | 2020-01-01T05:00:00[u-ca=buddhist] | 2023-07-19T00:00:00+00:00[UTC][u-ca=islamic-civil] | TypeError | 30",
        ])
    );
}

/// `until` and `since` throw for two calendars before they read the options (CalendarEquals);
/// `equals` compares the calendar too. A string's calendar is checked before the options: an
/// available one reads them (then the range check throws), `islamic` and `islamic-rgsa` throw
/// first. node 26 gives the same lines, except that V8 accepts `islamic` in a string.
#[test]
fn calendar_mismatch_and_string_calendar_order() {
    assert_eq!(
        run_both(
            r#"var r = [];
            function t(f) { var n = 0; var o = { get largestUnit() { n++; return undefined; } }; try { r.push(String(f(o))); } catch (e) { r.push(e.constructor.name + n); } }
            var g = new Temporal.PlainDate(2020, 1, 1, "gregory"), i = new Temporal.PlainDate(2020, 1, 1);
            t((o) => g.until(i, o));
            t((o) => g.since(i, o));
            t((o) => g.until(g, o));
            t((o) => g.toPlainDateTime().until(i.toPlainDateTime(), o));
            t((o) => g.toPlainDateTime().since(i.toPlainDateTime(), o));
            t((o) => g.toPlainYearMonth().until(i.toPlainYearMonth(), o));
            t((o) => g.toPlainYearMonth().since(i.toPlainYearMonth(), o));
            t((o) => g.toZonedDateTime("UTC").until(i.toZonedDateTime("UTC"), o));
            t((o) => g.toZonedDateTime("UTC").since(i.toZonedDateTime("UTC"), o));
            t((o) => g.equals(i));
            t((o) => g.toPlainMonthDay().equals(i.toPlainMonthDay()));
            t((o) => g.toPlainMonthDay().equals(new Temporal.PlainMonthDay(1, 1, "gregory")));
            t((o) => g.toPlainYearMonth().equals(new Temporal.PlainYearMonth(2020, 1, "gregory", 1)));
            print(r.join(" "));
            r = [];
            function u(C, s) { var n = 0; try { C.from(s, { get overflow() { n++; return "constrain"; } }); r.push("ok" + n); } catch (e) { r.push(e.constructor.name + n); } }
            u(Temporal.PlainDate, "+999999-01-01[u-ca=gregory]");
            u(Temporal.PlainDate, "2020-01-01[u-ca=hebrew]");
            u(Temporal.PlainDate, "2020-01-01[u-ca=islamic]");
            u(Temporal.PlainDateTime, "-271821-04-19T00:00[u-ca=japanese]");
            u(Temporal.PlainYearMonth, "+999999-01-01[u-ca=coptic]");
            u(Temporal.PlainYearMonth, "2020-01-01[u-ca=coptic]");
            u(Temporal.PlainMonthDay, "2020-01-01[u-ca=coptic]");
            u(Temporal.PlainMonthDay, "2020-01-01[u-ca=islamic-rgsa]");
            print(r.join(" "));"#
        ),
        lines(&[
            "RangeError0 RangeError0 PT0S RangeError0 RangeError0 RangeError0 RangeError0 RangeError0 RangeError0 false false true true",
            "RangeError1 ok1 RangeError0 RangeError1 RangeError1 ok1 ok1 RangeError0",
        ])
    );
}

/// PlainMonthDay.prototype.toPlainDate merges the monthCode and day of the month-day with the
/// year, era and eraYear of the input, and CalendarDateFromFields resolves them as one bag
/// (NonISOResolveFields of the Intl era and monthCode proposal): an unknown era and a year
/// that era and eraYear do not give are RangeErrors, era without eraYear (or the reverse) is
/// a TypeError, as for Temporal.PlainDate.from. node 26 (temporal_rs 0.2) keeps the year and
/// drops era and eraYear, so it gives "2020-01-01" on lines 1-3 and 10-11; the expected lines
/// follow the spec text. The ISO calendar reads no era fields.
#[test]
fn month_day_to_plain_date_resolves_era_fields_with_the_year() {
    assert_eq!(
        run_both(
            r#"var g = Temporal.PlainMonthDay.from({ calendar: "gregory", monthCode: "M01", day: 1 });
            var j = Temporal.PlainMonthDay.from({ calendar: "japanese", monthCode: "M01", day: 1 });
            var iso = new Temporal.PlainMonthDay(2, 29);
            var cases = [
              () => g.toPlainDate({ year: 2020, era: "ce", eraYear: 1 }),
              () => g.toPlainDate({ year: 2020, era: "xyz", eraYear: 1 }),
              () => j.toPlainDate({ year: 2020, era: "heisei", eraYear: 1 }),
              () => g.toPlainDate({ year: 2020, era: "ce", eraYear: 2020 }),
              () => g.toPlainDate({ year: -5, era: "bce", eraYear: 6 }),
              () => g.toPlainDate({ era: "bce", eraYear: 6 }),
              () => j.toPlainDate({ era: "reiwa", eraYear: 2 }),
              () => j.toPlainDate({ era: "heisei", eraYear: 32 }),
              () => g.toPlainDate({ year: 2021 }),
              () => g.toPlainDate({ year: 2020, era: "ce" }),
              () => g.toPlainDate({ year: 2020, eraYear: 2020 }),
              () => g.toPlainDate({}),
              () => iso.toPlainDate({ year: 2021 }),
              () => iso.toPlainDate({ year: 2020, era: "xyz", eraYear: 1 }),
              () => Temporal.PlainDate.from({ calendar: "gregory", year: 2020, era: "ce", eraYear: 1, monthCode: "M01", day: 1 }),
            ];
            for (var f of cases) { try { print(f().toString()); } catch (e) { print(e.constructor.name); } }"#
        ),
        lines(&[
            "RangeError",
            "RangeError",
            "RangeError",
            "2020-01-01[u-ca=gregory]",
            "-000005-01-01[u-ca=gregory]",
            "-000005-01-01[u-ca=gregory]",
            "2020-01-01[u-ca=japanese]",
            "2020-01-01[u-ca=japanese]",
            "2021-01-01[u-ca=gregory]",
            "TypeError",
            "TypeError",
            "TypeError",
            "2021-02-28",
            "2020-02-29",
            "RangeError",
        ])
    );
}

/// Temporal.PlainMonthDay.from(string, options) (ToTemporalMonthDay steps 4-12): the syntax
/// and the calendar id first, then GetOptionsObject and the overflow option, and only then
/// the RangeError of a date outside the Temporal limits (a non-ISO month-day string keeps its
/// year), in the form of PlainYearMonth.from. So `null` options give a TypeError, and the
/// overflow getter runs before the RangeError. node 26 gives the same lines.
#[test]
fn month_day_from_string_reads_the_options_before_the_limits_error() {
    assert_eq!(
        run_both(
            r#"for (var s of ["+999999-01-01[u-ca=gregory]", "+999999-01-01[u-ca=japanese]", "-271821-04-18[u-ca=gregory]", "-271821-04-19[u-ca=gregory]", "01-01", "2020-13-01[u-ca=gregory]", "2020-01-01[u-ca=julian]"]) {
              var row = [];
              var log = [];
              var opts = { get overflow() { log.push("overflow"); return "constrain"; } };
              function t(f) { try { row.push(f().toString()); } catch (e) { row.push(e.constructor.name); } }
              t(() => Temporal.PlainMonthDay.from(s, opts));
              t(() => Temporal.PlainMonthDay.from(s, null));
              print(s, row.join(" "), log.join());
            }"#
        ),
        lines(&[
            "+999999-01-01[u-ca=gregory] RangeError TypeError overflow",
            "+999999-01-01[u-ca=japanese] RangeError TypeError overflow",
            "-271821-04-18[u-ca=gregory] RangeError TypeError overflow",
            "-271821-04-19[u-ca=gregory] 1972-04-19[u-ca=gregory] TypeError overflow",
            "01-01 01-01 TypeError overflow",
            "2020-13-01[u-ca=gregory] RangeError RangeError ",
            "2020-01-01[u-ca=julian] RangeError RangeError ",
        ])
    );
}

/// The month count of a chinese or dangi difference (largestUnit "months") on every entry point
/// that reaches icu_calendar `until`: PlainDate until and since, PlainYearMonth until,
/// ZonedDateTime until, Duration total and round with a PlainDate and a ZonedDateTime
/// relativeTo. icu_calendar 2.3.0 found it with one NonISODateSurpasses check per month past a
/// lower bound that is about 0.035 months per year too low, each check O(years): O(years^2),
/// 2.2-3.4 s per call over 20,000 years in a release build, with no poll. The patch of
/// third_party/patches/icu4x does a binary search over the same check: the same results (the
/// lines are those of the unpatched code and of node 26), 29-91 ms per call in a release build.
/// The 120 s bound covers a slow debug build (about 15 s; the unpatched code: minutes).
#[test]
fn chinese_and_dangi_until_in_months_over_20000_years() {
    let t = std::time::Instant::now();
    assert_eq!(
        common::run(
            r#"var a = Temporal.PlainDate.from("2000-01-15").withCalendar("chinese"), b = Temporal.PlainDate.from("+022000-06-20").withCalendar("chinese");
            var za = a.toZonedDateTime("UTC"), zb = b.toZonedDateTime("UTC"), d = Temporal.Duration.from({ days: 7300000 });
            print(a.until(b, { largestUnit: "months" }), a.since(b, { largestUnit: "months" }),
              a.toPlainYearMonth().until(b.toPlainYearMonth(), { largestUnit: "months" }), za.until(zb, { largestUnit: "months" }),
              d.total({ unit: "months", relativeTo: a }), d.round({ largestUnit: "months", relativeTo: a }),
              d.round({ largestUnit: "months", relativeTo: za }));
            print(a.withCalendar("dangi").until(b.withCalendar("dangi"), { largestUnit: "months" }));"#,
            false
        ),
        lines(&[
            "P247370M26D -P247370M26D P247371M P247370M26D 247201.3103448276 P247201M9D P247201M9D",
            "P247370M26D",
        ])
    );
    let took = t.elapsed();
    assert!(took < std::time::Duration::from_secs(120), "{took:?}");
}

/// The binary search gives the month count of the spec loop (NonISODateUntil steps 5-6): for
/// seeded random chinese and dangi pairs up to 40 years apart, `until` with largestUnit
/// "months" equals a JS walk that adds one month at a time to the first day of the start month
/// and stops before the first count whose (year, month, start day) passes the end date. The
/// evidence run of the INTL W1 fixer r3 compared 446 pairs per calendar with the unpatched
/// icu_calendar (0 differences).
#[test]
fn chinese_and_dangi_month_counts_equal_the_spec_loop() {
    assert_eq!(
        common::run(
            r#"var seed = 12345, bad = 0, n = 0;
            function rnd(k) { seed = (seed * 1103515245 + 12345) >>> 0; return Math.floor((seed / 4294967296) * k); }
            function pick(cal, y) {
              var m = 1 + rnd(Temporal.PlainDate.from({ year: y, month: 1, day: 1, calendar: cal }).monthsInYear);
              var first = Temporal.PlainDate.from({ year: y, month: m, day: 1, calendar: cal });
              return first.with({ day: 1 + rnd(first.daysInMonth) });
            }
            function passes(sign, y, m, d, b) {
              var c = y !== b.year ? y - b.year : m !== b.month ? m - b.month : d - b.day;
              return sign * c > 0;
            }
            for (var cal of ["chinese", "dangi"]) {
              for (var i = 0; i < 24; i++) {
                var y = 1700 + rnd(600), a = pick(cal, y), b = pick(cal, y + rnd(81) - 40);
                var sign = Temporal.PlainDate.compare(b, a), months = 0;
                if (sign !== 0) {
                  var start = a.with({ day: 1 });
                  for (var k = sign; ; k += sign) {
                    var s = start.add({ months: k });
                    if (passes(sign, s.year, s.month, a.day, b)) break;
                    months = k;
                  }
                }
                n++;
                if (a.until(b, { largestUnit: "months" }).months !== months) bad++;
              }
            }
            print(n, bad);"#,
            false
        ),
        lines(&["48 0"])
    );
}

/// A 100,000-year month difference with a termination pending: icu_calendar has no poll, so
/// the call must end on its own, and with the binary search it does (O(log(months)) checks).
/// The unpatched loop is O(years^2): it did not end in the 60 s wait.
#[test]
fn a_long_month_difference_ends_with_a_termination_pending() {
    for call in [
        "new Temporal.PlainDate(1, 1, 1, 'chinese').until(new Temporal.PlainDate(100000, 1, 1, 'chinese'), { largestUnit: 'months' }).toString()",
        "new Temporal.PlainDate(100000, 1, 1, 'dangi').toPlainYearMonth().since(new Temporal.PlainDate(1, 1, 1, 'dangi').toPlainYearMonth(), { largestUnit: 'months' }).toString()",
    ] {
        assert_eq!(
            common::terminate_during(call),
            Some((
                false,
                Some("reached".to_string()),
                Some("again".to_string())
            )),
            "{call}"
        );
    }
}

/// The full Temporal range with largestUnit "months" (6,772,638 months), with a termination
/// pending: icu_calendar has no poll, so the call must end on its own. With the binary search
/// it ends after O(log(months)) checks (2.2-3.4 s in a release build); the unpatched loop ran
/// for more than 120 s. The run is in release builds only: a debug build of icu_calendar takes
/// several minutes for one call.
#[test]
#[cfg_attr(debug_assertions, ignore = "release only (cargo test --release)")]
fn the_full_range_month_difference_ends_with_a_termination_pending() {
    for call in [
        "new Temporal.PlainDate(-271821, 4, 19, 'chinese').until(new Temporal.PlainDate(275760, 9, 13, 'chinese'), { largestUnit: 'months' }).toString()",
        "new Temporal.PlainDate(275760, 9, 13, 'dangi').since(new Temporal.PlainDate(-271821, 4, 19, 'dangi'), { largestUnit: 'months' }).toString()",
    ] {
        let t = std::time::Instant::now();
        let r = common::terminate_during(call);
        let took = t.elapsed();
        assert!(r.is_some(), "not done in 60 s: {call}");
        assert!(took < std::time::Duration::from_secs(30), "{took:?}");
    }
}
