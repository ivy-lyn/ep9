//! Private names on the keyed property paths (C7; owner P2-CLS-VM). A private name is a
//! symbol with the private flag (`symbol::new_private`); `#x` access uses the keyed opcodes
//! with the private symbol as the key:
//!
//! ```text
//! GetKeyedProperty   o[#x]      slow::prop    -> keyed_get     PrivateGet (§7.3.31)
//! SetKeyedProperty   o[#x] = v  slow::prop    -> keyed_set     PrivateSet (§7.3.32)
//! DefineKeyedOwn*    flags      slow::prop    -> keyed_define  PrivateFieldAdd (§7.3.28),
//!                                                              PrivateMethodOrAccessorAdd (§7.3.29)
//! TestIn             #x in o    slow::compare -> keyed_has     PrivateElementFind (§7.3.27)
//! ```
//!
//! - A private element of `o` is an own entry of `o`'s property storage keyed by the private
//!   symbol, with the attributes WRITABLE only. `[[OwnPropertyKeys]]` and for-in never list a
//!   private key (property.rs), so the entries are invisible to JS. Every path here looks at
//!   the own storage only: no prototype walk, no getter, no exotic hook, no IC (C7: T0 records
//!   no keyed IC; ic.rs asserts that IC code never sees a private key).
//! - Private methods and accessors are not entries: the front end adds one brand (a private
//!   name) per class to each instance (or to the constructor for static ones), checks it with
//!   `GetKeyed`, and takes the closure from the class context. So the VM sees fields and brands
//!   only, and treats them alike except for the error text.
//! - Add on a non-extensible object throws a TypeError (PrivateFieldAdd and
//!   PrivateMethodOrAccessorAdd step 1, proposal nonextensible-applies-to-private; D4,
//!   M2-PRIVATE-EXT). Set and get ignore extensibility and frozen state.
//! - Description convention (the CLS-CG contract): `"#x"` names a private name; any other
//!   description (the class name, `""` for an anonymous class, or one that starts with `#.`)
//!   names a class brand. It only changes the error texts, which follow node v24.
//!
//! Runtime id 23 `CreatePrivateName(description)` (ABI 3) is routed here by `slow/runtime.rs`.

use ep9_value::Throw;

use crate::conv::Res;
use crate::error::throw_error;
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::is_string;
use crate::object::{attr, is_object};
use crate::props;
use crate::runtime::ErrorKind;
use crate::string;
use crate::symbol;

/// Bit 0 of the DefineKeyedOwnProperty flags: SetFunctionName from the key.
pub(super) const SET_NAME: u8 = 0x01;

/// What a private symbol names, from its description (the CLS-CG convention above).
enum Named {
    /// `#x`: a private field, method or accessor name; the text is the description.
    Element(String),
    /// A class brand; the text is the class name ("anonymous" for none).
    Brand(String),
}

fn named(k: u64) -> Named {
    let d = symbol::description(k);
    let text = if is_string(d) {
        string::to_rust_string(d)
    } else {
        String::new()
    };
    match text.strip_prefix('#') {
        Some(rest) if !rest.starts_with('.') => Named::Element(text),
        _ if text.is_empty() || text.starts_with("#.") => Named::Brand("anonymous".into()),
        _ => Named::Brand(text),
    }
}

/// The text of `k` in a "properties of undefined" or `in` message: `#x`, or `#<brand>`.
fn element_text(k: u64) -> String {
    match named(k) {
        Named::Element(t) => t,
        Named::Brand(_) => "#<brand>".into(),
    }
}

/// The TypeError of a get or set of the private key `k` on `o`, which does not have it.
fn throw_missing(cx: &mut Cx, o: u64, k: u64, write: bool) -> Throw {
    let msg = if o == UNDEFINED || o == NULL {
        let what = if o == UNDEFINED { "undefined" } else { "null" };
        let (verb, ing) = if write {
            ("set", "setting")
        } else {
            ("read", "reading")
        };
        format!(
            "Cannot {verb} properties of {what} ({ing} '{}')",
            element_text(k)
        )
    } else {
        match named(k) {
            Named::Element(t) if write => format!(
                "Cannot write private member {t} to an object whose class did not declare it"
            ),
            Named::Element(t) => format!(
                "Cannot read private member {t} from an object whose class did not declare it"
            ),
            Named::Brand(c) => format!("Receiver must be an instance of class {c}"),
        }
    };
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// `o.#x` (PrivateGet, §7.3.31, for a field; the brand check of a private method or
/// accessor): the value of `o`'s own entry `k`. TypeError if `o` has none (a primitive never
/// has one: ToObject would give a new wrapper). Not a GC point unless it throws.
pub(crate) fn keyed_get(cx: &mut Cx, o: u64, k: u64) -> Res<u64> {
    debug_assert!(symbol::is_private_key(k));
    match is_object(o).then(|| props::own_get(o, k)).flatten() {
        Some((v, _)) => Ok(v),
        None => Err(throw_missing(cx, o, k, false)),
    }
}

/// `o.#x = v` (PrivateSet, §7.3.32, for a field): overwrites `o`'s own entry `k`. TypeError if
/// `o` has none. The front end never sets a brand (a write to a private method or accessor
/// is its own throw or setter call). `_strict` does not matter: a failed PrivateSet always
/// throws. Not a GC point unless it throws.
pub(crate) fn keyed_set(cx: &mut Cx, o: u64, k: u64, v: u64, _strict: bool) -> Res<()> {
    debug_assert!(symbol::is_private_key(k));
    if is_object(o) && props::own_replace(cx.rt(), o, k, v) {
        return Ok(());
    }
    Err(throw_missing(cx, o, k, true))
}

/// The own define of a private key (`DefineKeyedOwnProperty` with flag bit 1, or any other own
/// define that meets a private key): PrivateFieldAdd (§7.3.28) for a field,
/// PrivateMethodOrAccessorAdd (§7.3.29) for a brand. TypeError if `o` is not an object, is
/// not extensible (step 1), or already has `k` (step 3). Bit 0 ([`SET_NAME`]): an anonymous
/// function value is named from the key's description first (SetFunctionName §10.2.9 step
/// 3: a private name gives its description). GC point.
pub(crate) fn keyed_define(cx: &mut Cx, o: u64, k: u64, v: u64, flags: u8) -> Res<()> {
    debug_assert!(symbol::is_private_key(k));
    if !is_object(o) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot define a property on a non-object",
        ));
    }
    let present = props::own_get(o, k).is_some();
    if present || !crate::property::is_extensible(o) {
        let msg = match (named(k), present) {
            (Named::Element(t), true) => format!("Cannot initialize {t} twice on the same object"),
            (Named::Brand(c), true) => {
                format!("Cannot initialize private methods of class {c} twice on the same object")
            }
            (Named::Element(t), false) => {
                format!("Cannot define private field {t} on a non-extensible object")
            }
            (Named::Brand(c), false) => {
                format!("Cannot define private methods of class {c} on a non-extensible object")
            }
        };
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let description = symbol::description(k);
    if flags & SET_NAME != 0 && is_string(description) {
        // GC point: `o`, `k` and `v` live in Rust locals (CSS).
        super::class_fields::name_anonymous_function(cx, v, description)?;
    }
    // GC point (a new shape or table).
    props::own_add(cx.rt(), o, k, v, attr::WRITABLE);
    Ok(())
}

/// `#x in o` (§13.10.1 step 5-6: PrivateElementFind, §7.3.27): true if `o` has the own entry
/// `k`. TypeError if `o` is not an object. Not a GC point unless it throws.
pub(crate) fn keyed_has(cx: &mut Cx, o: u64, k: u64) -> Res<bool> {
    debug_assert!(symbol::is_private_key(k));
    if !is_object(o) {
        let msg = format!(
            "Cannot use 'in' operator to search for '{}' in {}",
            element_text(k),
            crate::conv::describe(o).trim_matches('"')
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    Ok(props::own_get(o, k).is_some())
}

/// `CreatePrivateName(description)` (runtime id 23): a new private name symbol
/// (§6.2.12 NewPrivateName, `symbol::new_private`) for `#x` or a class brand. `description`
/// is a string (`"#x"`, or the class name for a brand) or undefined. GC point.
pub(crate) fn create_private_name(cx: &mut Cx, description: u64) -> Res<u64> {
    if description != UNDEFINED && !is_string(description) {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: CreatePrivateName description is not a string",
        ));
    }
    // A string description is flattened so error texts and SetFunctionName read it directly.
    let d = if is_string(description) {
        string::flatten(cx, description)
    } else {
        description
    };
    Ok(symbol::new_private(cx.rt(), d))
}
