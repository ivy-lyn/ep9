//! The ICU4X object cache (INTL-I1-D7, design §4.5): ICU4X formatters live here, never in a
//! slot (plan rule 8: slots hold Values only). One small LRU per service in a `thread_local`,
//! of `Rc`s of immutable formatters made from the `'static` baked data.
//!
//! ```text
//! cache::get_or_make(Service::Collator, "de", packed_options, || Collator::try_new(..))
//!   key (service, resolved locale, packed options u64) ─ hit: move to the front, give the Rc
//!                                                      └ miss: make, put at the front, drop
//!                                                        the last one past CAPACITY
//! cache::get_or_make_default(Service::Collator, ..)     one slot per service for toLocale*
//!                                                       with locales and options undefined
//! ```
//!
//! An isolate that moves to another thread only misses the cache (correct, slower). The GC
//! needs no change: an Intl object holds its resolved locale and packed options in slots and
//! finds its formatter here at each use.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// The Intl services that cache ICU4X objects (all waves; a later wave uses its variant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Service {
    Collator,
    Locale,
    PluralRules,
    NumberFormat,
    ListFormat,
    Segmenter,
    DisplayNames,
    CaseMap,
    RelativeTimeFormat,
    DateTimeFormat,
    DurationFormat,
}

/// The number of [`Service`] variants.
const SERVICES: usize = 11;

/// Entries per service (est.; a tuning number, measured in W2 with the NumberFormat bench).
pub const CAPACITY: usize = 32;

type Entry = (String, u64, Rc<dyn Any>);

#[derive(Default)]
struct Lru {
    /// Most recent first.
    entries: Vec<Entry>,
    /// The default formatter (locales and options undefined).
    default: Option<Rc<dyn Any>>,
}

thread_local! {
    static CACHE: RefCell<Vec<Lru>> = RefCell::new((0..SERVICES).map(|_| Lru::default()).collect());
}

/// The cached object of `service` for (`locale`, `options`), made by `make` on a miss. `None`
/// if `make` gives `None` (nothing is cached then). A cached object of another type under the
/// same key is a caller bug: it is replaced.
pub fn get_or_make<T: 'static>(
    service: Service,
    locale: &str,
    options: u64,
    make: impl FnOnce() -> Option<T>,
) -> Option<Rc<T>> {
    let hit = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let lru = &mut c[service as usize];
        let i = lru
            .entries
            .iter()
            .position(|(l, o, _)| *o == options && l == locale)?;
        let e = lru.entries.remove(i);
        let rc = e.2.clone();
        lru.entries.insert(0, e);
        rc.downcast::<T>().ok()
    });
    if hit.is_some() {
        return hit;
    }
    // `make` runs with no borrow held (it may load data; it never re-enters the cache, but a
    // borrow across it would turn such a bug into a panic).
    let made = Rc::new(make()?);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let lru = &mut c[service as usize];
        lru.entries
            .retain(|(l, o, _)| !(*o == options && l == locale));
        lru.entries.insert(
            0,
            (locale.to_string(), options, made.clone() as Rc<dyn Any>),
        );
        lru.entries.truncate(CAPACITY);
    });
    Some(made)
}

/// The default object of `service` (`toLocale*` with locales and options undefined), made
/// once per thread by `make`.
pub fn get_or_make_default<T: 'static>(
    service: Service,
    make: impl FnOnce() -> Option<T>,
) -> Option<Rc<T>> {
    let hit = CACHE.with(|c| {
        c.borrow()[service as usize]
            .default
            .clone()
            .and_then(|rc| rc.downcast::<T>().ok())
    });
    if hit.is_some() {
        return hit;
    }
    let made = Rc::new(make()?);
    CACHE.with(|c| {
        c.borrow_mut()[service as usize].default = Some(made.clone() as Rc<dyn Any>);
    });
    Some(made)
}

/// The number of cached objects of `service` (tests).
pub fn len(service: Service) -> usize {
    CACHE.with(|c| c.borrow()[service as usize].entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_and_evicts() {
        let mut made = 0;
        for _ in 0..3 {
            let v = get_or_make(Service::ListFormat, "de", 7, || {
                made += 1;
                Some(42u32)
            });
            assert_eq!(v.as_deref(), Some(&42));
        }
        assert_eq!(made, 1);
        for i in 0..(CAPACITY as u64 + 5) {
            get_or_make(Service::ListFormat, "en", i, || Some(i));
        }
        assert_eq!(len(Service::ListFormat), CAPACITY);
        // "de"/7 was the oldest: it is gone, so a new object is made.
        get_or_make(Service::ListFormat, "de", 7, || {
            made += 1;
            Some(43u32)
        });
        assert_eq!(made, 2);
        assert_eq!(
            get_or_make::<u32>(Service::ListFormat, "x", 0, || None),
            None
        );
        let d = get_or_make_default(Service::ListFormat, || Some("d"));
        let e = get_or_make_default(Service::ListFormat, || Some("e"));
        assert!(Rc::ptr_eq(&d.unwrap(), &e.unwrap()));
    }
}
