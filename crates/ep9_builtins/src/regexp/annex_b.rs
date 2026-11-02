//! Annex B RegExp (M5-P2 REGEXP2): `RegExp.prototype.compile` (B.2.4.1) and the legacy static
//! accessors of `%RegExp%` (the "legacy RegExp features" proposal, test262 feature
//! `legacy-regexp`).
//!
//! ```text
//! re.compile(p, f)   RequireInternalSlot; [[Realm]] = this realm and
//!                    [[LegacyFeaturesEnabled]] (Cx::regexp_legacy_features), else TypeError;
//!                    p a RegExp: its source and flags (f must be undefined);
//!                    RegExpInitialize: ToString p, f; Cx::regexp_initialize; lastIndex = 0
//! RegExp.input ($_)  get / set; RegExp.lastMatch ($&), lastParen ($+), leftContext ($`),
//! rightContext ($'), $1-$9: get only. Each checks SameValue(%RegExp%, this value), then
//!                    reads the slot (Cx::regexp_legacy_static); an `empty` slot throws
//! RegExpBuiltinExec  exec.rs builtin_match: Cx::regexp_note_match after a match
//! ```
//!
//! The values live in the VM (`regexp.rs`, one set per realm, behind a root source); the
//! RegExp constructor clears `[[LegacyFeaturesEnabled]]` for a NewTarget that is not
//! `%RegExp%` (`Cx::regexp_disable_legacy_features`).

use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local, ObjectClass};

use super::exec::{is_regexp_object, set_last_index, slot};

/// RegExp.prototype.compile ( pattern, flags ) (B.2.4.1, with the legacy proposal's steps).
pub(super) fn compile<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (pattern, flags) = (args.get(0), args.get(1));
    // 1. Let O be the this value.
    // 2. Perform ? RequireInternalSlot(O, [[RegExpMatcher]]).
    let o = match this.as_object() {
        Some(o) if is_regexp_object(cx, o) => o,
        _ => {
            let shown = cx.display(this);
            let msg = format!("RegExp.prototype.compile called on incompatible receiver {shown}");
            return Err(cx.throw_type_error(&msg));
        }
    };
    // 3. Let thisRealm be the current Realm Record.
    // 4. Let oRealm be the value of O's [[Realm]] internal slot.
    // 5. If SameValue(thisRealm, oRealm) is false, throw a TypeError exception.
    // 6. If the value of R's [[LegacyFeaturesEnabled]] internal slot is false, throw a
    //    TypeError exception.
    if !cx.regexp_legacy_features(o) {
        return Err(cx.throw_type_error(
            "RegExp.prototype.compile called on a RegExp of another realm or a subclass",
        ));
    }
    // 7. If pattern is an Object and pattern has a [[RegExpMatcher]] internal slot, then
    let (p, f) = if let Some(po) = pattern.as_object()
        && is_regexp_object(cx, po)
    {
        //  a. If flags is not undefined, throw a TypeError exception.
        if !flags.is_undefined() {
            return Err(cx.throw_type_error(
                "Cannot supply flags when constructing one RegExp from another",
            ));
        }
        //  b. Let P be pattern.[[OriginalSource]].
        //  c. Let F be pattern.[[OriginalFlags]].
        let p = cx.slot(po, ObjectClass::RegExp, slot::SOURCE)?;
        let f = cx.slot(po, ObjectClass::RegExp, slot::FLAGS)?;
        (p, f)
    } else {
        // 8. Else,
        //  a. Let P be pattern.
        //  b. Let F be flags.
        (pattern, flags)
    };
    // 9. Return ? RegExpInitialize(O, P, F).
    //    RegExpInitialize steps 1-4: P and F undefined -> "", else ? ToString.
    let p = if p.is_undefined() {
        cx.new_string("")?
    } else {
        cx.to_string(p)?
    };
    let f = if f.is_undefined() {
        cx.new_string("")?
    } else {
        cx.to_string(f)?
    };
    //    Steps 5-15: the flags, the pattern and the slots.
    cx.regexp_initialize(o, p, f)?;
    //    Step 16. Perform ? Set(obj, "lastIndex", +0𝔽, true).
    set_last_index(cx, o, 0)?;
    //    Step 17. Return obj.
    Ok(o.value())
}

/// The receiver check of GetLegacyRegExpStaticProperty and SetLegacyRegExpStaticProperty
/// (step 2): SameValue(%RegExp%, thisValue), else a TypeError naming `name`.
fn check_receiver<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<()> {
    let c = cx.intrinsic(IntrinsicId::RegExp);
    if cx.same_value(c, this) {
        return Ok(());
    }
    let msg = format!("RegExp.{name} accessor called on an incompatible receiver");
    Err(cx.throw_type_error(&msg))
}

/// GetLegacyRegExpStaticProperty ( %RegExp%, this value, slot `which` )
/// (`Cx::regexp_legacy_static` numbering).
fn get_static<'s>(cx: &mut Cx<'s>, this: Local<'s>, which: u32, name: &str) -> JsResult<Local<'s>> {
    // 1. Assert C is an object that has an internal slot named internalSlotName.
    // 2. If SameValue(C, thisValue) is false, throw a TypeError exception.
    check_receiver(cx, this, name)?;
    // 3. Let value be the value of the internal slot of C named internalSlotName.
    // 4. If value is empty, throw a TypeError exception.
    // 5. Return value.
    match cx.regexp_legacy_static(which)? {
        Some(v) => Ok(v.value()),
        None => {
            let msg = format!("RegExp.{name} is not available after a RegExp subclass exec");
            Err(cx.throw_type_error(&msg))
        }
    }
}

/// get RegExp.input (= `RegExp.$_`).
pub(super) fn get_input<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? GetLegacyRegExpStaticProperty(%RegExp%, this value, [[RegExpInput]]).
    get_static(cx, this, 0, "input")
}

/// set RegExp.input (= `RegExp.$_`).
pub(super) fn set_input<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? SetLegacyRegExpStaticProperty(%RegExp%, this value, [[RegExpInput]], val).
    //    SetLegacyRegExpStaticProperty ( C, thisValue, internalSlotName, val ):
    //    1. Assert C is an object that has an internal slot named internalSlotName.
    //    2. If SameValue(C, thisValue) is false, throw a TypeError exception.
    check_receiver(cx, this, "input")?;
    //    3. Let strVal be ? ToString(val).
    let s = cx.to_string(args.get(0))?;
    //    4. Set the value of the internal slot of C named internalSlotName to strVal.
    cx.regexp_set_legacy_input(s);
    Ok(Local::undefined())
}

/// Defines the getter of one legacy static slot.
macro_rules! legacy_getter {
    ($(#[$doc:meta])* $fn:ident, $which:expr, $name:literal) => {
        $(#[$doc])*
        pub(super) fn $fn<'s>(
            cx: &mut Cx<'s>,
            this: Local<'s>,
            _args: Args<'s>,
            _new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            get_static(cx, this, $which, $name)
        }
    };
}

legacy_getter!(
    /// get RegExp.lastMatch (= `RegExp["$&"]`): `[[RegExpLastMatch]]`.
    get_last_match, 1, "lastMatch"
);
legacy_getter!(
    /// get RegExp.lastParen (= `RegExp["$+"]`): `[[RegExpLastParen]]`.
    get_last_paren, 2, "lastParen"
);
legacy_getter!(
    /// get RegExp.leftContext (= ``RegExp["$`"]``): `[[RegExpLeftContext]]`.
    get_left_context, 3, "leftContext"
);
legacy_getter!(
    /// get RegExp.rightContext (= `RegExp["$'"]`): `[[RegExpRightContext]]`.
    get_right_context, 4, "rightContext"
);
legacy_getter!(
    /// get RegExp.$1: `[[RegExpParen1]]`.
    get_paren_1, 5, "$1"
);
legacy_getter!(
    /// get RegExp.$2: `[[RegExpParen2]]`.
    get_paren_2, 6, "$2"
);
legacy_getter!(
    /// get RegExp.$3: `[[RegExpParen3]]`.
    get_paren_3, 7, "$3"
);
legacy_getter!(
    /// get RegExp.$4: `[[RegExpParen4]]`.
    get_paren_4, 8, "$4"
);
legacy_getter!(
    /// get RegExp.$5: `[[RegExpParen5]]`.
    get_paren_5, 9, "$5"
);
legacy_getter!(
    /// get RegExp.$6: `[[RegExpParen6]]`.
    get_paren_6, 10, "$6"
);
legacy_getter!(
    /// get RegExp.$7: `[[RegExpParen7]]`.
    get_paren_7, 11, "$7"
);
legacy_getter!(
    /// get RegExp.$8: `[[RegExpParen8]]`.
    get_paren_8, 12, "$8"
);
legacy_getter!(
    /// get RegExp.$9: `[[RegExpParen9]]`.
    get_paren_9, 13, "$9"
);
