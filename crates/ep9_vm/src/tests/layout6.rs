//! The OBJECT_LAYOUT 6, IC_LAYOUT 4 and VM_LAYOUT 1 tests (M6-P0 VM6, plan v11 §2.4 step 8):
//! the version pins, kind 84 registered, the FUSE cell size and the `Root::RealmFuses` index
//! against live objects, the (IC_LAYOUT_VERSION, keyed kinds-set mask) pair, the shape flags
//! probe at bit 30, and each hand-fact export against the live value. The module is declared in
//! VM6 step 0 with the other month-6 test modules; VM6 step 8 fills it.
