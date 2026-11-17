//! The JSON.stringify text on the string payload (H20 part 2; OXC-PERF wave 3 W3-JSON-QUOTE).
//!
//! ```text
//! Cx::json_quote_into   QuoteJSONString (§25.5.2.3) of a flat string, appended   quote, keys
//! Cx::json_push_flat    the units of a flat string, appended as they are         numbers, rawJSON
//! Cx::json_text_finish  the string of the text                                   the result
//! ```
//!
//! The text is a [`JsonText`]: one-byte until the first code unit above 0xFF, then two-byte
//! (`string/json_quote.rs`). The first two methods take `&self`: they are not GC points; they
//! read the payload in place and run no user code. The quote uses the SIMD body of the CPU
//! (SSE2 or AVX2 on x86-64, NEON on aarch64, simd128 on wasm32, SWAR elsewhere). That body is
//! a human fast path (FASTDIFF row `json-quote`): `Cx::set_human_fast_paths(false)` makes it
//! the per-unit reference loop.

use ep9_value::JsResult;

use super::{Cx, FlatStr, JsString, Local};
use crate::string::json_quote::Kernel;
use crate::string::{self, JsonText, MAX_STRING_LENGTH};

impl<'s> Cx<'s> {
    /// Appends QuoteJSONString(`s`) (§25.5.2.3) to `out`: `"`, the code units of `s` with the
    /// JSON escapes (Table 77; `\u00xx` for the other units below 0x20; `\udxxx` for a lone
    /// surrogate; lowercase hex), `"`. Not a GC point (flatten first: [`Cx::flatten`]).
    ///
    /// ```
    /// # use ep9_vm::{Isolate, IsolateConfig, JsonText};
    /// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
    /// let t = iso
    ///     .with_cx(|cx| {
    ///         let s = cx.new_string("a\"b\n")?;
    ///         let flat = cx.flatten(s)?;
    ///         let mut out = JsonText::new();
    ///         cx.json_quote_into(flat, &mut out);
    ///         let r = cx.json_text_finish(out)?;
    ///         cx.to_rust_string(r)
    ///     })
    ///     .unwrap();
    /// assert_eq!(t, r#""a\"b\n""#);
    /// ```
    pub fn json_quote_into(&self, s: Local<'s, FlatStr>, out: &mut JsonText) {
        let k = if crate::call::human_fast_paths(self.rt()) {
            Kernel::detect()
        } else {
            Kernel::scalar()
        };
        if k != Kernel::scalar() {
            crate::call::fast_hit!(6);
        }
        string::json_quote(k, s.bits(), out);
    }

    /// Appends the code units of `s` to `out` as they are (a unit above 0xFF makes the text
    /// two-byte). Not a GC point.
    pub fn json_push_flat(&self, s: Local<'s, FlatStr>, out: &mut JsonText) {
        string::json_push(s.bits(), out);
    }

    /// The string of the text `text`. RangeError past the string length limit (2^29 - 24).
    /// A one-byte text is one copy, with no scan. GC point.
    pub fn json_text_finish(&mut self, text: JsonText) -> JsResult<Local<'s, JsString>> {
        if text.len() > MAX_STRING_LENGTH {
            return Err(self.throw_range_error("Invalid string length"));
        }
        Ok(Self::local(string::json_text_string(self.rt(), text)))
    }
}
