//! Temporal (plan Q2; design SPEC-T-D11): the glue from the builtin ABI (H20, `ep9_vm::Cx`) to
//! `temporal_rs`. The crate has no `unsafe` and reaches the heap only through `Cx`.
//!
//! ```text
//! ep9_builtins::MODULES = [.., bigint, ep9_temporal::MODULES..]   one table per isolate
//! ep9_temporal::MODULES (blocks, append-only)
//!   19 temporal   %Temporal%, %Temporal.Now%            namespace.rs
//!   20 duration   %Temporal.Duration%                    duration.rs
//!   21 instant    %Temporal.Instant%                     instant.rs
//!   22 plain_time %Temporal.PlainTime%                   plain_time.rs
//!   23 plain_date %Temporal.PlainDate%                   plain_date.rs
//!   24 plain_date_time %Temporal.PlainDateTime%          plain_date_time.rs
//!   25 plain_year_month %Temporal.PlainYearMonth%        plain_year_month.rs
//!   26 plain_month_day %Temporal.PlainMonthDay%          plain_month_day.rs
//!   27 zoned_date_time %Temporal.ZonedDateTime%          zoned_date_time.rs
//! support: tzdb (provider, no ep9_vm import), error (temporal_rs error -> JS error),
//!          convert (ToIntegerWithTruncation ...), options (option bags), calendar (iso8601
//!          only), fields (property bags), relative_to (Duration relativeTo)
//! ```
//!
//! Rules of the glue (design §3.1): only the `*_with_provider` forms of temporal_rs; a
//! temporal_rs value lives only on the Rust stack during one builtin call (the object keeps
//! its fields as Values in internal slots); no `Local` in a Rust container; `Err` only from a
//! `cx.throw_*` call.
#![forbid(unsafe_code)]

pub(crate) mod calendar;
pub(crate) mod convert;
pub mod duration;
pub(crate) mod error;
pub(crate) mod fields;
pub mod instant;
pub mod namespace;
pub(crate) mod options;
pub mod plain_date;
pub mod plain_date_time;
pub mod plain_month_day;
pub mod plain_time;
pub mod plain_year_month;
pub(crate) mod relative_to;
pub mod tzdb;
pub mod zoned_date_time;

use ep9_vm::BuiltinModule;

/// The Temporal modules, in block order; `ep9_builtins::MODULES` lists them after its own.
pub const MODULES: &[BuiltinModule] = &[
    namespace::MODULE,
    duration::MODULE,
    instant::MODULE,
    plain_time::MODULE,
    plain_date::MODULE,
    plain_date_time::MODULE,
    plain_year_month::MODULE,
    plain_month_day::MODULE,
    zoned_date_time::MODULE,
];
