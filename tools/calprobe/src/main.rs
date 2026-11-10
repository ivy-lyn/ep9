//! calprobe (INTL-I1-D2, H36): counts the calls of `core_maths` (the float methods that reach
//! `libm`) and of the astronomy code of `calendrical_calculations` on the calendar paths that
//! ep9 reaches through temporal_rs and icu_calendar. `run.sh` builds this crate with counting
//! copies of the two crates (feature `instr`, made by `instr.py`).
//!
//! ```text
//! mode      what runs                                                         expected
//! icu       18 icu_calendar kinds x 24,580 ISO dates: year, day, days in       0 calls
//!           year, months in year
//! temporal  the 17 ids that temporal_rs accepts (ep9 accepts 16 of them)       0 calls
//!           x 297 ISO years: every getter, fields -> date, year-month and
//!           month-day for 26 month codes x 2 days x 2 overflows, era fields,
//!           `with`, add and subtract of 6 durations, until and since in 4
//!           units, string round trips, the PlainMonthDay reference-year search
//! control   the opt-in China::gb_t_33661_2017 and                              sin, cos ... > 0
//!           Korea::adapted_gb_t_33661_2017 (astronomy; ep9 never calls them)
//! ```
//!
//! Each mode prints `TOTAL <mode> core_maths=<n> astronomy=<m> sin=<s>` and a checksum of the
//! results (equal checksums at two icu_calendar versions mean equal results).

use std::hint::black_box;
use std::str::FromStr;

use icu_calendar::{AnyCalendar, AnyCalendarKind, Date};
use temporal_rs::options::{DifferenceSettings, Overflow, Unit};
use temporal_rs::fields::{CalendarFields, YearMonthCalendarFields};
use temporal_rs::partial::{PartialDate, PartialYearMonth};
use temporal_rs::{
    Calendar, Duration, MonthCode, PlainDate, PlainMonthDay, PlainYearMonth, TinyAsciiStr,
};

/// The counts since the last call: (core_maths calls, astronomy calls, `sin` calls). An
/// astronomy call is a call of a function in `astronomy.rs` or `chinese_based.rs` of
/// calendrical_calculations.
#[cfg(feature = "instr")]
fn take(tag: &str) -> (u64, u64, u64) {
    let (mut m, mut a, mut sin) = (0u64, 0u64, 0u64);
    let mut ms = Vec::new();
    core_maths::__probe::each(&mut |n, v| {
        m += v;
        if n.ends_with(":sin") {
            sin += v;
        }
        ms.push(format!("{n}={v}"));
    });
    let mut astro = Vec::new();
    calendrical_calculations::__probe::each(&mut |n, v| {
        if n.starts_with("astronomy.rs") || n.starts_with("chinese_based.rs") {
            a += v;
            astro.push(format!("{n}={v}"));
        }
    });
    if m != 0 || a != 0 {
        println!("  [{tag}] core_maths: {}", ms.join(" "));
        println!("  [{tag}] astronomy: {}", astro.join(" "));
    }
    core_maths::__probe::reset();
    calendrical_calculations::__probe::reset();
    (m, a, sin)
}

/// Without `instr` there is nothing to count (a plain build, for `nm` and `objdump`).
#[cfg(not(feature = "instr"))]
fn take(_tag: &str) -> (u64, u64, u64) {
    (0, 0, 0)
}

/// Sums of [`take`] over a mode.
#[derive(Default)]
struct Total {
    core_maths: u64,
    astronomy: u64,
    sin: u64,
}

impl Total {
    fn add(&mut self, (m, a, s): (u64, u64, u64)) {
        self.core_maths += m;
        self.astronomy += a;
        self.sin += s;
    }

    fn print(&self, mode: &str, checksum: i64) {
        println!(
            "TOTAL {mode} core_maths={} astronomy={} sin={} checksum={checksum}",
            self.core_maths, self.astronomy, self.sin
        );
    }
}

/// The calendar ids that temporal_rs 0.2.6 accepts: ep9's AvailableCalendars (16) and
/// `islamic`, which ep9 rejects before temporal_rs (it maps to `islamic-civil` there).
const IDS: [&str; 17] = [
    "buddhist",
    "chinese",
    "coptic",
    "dangi",
    "ethioaa",
    "ethiopic",
    "gregory",
    "hebrew",
    "indian",
    "islamic-civil",
    "islamic-tbla",
    "islamic-umalqura",
    "islamic",
    "iso8601",
    "japanese",
    "persian",
    "roc",
];

/// Every ISO year -3000..=3000, every 97th year over the Temporal range, and the range edges.
fn dense_years() -> Vec<i32> {
    let mut v: Vec<i32> = (-3000..=3000).collect();
    v.extend((-271_821..=275_760).step_by(97));
    v.extend([-271_821, -271_820, 275_759, 275_760]);
    v.sort_unstable();
    v.dedup();
    v
}

/// Mode `icu`: icu_calendar directly, every kind (the 16 of ep9, Julian and the simulated
/// Hijri calendar that no id reaches).
fn icu() {
    let kinds = [
        AnyCalendarKind::Buddhist,
        AnyCalendarKind::Chinese,
        AnyCalendarKind::Coptic,
        AnyCalendarKind::Dangi,
        AnyCalendarKind::Ethiopian,
        AnyCalendarKind::EthiopianAmeteAlem,
        AnyCalendarKind::Gregorian,
        AnyCalendarKind::Hebrew,
        AnyCalendarKind::Indian,
        AnyCalendarKind::HijriTabularTypeIIFriday,
        AnyCalendarKind::HijriTabularTypeIIThursday,
        AnyCalendarKind::HijriUmmAlQura,
        AnyCalendarKind::Iso,
        AnyCalendarKind::Japanese,
        AnyCalendarKind::Persian,
        AnyCalendarKind::Roc,
        AnyCalendarKind::Julian,
        AnyCalendarKind::HijriSimulatedMecca,
    ];
    let years = dense_years();
    let mut total = Total::default();
    let mut sum = 0i64;
    for k in kinds {
        let mut n = 0u64;
        for &y in &years {
            for m in [1u8, 4, 7, 10] {
                let Ok(iso) = Date::try_new_iso(y, m, 15) else {
                    continue;
                };
                let d = iso.to_calendar(AnyCalendar::new(black_box(k)));
                sum += i64::from(d.year().extended_year())
                    + i64::from(d.day_of_month().0)
                    + i64::from(d.days_in_year())
                    + i64::from(d.months_in_year());
                n += 1;
            }
        }
        let r = take(&format!("icu {k:?}"));
        println!("icu {k:?}: {n} dates, core_maths={} astronomy={}", r.0, r.1);
        total.add(r);
    }
    total.print("icu", sum);
}

fn month_code(s: &str) -> MonthCode {
    MonthCode::from_str(s).expect("a month code")
}

/// Mode `temporal`: the temporal_rs operations that ep9's Temporal builtins call.
fn temporal() {
    let durations: Vec<Duration> = ["P1Y", "P13M", "-P100Y", "P1000Y2M3D", "P400W", "-P1Y1M1D"]
        .iter()
        .map(|s| Duration::from_str(s).expect("a duration"))
        .collect();
    let units = [Unit::Year, Unit::Month, Unit::Week, Unit::Day];
    let mut years: Vec<i32> = (1860..=2140).collect();
    years.extend([
        -271_820, -100_000, -10_000, -3000, -1, 0, 1, 622, 1000, 1500, 1800, 3000, 5000, 10_000,
        100_000, 275_759,
    ]);
    let codes: Vec<String> = (1..=13)
        .flat_map(|m| [format!("M{m:02}"), format!("M{m:02}L")])
        .collect();
    let mut total = Total::default();
    let mut sum = 0i64;
    for id in IDS {
        let cal = Calendar::from_str(id).expect("an id temporal_rs accepts");
        let (mut ok, mut out) = (0u64, 0u64);
        for &y in &years {
            let Ok(d) = PlainDate::try_new(black_box(y), 6, 15, cal.clone()) else {
                out += 1;
                continue;
            };
            ok += 1;
            sum += i64::from(d.year())
                + i64::from(d.month())
                + i64::from(d.day())
                + i64::from(d.day_of_week())
                + i64::from(d.day_of_year())
                + i64::from(d.days_in_month())
                + i64::from(d.days_in_year())
                + i64::from(d.months_in_year())
                + i64::from(d.in_leap_year())
                + i64::from(d.week_of_year().unwrap_or(0))
                + i64::from(d.year_of_week().unwrap_or(0))
                + i64::from(d.era_year().unwrap_or(0))
                + d.month_code().as_str().len() as i64
                + d.era().map_or(0, |e| e.len()) as i64;
            let cy = d.year();
            // Fields -> date, year-month and month-day (with a year): every month code (leap
            // codes too), days 1 and 30, both overflows.
            for c in &codes {
                for overflow in [Overflow::Constrain, Overflow::Reject] {
                    let mut f = YearMonthCalendarFields::new();
                    f.year = Some(cy);
                    f.month_code = Some(month_code(c));
                    let p = PartialYearMonth {
                        calendar_fields: f,
                        calendar: cal.clone(),
                    };
                    if let Ok(ym) = PlainYearMonth::from_partial(p, Some(overflow)) {
                        sum += i64::from(ym.month()) + i64::from(ym.days_in_month());
                    }
                    for day in [1u8, 30] {
                        let p = PartialDate::new()
                            .with_calendar(cal.clone())
                            .with_year(Some(cy))
                            .with_month_code(Some(month_code(c)))
                            .with_day(Some(day));
                        if let Ok(e) = PlainDate::from_partial(p.clone(), Some(overflow)) {
                            sum += i64::from(e.day()) + i64::from(e.month());
                        }
                        if let Ok(md) = PlainMonthDay::from_partial(p, Some(overflow)) {
                            sum += i64::from(md.day());
                        }
                    }
                }
            }
            // Fields with an era (the era and era year of `d`), and `with` of one field.
            if let (Some(era), Some(ey)) = (d.era(), d.era_year()) {
                let era = TinyAsciiStr::try_from_str(era.as_str()).ok();
                let p = PartialDate::new()
                    .with_calendar(cal.clone())
                    .with_era(era)
                    .with_era_year(Some(ey))
                    .with_month_code(Some(d.month_code()))
                    .with_day(Some(d.day()));
                if let Ok(e) = PlainDate::from_partial(p, Some(Overflow::Reject)) {
                    sum += i64::from(e.year());
                }
            }
            let mut f = CalendarFields::new();
            f.day = Some(1);
            if let Ok(e) = d.with(f, Some(Overflow::Constrain)) {
                sum += i64::from(e.day_of_year());
            }
            for e in &durations {
                if let Ok(f) = d.add(e, Some(Overflow::Constrain)) {
                    for u in units {
                        let mut s = DifferenceSettings::default();
                        s.largest_unit = Some(u);
                        if let Ok(r) = d.until(&f, s) {
                            sum += r.years() as i64 + r.months() as i64 + r.days() as i64;
                        }
                        let mut s = DifferenceSettings::default();
                        s.largest_unit = Some(u);
                        s.smallest_unit = Some(u);
                        if let Ok(r) = f.since(&d, s) {
                            sum += r.months() as i64;
                        }
                    }
                }
                let _ = black_box(d.subtract(e, Some(Overflow::Reject)));
            }
            let text = format!("{d}");
            if let Ok(ym) = PlainYearMonth::from_utf8(text.as_bytes()) {
                sum += i64::from(ym.month());
            }
            if let Ok(s) = PlainDate::from_utf8(text.as_bytes()) {
                sum += i64::from(s.day());
            }
        }
        // A month-day with no year: the reference-year search, every code, days 1, 29, 30.
        for c in &codes {
            for day in [1u8, 29, 30] {
                for overflow in [Overflow::Constrain, Overflow::Reject] {
                    let p = PartialDate::new()
                        .with_calendar(cal.clone())
                        .with_month_code(Some(month_code(c)))
                        .with_day(Some(day));
                    if let Ok(md) = PlainMonthDay::from_partial(p, Some(overflow)) {
                        sum += i64::from(md.reference_year()) + i64::from(md.day());
                    }
                }
            }
        }
        let r = take(&format!("temporal {id}"));
        println!(
            "temporal {id}: {ok} dates ({out} out of range), core_maths={} astronomy={}",
            r.0, r.1
        );
        total.add(r);
    }
    total.print("temporal", sum);
}

/// Mode `control`: the opt-in astronomical years, so the counters must count.
#[cfg(feature = "control")]
fn control() {
    use icu_calendar::cal::east_asian_traditional::{China, Korea};
    let mut total = Total::default();
    let mut sum = 0i64;
    for y in [1800i32, 2200, 3000] {
        let a = China::gb_t_33661_2017(black_box(y));
        let b = Korea::adapted_gb_t_33661_2017(black_box(y));
        sum += format!("{:?}{:?}", black_box(&a), black_box(&b)).len() as i64;
        let r = take(&format!("control {y}"));
        println!(
            "control {y}: core_maths={} astronomy={} sin={}",
            r.0, r.1, r.2
        );
        total.add(r);
    }
    total.print("control", sum);
}

#[cfg(not(feature = "control"))]
fn control() {
    eprintln!("calprobe: mode control needs the feature control");
    std::process::exit(2);
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let _ = take("start");
    match mode.as_str() {
        "icu" => icu(),
        "temporal" => temporal(),
        "control" => control(),
        "all" => {
            icu();
            temporal();
        }
        m => {
            eprintln!("calprobe: unknown mode {m} (icu, temporal, control, all)");
            std::process::exit(2);
        }
    }
}
