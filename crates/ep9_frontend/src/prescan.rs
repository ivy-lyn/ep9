//! Depth guard layer 0: a token pre-scan that bounds oxc's parser recursion before oxc runs.
//!
//! oxc 0.155 has no depth limit: deep input overflows the stack and aborts the process
//! (01-front-end §6). Until the oxc counter lands (layer 1, M1), this scan estimates the parser's
//! recursion depth. The front end rejects an estimate above [`crate::depth::PRESCAN_CAP`] and runs
//! the parse on a worker whose stack fits that cap (see `stack.rs`).
//!
//! ```text
//! estimate at a token = sum over open frames of (1 + chain ops pending in that frame)
//!
//!   frame       = ( [ { ${ and a regex group      closed by the matching closer
//!   chain op    = a token that makes the parser recurse without a bracket:
//!                 expression level: prefix ! ~ + - ++ -- typeof void delete await yield new,
//!                                   = and op=, ? (not the matching :), =>, **, extends
//!                 statement level:  if, for, while (not a do-while tail), do, with, label:
//! ```
//!
//! A pending chain op ends only where every production it opened has ended. Each reset rule below
//! is sound for valid code:
//!
//! | Event | Resets in the current frame | Why |
//! |---|---|---|
//! | `,` | expression ops | ends every AssignmentExpression of this frame |
//! | `;` | expression ops; statement ops unless the next token is `else`/`while`/`catch`/`finally` | an `if` waits for `else`, a `do` for `while` |
//! | newline + ASI (expression end, then a token that cannot continue it) | same as `;` | ECMA-262 §12.10 |
//! | `}` of a statement-level block, function or class declaration | same as `;`, in the parent frame | the statement ended |
//!
//! Limits: this is a lexer, not a parser. It decides regex vs division and the role of each
//! `{` from the tokens before it, as the grammar does for valid code. Input that fools these rules
//! can still misplace the estimate; layer 1 removes that risk at M1, and the M1 fuzz gate covers it.

/// Estimate units per regex group or class level. oxc's regex parser uses about 4 times the
/// stack of a parser level in a debug build (16.9 KB vs 4.2 KB per level on aarch64; 0.7 KB vs
/// 2.3 KB in release), so one regex level counts as 4 units.
pub(crate) const REGEX_WEIGHT: u32 = 4;

/// The result of a pre-scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Estimate {
    /// The highest estimated recursion depth, in units (frames + chain ops).
    pub(crate) depth: u32,
    /// Byte offset of the token where `depth` was first reached.
    pub(crate) at: u32,
}

/// Estimate the parser recursion depth of `src` (UTF-8). `module` selects the Module goal
/// (no HTML-like comments).
pub(crate) fn estimate(src: &[u8], module: bool) -> Estimate {
    let mut scanner = Scanner::new(src, module);
    scanner.run();
    Estimate {
        depth: scanner.max,
        at: scanner.max_at,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kw {
    None,
    If,
    Else,
    For,
    While,
    Do,
    With,
    Switch,
    Case,
    Default,
    Catch,
    Try,
    Finally,
    Function,
    Class,
    Extends,
    Return,
    Throw,
    New,
    Delete,
    Void,
    Typeof,
    In,
    Instanceof,
    Of,
    Yield,
    Await,
    This,
    Super,
    Null,
    True,
    False,
    Async,
    Static,
    Export,
    Import,
    As,
    Satisfies,
    /// Other reserved words that never end an expression: var, let, const, break, continue,
    /// debugger. `let` is listed here even though it can be an identifier: a `/` after it is
    /// rare either way, and a statement start is the safe reading.
    Other,
}

fn keyword(word: &[u8]) -> Kw {
    match word {
        b"if" => Kw::If,
        b"else" => Kw::Else,
        b"for" => Kw::For,
        b"while" => Kw::While,
        b"do" => Kw::Do,
        b"with" => Kw::With,
        b"switch" => Kw::Switch,
        b"case" => Kw::Case,
        b"default" => Kw::Default,
        b"catch" => Kw::Catch,
        b"try" => Kw::Try,
        b"finally" => Kw::Finally,
        b"function" => Kw::Function,
        b"class" => Kw::Class,
        b"extends" => Kw::Extends,
        b"return" => Kw::Return,
        b"throw" => Kw::Throw,
        b"new" => Kw::New,
        b"delete" => Kw::Delete,
        b"void" => Kw::Void,
        b"typeof" => Kw::Typeof,
        b"in" => Kw::In,
        b"instanceof" => Kw::Instanceof,
        b"of" => Kw::Of,
        b"yield" => Kw::Yield,
        b"await" => Kw::Await,
        b"this" => Kw::This,
        b"super" => Kw::Super,
        b"null" => Kw::Null,
        b"true" => Kw::True,
        b"false" => Kw::False,
        b"async" => Kw::Async,
        b"static" => Kw::Static,
        b"export" => Kw::Export,
        b"import" => Kw::Import,
        b"as" => Kw::As,
        b"satisfies" => Kw::Satisfies,
        b"var" | b"let" | b"const" | b"break" | b"continue" | b"debugger" => Kw::Other,
        _ => Kw::None,
    }
}

/// What a `(` frame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParenKind {
    /// `if (`, `while (`, `for (`, `with (`, `switch (`, `catch (`: a statement head.
    Head,
    /// The `while (` that ends a `do` statement.
    DoTail,
    /// Formal parameters; the next `{` after the `)` is a body of this kind.
    Params(Body),
    /// Anything else: call arguments, grouping, arrow parameters.
    Expr,
    /// `async (`: call arguments, or the parameters of an async arrow.
    AsyncArgs,
}

/// The function a frame belongs to, for `yield` and `await`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FnCtx {
    /// Inside a generator body: `yield` is an operator.
    generator: bool,
    /// Inside an async body or module code: `await` is an operator.
    async_: bool,
}

/// The kind of a function body that follows a parameter list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Body {
    /// A function declaration: its `}` ends a statement.
    FunctionDecl,
    /// A function expression: its `}` ends an operand.
    FunctionExpr,
    /// A method, getter, setter or constructor.
    Method,
}

/// What a frame is. Statement-list roles hold statements; the others hold expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Top,
    /// Block statement, switch body, static block, try/catch/finally block, statement bodies.
    Block,
    Body(Body),
    ArrowBody,
    ClassBody {
        decl: bool,
    },
    Object,
    TemplateSubst,
    Paren(ParenKind),
    Bracket,
}

impl Role {
    fn holds_statements(self) -> bool {
        matches!(
            self,
            Role::Top | Role::Block | Role::Body(_) | Role::ArrowBody
        )
    }

    /// The `}` of this role ends a statement in the parent frame.
    fn closes_statement(self) -> bool {
        matches!(
            self,
            Role::Block | Role::Body(Body::FunctionDecl) | Role::ClassBody { decl: true }
        )
    }

    /// After the `}` of this role, an operand has ended (a `/` divides).
    fn closes_operand(self) -> bool {
        matches!(
            self,
            Role::Object
                | Role::Body(Body::FunctionExpr | Body::Method)
                | Role::ClassBody { decl: false }
                | Role::ArrowBody
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    role: Role,
    /// Units of every enclosing frame, including this frame's own 1.
    base: u32,
    /// Pending expression-level chain ops.
    expr: u32,
    /// Pending statement-level chain ops.
    stmt: u32,
    /// Unmatched `?` of conditionals.
    questions: u32,
    /// A `case`/`default` waits for its `:`.
    in_case: bool,
    /// Object literal: a `:` came after the last `,` (value position).
    value_position: bool,
    /// Class body: inside a field initializer (after `=`).
    field_init: bool,
    /// `function` was seen in this frame; the next `(` is its parameter list.
    pending_params: Option<(Body, FnCtx)>,
    /// A parameter list closed in this frame; the next `{` is its body.
    pending_body: Option<(Body, FnCtx)>,
    /// The function this frame belongs to.
    ctx: FnCtx,
    /// The concise body of an async arrow is open in this frame (until the expression ends).
    concise_async: bool,
    /// Class body or object literal: `async` / `*` came before the member name.
    member_async: bool,
    member_generator: bool,
    /// `class` was seen in this frame; the next `{` is its body.
    pending_class: Option<bool>,
}

impl Frame {
    fn new(role: Role, base: u32, ctx: FnCtx) -> Self {
        Self {
            ctx,
            concise_async: false,
            member_async: false,
            member_generator: false,
            role,
            base,
            expr: 0,
            stmt: 0,
            questions: 0,
            in_case: false,
            value_position: false,
            field_init: false,
            pending_params: None,
            pending_body: None,
            pending_class: None,
        }
    }

    fn depth(&self) -> u32 {
        self.base
            .saturating_add(self.expr)
            .saturating_add(self.stmt)
    }
}

/// The previous significant token, as far as the rules need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Start,
    /// Identifier or keyword. `after_dot`: a property name after `.` or `?.`.
    Word {
        kw: Kw,
        after_dot: bool,
    },
    /// Number, string, complete template, regex, private name.
    Literal,
    /// `)` of a paren frame of this kind.
    CloseParen(ParenKind),
    CloseBracket,
    /// `}` of a frame with this role.
    CloseBrace(Role),
    /// Postfix `++`/`--`.
    Postfix,
    /// `;`
    Semi,
    /// `:` that ends a label or a `case`/`default` clause.
    StatementColon,
    /// `=>`
    Arrow,
    /// `{` of a frame with this role (also `${`).
    Open(Role),
    /// `.` or `?.`
    Dot,
    /// `*` (generator marker or multiplication)
    Star,
    /// Any other punctuator.
    Punct,
}

impl Tok {
    /// True if an expression (operand) ends with this token: a following `/` divides.
    fn ends_operand(self) -> bool {
        match self {
            Tok::Word { kw, after_dot } => {
                after_dot
                    || matches!(
                        kw,
                        Kw::None
                            | Kw::This
                            | Kw::Super
                            | Kw::Null
                            | Kw::True
                            | Kw::False
                            | Kw::Async
                            | Kw::Static
                            | Kw::As
                            | Kw::Satisfies
                    )
            }
            Tok::Literal | Tok::CloseBracket | Tok::Postfix => true,
            Tok::CloseParen(kind) => matches!(kind, ParenKind::Expr | ParenKind::AsyncArgs),
            Tok::CloseBrace(role) => role.closes_operand(),
            _ => false,
        }
    }
}

struct Scanner<'s> {
    src: &'s [u8],
    i: usize,
    module: bool,
    frames: Vec<Frame>,
    prev: Tok,
    /// The token before `prev` (for `for await (`).
    prev2: Tok,
    /// A line terminator came after `prev`.
    newline: bool,
    /// No token yet on the current line (for `-->` comments).
    line_start: bool,
    /// `prev` started a statement.
    prev_at_statement_start: bool,
    /// A `;` or statement-level `}` waits for the next token to reset statement ops.
    reset_pending: bool,
    /// The previous token is a `while` that ends a `do`: its `(` is a [`ParenKind::DoTail`].
    do_tail: bool,
    /// The function context of the frame that `close` popped last.
    frames_ctx_of_closed: FnCtx,
    max: u32,
    max_at: u32,
    /// Test only: the start offset and kind of every literal token, to compare with oxc.
    #[cfg(test)]
    log: Vec<(LitKind, u32)>,
}

/// Literal token kinds in the test log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LitKind {
    Regex,
    String,
    Template,
}

/// Test only: run the scanner and return its literal tokens (start offsets by kind).
#[cfg(test)]
pub(crate) fn literal_tokens(src: &[u8], module: bool) -> Vec<(LitKind, u32)> {
    let mut scanner = Scanner::new(src, module);
    scanner.run();
    scanner.log
}

impl<'s> Scanner<'s> {
    #[cfg(test)]
    fn log(&mut self, kind: LitKind, at: usize) {
        self.log.push((kind, u32::try_from(at).unwrap_or(u32::MAX)));
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn log(&mut self, _kind: LitKind, _at: usize) {}

    fn new(src: &'s [u8], module: bool) -> Self {
        Self {
            src,
            i: 0,
            module,
            frames: vec![Frame::new(
                Role::Top,
                1,
                FnCtx {
                    generator: false,
                    async_: module,
                },
            )],
            prev: Tok::Start,
            prev2: Tok::Start,
            newline: false,
            line_start: true,
            prev_at_statement_start: false,
            reset_pending: false,
            do_tail: false,
            frames_ctx_of_closed: FnCtx::default(),
            max: 1,
            max_at: 0,
            #[cfg(test)]
            log: Vec::new(),
        }
    }

    fn top(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("the Top frame is never popped")
    }

    fn top_ref(&self) -> &Frame {
        self.frames.last().expect("the Top frame is never popped")
    }

    fn note(&mut self, depth: u32, at: usize) {
        if depth > self.max {
            self.max = depth;
            self.max_at = u32::try_from(at).unwrap_or(u32::MAX);
        }
    }

    fn bump_expr(&mut self, at: usize) {
        let top = self.top();
        top.expr = top.expr.saturating_add(1);
        let depth = top.depth();
        self.note(depth, at);
    }

    fn bump_stmt(&mut self, at: usize) {
        let top = self.top();
        top.stmt = top.stmt.saturating_add(1);
        let depth = top.depth();
        self.note(depth, at);
    }

    fn push(&mut self, role: Role, at: usize) {
        let ctx = self.top_ref().ctx;
        self.push_with(role, ctx, at);
    }

    fn push_with(&mut self, role: Role, ctx: FnCtx, at: usize) {
        let base = self.top_ref().depth().saturating_add(1);
        self.frames.push(Frame::new(role, base, ctx));
        self.note(base, at);
    }

    /// Every expression of the top frame has ended.
    fn end_expressions(&mut self) {
        let top = self.top();
        top.expr = 0;
        top.questions = 0;
        top.in_case = false;
        top.value_position = false;
        top.field_init = false;
        top.pending_body = None;
        top.pending_params = None;
        top.pending_class = None;
        top.concise_async = false;
        top.member_async = false;
        top.member_generator = false;
    }

    /// Newline ASI before a token that cannot continue an expression (ECMA-262 §12.10).
    /// `continues_statement`: the token is `else`/`while`/`catch`/`finally`.
    fn asi(&mut self, continues_statement: bool) {
        if self.newline && self.prev.ends_operand() {
            self.end_expressions();
            if !continues_statement {
                self.top().stmt = 0;
            }
        }
    }

    /// A pending `;` or statement-level `}` reset: drop statement ops unless this token
    /// continues the statement.
    fn resolve_reset(&mut self, continues_statement: bool) {
        if self.reset_pending {
            self.reset_pending = false;
            if !continues_statement {
                self.top().stmt = 0;
            }
        }
    }

    /// The next token would start a statement.
    fn at_statement_start(&self) -> bool {
        if !self.top_ref().role.holds_statements() {
            return false;
        }
        match self.prev {
            Tok::Start | Tok::Semi | Tok::StatementColon => true,
            Tok::Open(role) => role.holds_statements(),
            Tok::CloseBrace(role) => role.closes_statement(),
            Tok::CloseParen(ParenKind::Head | ParenKind::DoTail) => true,
            Tok::Word {
                kw: Kw::Else | Kw::Do | Kw::Try | Kw::Finally | Kw::Catch,
                after_dot: false,
            } => true,
            prev => self.newline && prev.ends_operand(),
        }
    }

    /// `function`/`class` here is a declaration (its `}` ends a statement).
    fn declaration_position(&self, statement_start: bool) -> bool {
        let after_export = |tok: Tok| {
            matches!(
                tok,
                Tok::Word {
                    kw: Kw::Export | Kw::Default,
                    after_dot: false
                }
            )
        };
        statement_start || after_export(self.prev)
    }

    fn run(&mut self) {
        if self.src.starts_with(b"#!") {
            self.skip_line();
        }
        loop {
            self.skip_trivia();
            if self.i >= self.src.len() {
                break;
            }
            let start = self.i;
            let byte = self.src[start];
            let statement_start = self.at_statement_start();
            let tok = match byte {
                b'(' => self.open_paren(start),
                b')' => self.close(start, CloserKind::Paren),
                b'[' => {
                    self.i += 1;
                    self.resolve_reset(false);
                    self.push(Role::Bracket, start);
                    Tok::Punct
                }
                b']' => self.close(start, CloserKind::Bracket),
                b'{' => self.open_brace(start, statement_start),
                b'}' => self.close(start, CloserKind::Brace),
                b'`' => {
                    // A template after a newline continues the expression (tagged template).
                    self.log(LitKind::Template, start);
                    self.i += 1;
                    self.resolve_reset(false);
                    self.template_chars()
                }
                b'\'' | b'"' => {
                    self.asi(false);
                    self.resolve_reset(false);
                    self.log(LitKind::String, start);
                    self.string(byte);
                    Tok::Literal
                }
                b'/' if !self.prev.ends_operand() => {
                    self.resolve_reset(false);
                    self.log(LitKind::Regex, start);
                    self.regex(start);
                    Tok::Literal
                }
                b'0'..=b'9' => {
                    self.asi(false);
                    self.resolve_reset(false);
                    self.number();
                    Tok::Literal
                }
                b'.' if self.src.get(start + 1).is_some_and(u8::is_ascii_digit) => {
                    self.asi(false);
                    self.resolve_reset(false);
                    self.number();
                    Tok::Literal
                }
                b'#' => {
                    self.asi(false);
                    self.resolve_reset(false);
                    self.identifier_tail_from(start + 1);
                    Tok::Literal
                }
                _ if is_ident_start(byte) => self.word(start, statement_start),
                _ => self.punctuator(start),
            };
            self.prev2 = self.prev;
            self.prev = tok;
            self.prev_at_statement_start = statement_start;
            self.newline = false;
            self.line_start = false;
        }
    }

    fn open_paren(&mut self, start: usize) -> Tok {
        self.i += 1;
        self.resolve_reset(false);
        let top = *self.top_ref();
        let head_keyword = matches!(
            self.prev,
            Tok::Word {
                kw: Kw::If | Kw::While | Kw::For | Kw::With | Kw::Switch | Kw::Catch,
                after_dot: false,
            }
        );
        let for_await = matches!(
            (self.prev, self.prev2),
            (
                Tok::Word {
                    kw: Kw::Await,
                    after_dot: false
                },
                Tok::Word {
                    kw: Kw::For,
                    after_dot: false
                }
            )
        );
        let after_async = self.prev
            == (Tok::Word {
                kw: Kw::Async,
                after_dot: false,
            })
            && !self.newline;
        let mut params_ctx = None;
        let kind = if let Some((body, ctx)) = top.pending_params {
            params_ctx = Some(ctx);
            ParenKind::Params(body)
        } else if self.do_tail {
            ParenKind::DoTail
        } else if head_keyword || for_await {
            ParenKind::Head
        } else if (matches!(top.role, Role::ClassBody { .. }) && !top.field_init)
            || (top.role == Role::Object && !top.value_position)
        {
            // A method. `async(` is a method named async; `async m(` is an async method.
            params_ctx = Some(FnCtx {
                generator: top.member_generator,
                async_: top.member_async && !after_async,
            });
            ParenKind::Params(Body::Method)
        } else if after_async {
            ParenKind::AsyncArgs
        } else {
            ParenKind::Expr
        };
        let top = self.top();
        top.pending_params = None;
        top.member_async = false;
        top.member_generator = false;
        self.do_tail = false;
        // Parameters belong to their function (defaults may hold `await`/`yield` of the body's kind).
        let ctx = params_ctx.unwrap_or(self.top_ref().ctx);
        self.push_with(Role::Paren(kind), ctx, start);
        Tok::Punct
    }

    fn open_brace(&mut self, start: usize, statement_start: bool) -> Tok {
        self.i += 1;
        self.resolve_reset(false);
        let top = *self.top_ref();
        let static_block = matches!(top.role, Role::ClassBody { .. })
            && self.prev
                == (Tok::Word {
                    kw: Kw::Static,
                    after_dot: false,
                });
        let restricted = self.newline
            && matches!(
                self.prev,
                Tok::Word {
                    kw: Kw::Return | Kw::Yield,
                    after_dot: false
                }
            );
        let mut ctx = top.ctx;
        let role = if let Some(decl) = top.pending_class {
            // Class bodies are strict: `yield`/`await` keep the outer meaning only in computed keys
            // and initializers, which this approximation ignores.
            Role::ClassBody { decl }
        } else if let Some((body, body_ctx)) = top.pending_body {
            ctx = body_ctx;
            Role::Body(body)
        } else if let Tok::Arrow = self.prev {
            ctx = FnCtx {
                generator: false,
                async_: top.concise_async,
            };
            Role::ArrowBody
        } else if static_block || statement_start || restricted {
            Role::Block
        } else {
            Role::Object
        };
        if role == Role::Block {
            // `x = y \n {`: ASI ended the expression before the block.
            self.asi(false);
        }
        let top = self.top();
        top.pending_class = None;
        top.pending_body = None;
        if role == Role::ArrowBody {
            top.concise_async = false;
        }
        self.push_with(role, ctx, start);
        Tok::Open(role)
    }

    fn close(&mut self, start: usize, closer: CloserKind) -> Tok {
        self.i += 1;
        // A pending reset belonged to the frame that ends here.
        self.reset_pending = false;
        let role = self.top_ref().role;
        if closer == CloserKind::Brace && role == Role::TemplateSubst {
            self.frames.pop();
            return self.template_chars();
        }
        let matches = match closer {
            CloserKind::Paren => matches!(role, Role::Paren(_)),
            CloserKind::Bracket => role == Role::Bracket,
            CloserKind::Brace => !matches!(role, Role::Top | Role::Paren(_) | Role::Bracket),
        };
        if !matches {
            // A mismatched closer (invalid input): keep the frame, so the estimate stays high.
            let _ = start;
            return Tok::Punct;
        }
        let closed = self.frames.pop().expect("not the Top frame");
        self.frames_ctx_of_closed = closed.ctx;
        match role {
            Role::Paren(kind) => {
                if let ParenKind::Params(body) = kind {
                    let ctx = self.frames_ctx_of_closed;
                    self.top().pending_body = Some((body, ctx));
                }
                Tok::CloseParen(kind)
            }
            Role::Bracket => Tok::CloseBracket,
            role => {
                if role.closes_statement() {
                    self.end_expressions();
                    self.reset_pending = true;
                }
                Tok::CloseBrace(role)
            }
        }
    }

    fn word(&mut self, start: usize, statement_start: bool) -> Tok {
        let escaped = self.identifier_tail_from(start);
        let after_dot = self.prev == Tok::Dot;
        let top = *self.top_ref();
        // A property key position: keywords are plain names there (`{ if: 1 }`, `m() {}`).
        let key_position = (top.role == Role::Object && !top.value_position)
            || (matches!(top.role, Role::ClassBody { .. }) && !top.field_init);
        let word = &self.src[start..self.i];
        let kw = if escaped || after_dot {
            Kw::None
        } else if key_position {
            match word {
                b"static" => Kw::Static,
                b"async" => Kw::Async,
                _ => Kw::None,
            }
        } else {
            match keyword(word) {
                // `of` is a keyword only in a `for (` head.
                Kw::Of if top.role != Role::Paren(ParenKind::Head) => Kw::None,
                // `yield` is an operator only in generator bodies (an identifier elsewhere in
                // sloppy code, and reserved in strict code).
                Kw::Yield if !top.ctx.generator => Kw::None,
                // `await` is an operator in async bodies and module code.
                Kw::Await if !(top.ctx.async_ || top.concise_async) => Kw::None,
                kw => kw,
            }
        };
        if key_position && kw == Kw::Async {
            self.top().member_async = true;
        }
        let continues_statement = matches!(kw, Kw::Else | Kw::While | Kw::Catch | Kw::Finally);
        // An identifier-name after a newline cannot continue an expression, except the binary
        // keywords.
        if !matches!(
            kw,
            Kw::In | Kw::Instanceof | Kw::Of | Kw::As | Kw::Satisfies
        ) {
            self.asi(continues_statement);
        }
        // A `while` right after a statement end (`;`, `}`, ASI, or the `)` of an inner do-while
        // tail) closes a `do`: no new level.
        let do_tail = kw == Kw::While
            && (self.reset_pending
                || (self.newline && self.prev.ends_operand())
                || self.prev == Tok::CloseParen(ParenKind::DoTail));
        self.do_tail = do_tail;
        self.resolve_reset(continues_statement);
        match kw {
            Kw::Typeof | Kw::Void | Kw::Delete | Kw::Await | Kw::Yield | Kw::New | Kw::Extends => {
                self.bump_expr(start);
            }
            // The class body is a frame and `extends` a chain op; `class` adds no level itself.
            Kw::Class => {
                let decl = self.declaration_position(statement_start);
                self.top().pending_class = Some(decl);
            }
            // `else` continues the `if` production: no new level. `else if` nests through `if`.
            Kw::If | Kw::For | Kw::Do | Kw::With => self.bump_stmt(start),
            Kw::While if !do_tail => self.bump_stmt(start),
            Kw::Case | Kw::Default => self.top().in_case = true,
            Kw::Function => {
                let after_async = self.prev
                    == (Tok::Word {
                        kw: Kw::Async,
                        after_dot: false,
                    });
                let decl = if after_async {
                    self.prev_at_statement_start
                        || matches!(
                            self.prev2,
                            Tok::Word {
                                kw: Kw::Export | Kw::Default,
                                after_dot: false
                            }
                        )
                } else {
                    self.declaration_position(statement_start)
                };
                let body = if decl {
                    Body::FunctionDecl
                } else {
                    Body::FunctionExpr
                };
                let ctx = FnCtx {
                    generator: false,
                    async_: after_async && !self.newline,
                };
                self.top().pending_params = Some((body, ctx));
            }
            _ => {}
        }
        Tok::Word { kw, after_dot }
    }

    fn punctuator(&mut self, start: usize) -> Tok {
        let (len, op) = punct(&self.src[start..]);
        self.i += len;
        // `!`, `~`, `++`, `--` after a newline cannot continue an expression (`++`/`--` are a
        // restricted production).
        if matches!(op, Op::Bang | Op::Tilde | Op::IncDec) {
            self.asi(false);
        }
        self.resolve_reset(false);
        match op {
            Op::Semi => {
                self.end_expressions();
                self.reset_pending = true;
                return Tok::Semi;
            }
            Op::Comma => {
                let top = self.top();
                top.expr = 0;
                top.questions = 0;
                top.value_position = false;
                top.pending_body = None;
                top.pending_params = None;
                top.pending_class = None;
                top.concise_async = false;
                top.member_async = false;
                top.member_generator = false;
            }
            Op::Question => {
                self.top().questions += 1;
                self.bump_expr(start);
            }
            Op::Colon => {
                let prev = self.prev;
                let top = self.top();
                if top.questions > 0 {
                    top.questions -= 1;
                } else if top.in_case {
                    top.in_case = false;
                    return Tok::StatementColon;
                } else if top.role == Role::Object {
                    top.value_position = true;
                } else if top.role.holds_statements()
                    && matches!(
                        prev,
                        Tok::Word {
                            after_dot: false,
                            ..
                        }
                    )
                {
                    // A label.
                    self.bump_stmt(start);
                    return Tok::StatementColon;
                }
            }
            Op::Assign => {
                let top = self.top();
                match top.role {
                    Role::ClassBody { .. } => top.field_init = true,
                    // `{ x = init }`: a shorthand default in a pattern; `init` is a value.
                    Role::Object => top.value_position = true,
                    _ => {}
                }
                self.bump_expr(start);
            }
            Op::Arrow => {
                let async_arrow = self.prev == Tok::CloseParen(ParenKind::AsyncArgs)
                    || (matches!(self.prev, Tok::Word { kw: Kw::None, .. })
                        && self.prev2
                            == (Tok::Word {
                                kw: Kw::Async,
                                after_dot: false,
                            }));
                if async_arrow {
                    // A concise body (no `{`) stays in this frame until the expression ends.
                    self.top().concise_async = true;
                }
                self.bump_expr(start);
                return Tok::Arrow;
            }
            Op::StarStar | Op::Bang | Op::Tilde => self.bump_expr(start),
            Op::PlusMinus => {
                if !self.prev.ends_operand() {
                    self.bump_expr(start);
                }
            }
            Op::IncDec => {
                if self.prev.ends_operand() && !self.newline {
                    return Tok::Postfix;
                }
                self.bump_expr(start);
            }
            Op::Spread => {
                let top = self.top();
                if top.role == Role::Object {
                    top.value_position = true;
                }
            }
            Op::Dot => return Tok::Dot,
            Op::Star => {
                let prev = self.prev;
                let top = self.top();
                if prev
                    == (Tok::Word {
                        kw: Kw::Function,
                        after_dot: false,
                    })
                {
                    if let Some((_, ctx)) = top.pending_params.as_mut() {
                        ctx.generator = true;
                    }
                } else if (matches!(top.role, Role::ClassBody { .. }) && !top.field_init)
                    || (top.role == Role::Object && !top.value_position)
                {
                    top.member_generator = true;
                }
                return Tok::Star;
            }
            Op::Other => {}
        }
        Tok::Punct
    }

    // ---- lexical pieces ----

    fn is_ls_ps(&self, at: usize) -> bool {
        self.src.get(at) == Some(&0xE2)
            && self.src.get(at + 1) == Some(&0x80)
            && matches!(self.src.get(at + 2), Some(0xA8 | 0xA9))
    }

    fn skip_line(&mut self) {
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\n' | b'\r' => return,
                0xE2 if self.is_ls_ps(self.i) => return,
                _ => self.i += 1,
            }
        }
    }

    fn line_break(&mut self, len: usize) {
        self.i += len;
        self.newline = true;
        self.line_start = true;
    }

    fn skip_trivia(&mut self) {
        while self.i < self.src.len() {
            match self.src[self.i] {
                b' ' | b'\t' | 0x0B | 0x0C => self.i += 1,
                b'\n' | b'\r' => self.line_break(1),
                b'/' if self.src.get(self.i + 1) == Some(&b'/') => self.skip_line(),
                b'/' if self.src.get(self.i + 1) == Some(&b'*') => {
                    self.i += 2;
                    loop {
                        if self.i >= self.src.len() {
                            return;
                        }
                        match self.src[self.i] {
                            b'*' if self.src.get(self.i + 1) == Some(&b'/') => {
                                self.i += 2;
                                break;
                            }
                            b'\n' | b'\r' => self.line_break(1),
                            0xE2 if self.is_ls_ps(self.i) => self.line_break(3),
                            _ => self.i += 1,
                        }
                    }
                }
                // Annex B HTML-like comments, Script goal only.
                b'<' if !self.module && self.src[self.i..].starts_with(b"<!--") => {
                    self.skip_line();
                }
                b'-' if !self.module
                    && self.line_start
                    && self.src[self.i..].starts_with(b"-->") =>
                {
                    self.skip_line();
                }
                0x80..=0xFF => {
                    let (len, kind) = non_ascii(&self.src[self.i..]);
                    match kind {
                        NonAscii::Space => self.i += len,
                        NonAscii::LineTerminator => self.line_break(len),
                        NonAscii::Other => return,
                    }
                }
                _ => return,
            }
        }
    }

    /// Skip a `\` escape at `self.i` in a string or template, incl. a line continuation.
    fn skip_escape(&mut self) {
        self.i += 1;
        match self.src.get(self.i) {
            Some(b'\r') if self.src.get(self.i + 1) == Some(&b'\n') => self.i += 2,
            Some(&byte) if byte >= 0x80 => self.i += non_ascii(&self.src[self.i..]).0,
            Some(_) => self.i += 1,
            None => {}
        }
    }

    fn string(&mut self, quote: u8) {
        self.i += 1;
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\\' => self.skip_escape(),
                // Unterminated (invalid input): stop at the line end.
                b'\n' | b'\r' => return,
                byte if byte == quote => {
                    self.i += 1;
                    return;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.src.len());
    }

    /// Scan template characters after `` ` `` or after the `}` of a substitution.
    fn template_chars(&mut self) -> Tok {
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\\' => self.skip_escape(),
                b'`' => {
                    self.i += 1;
                    return Tok::Literal;
                }
                b'$' if self.src.get(self.i + 1) == Some(&b'{') => {
                    let at = self.i;
                    self.i += 2;
                    self.push(Role::TemplateSubst, at);
                    return Tok::Open(Role::TemplateSubst);
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.src.len());
        Tok::Literal
    }

    /// Scan a regex literal from its opening `/`. Groups and classes nest in oxc's regex parser,
    /// which runs inside oxc's parser, so their depth adds to the current depth.
    ///
    /// The token ends as in the lexical grammar (ECMA-262 §12.9.5): a class `[...]` ends at its
    /// first unescaped `]`. With the `v` flag, classes nest in the pattern grammar; every `[`
    /// inside a class counts as one more level, an upper bound.
    fn regex(&mut self, start: usize) {
        self.i += 1;
        let mut groups = 0u32;
        let mut deepest = 0u32;
        let mut in_class = false;
        let mut class_levels = 0u32;
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\\' => self.i += 2,
                // Unterminated (invalid input).
                b'\n' | b'\r' => break,
                0xE2 if self.is_ls_ps(self.i) => break,
                b'[' => {
                    class_levels += 1;
                    in_class = true;
                    deepest = deepest.max(groups + class_levels);
                    self.i += 1;
                }
                b']' if in_class => {
                    in_class = false;
                    class_levels = 0;
                    self.i += 1;
                }
                b'(' if !in_class => {
                    groups += 1;
                    deepest = deepest.max(groups);
                    self.i += 1;
                }
                b')' if !in_class => {
                    groups = groups.saturating_sub(1);
                    self.i += 1;
                }
                b'/' if !in_class => {
                    self.i += 1;
                    break;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.src.len());
        // Flags.
        let flags_start = self.i;
        self.identifier_tail_from(flags_start);
        let depth = self
            .top_ref()
            .depth()
            .saturating_add(deepest.saturating_mul(REGEX_WEIGHT));
        self.note(depth, start);
    }

    fn number(&mut self) {
        // Digits, letters (hex digits, exponent, BigInt `n`), `_` and `.`; a sign after the
        // exponent `e`/`E` of a decimal number. Over-reading is harmless: no bracket or quote.
        let begin = self.i;
        let hex = self.src.get(begin) == Some(&b'0')
            && matches!(self.src.get(begin + 1), Some(b'x' | b'X'));
        while self.i < self.src.len() {
            let byte = self.src[self.i];
            let part = byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.';
            let exponent_sign =
                matches!(byte, b'+' | b'-') && !hex && matches!(self.src[self.i - 1], b'e' | b'E');
            if !(part || exponent_sign) {
                break;
            }
            self.i += 1;
        }
    }

    /// Scan an identifier (or regex flags, or a private name) from `start`. Returns true if it
    /// holds a `\u` escape.
    fn identifier_tail_from(&mut self, start: usize) -> bool {
        self.i = start;
        let mut escaped = false;
        while self.i < self.src.len() {
            let byte = self.src[self.i];
            if byte.is_ascii_alphanumeric() || byte == b'$' || byte == b'_' {
                self.i += 1;
            } else if byte == b'\\' {
                escaped = true;
                self.i += 2;
                if self.src.get(self.i) == Some(&b'{') {
                    while self.i < self.src.len() && self.src[self.i] != b'}' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
            } else if byte >= 0x80 {
                let (len, kind) = non_ascii(&self.src[self.i..]);
                if kind != NonAscii::Other {
                    break;
                }
                self.i += len;
            } else {
                break;
            }
        }
        self.i = self.i.min(self.src.len());
        escaped
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloserKind {
    Paren,
    Bracket,
    Brace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Semi,
    Comma,
    Question,
    Colon,
    Assign,
    Arrow,
    StarStar,
    PlusMinus,
    Bang,
    Tilde,
    IncDec,
    Spread,
    Dot,
    Star,
    Other,
}

/// Longest-match punctuator at the start of `rest`: its length and class.
fn punct(rest: &[u8]) -> (usize, Op) {
    let at = |k: usize| rest.get(k).copied().unwrap_or(0);
    match at(0) {
        b';' => (1, Op::Semi),
        b',' => (1, Op::Comma),
        b'~' => (1, Op::Tilde),
        b':' => (1, Op::Colon),
        b'@' => (1, Op::Other),
        b'?' => match at(1) {
            b'?' if at(2) == b'=' => (3, Op::Assign),
            b'?' => (2, Op::Other),
            // `?.` unless a digit follows (`a?.5:b` is a conditional).
            b'.' if !at(2).is_ascii_digit() => (2, Op::Dot),
            _ => (1, Op::Question),
        },
        b'.' => {
            if at(1) == b'.' && at(2) == b'.' {
                (3, Op::Spread)
            } else {
                (1, Op::Dot)
            }
        }
        b'=' => match at(1) {
            b'>' => (2, Op::Arrow),
            b'=' if at(2) == b'=' => (3, Op::Other),
            b'=' => (2, Op::Other),
            _ => (1, Op::Assign),
        },
        b'!' => match at(1) {
            b'=' if at(2) == b'=' => (3, Op::Other),
            b'=' => (2, Op::Other),
            _ => (1, Op::Bang),
        },
        b'+' | b'-' => {
            let c = at(0);
            if at(1) == c {
                (2, Op::IncDec)
            } else if at(1) == b'=' {
                (2, Op::Assign)
            } else {
                (1, Op::PlusMinus)
            }
        }
        b'*' => match (at(1), at(2)) {
            (b'*', b'=') => (3, Op::Assign),
            (b'*', _) => (2, Op::StarStar),
            (b'=', _) => (2, Op::Assign),
            _ => (1, Op::Star),
        },
        b'/' | b'%' | b'^' => {
            if at(1) == b'=' {
                (2, Op::Assign)
            } else {
                (1, Op::Other)
            }
        }
        b'&' | b'|' => {
            let c = at(0);
            match (at(1), at(2)) {
                (x, b'=') if x == c => (3, Op::Assign),
                (x, _) if x == c => (2, Op::Other),
                (b'=', _) => (2, Op::Assign),
                _ => (1, Op::Other),
            }
        }
        b'<' => match (at(1), at(2)) {
            (b'<', b'=') => (3, Op::Assign),
            (b'<', _) | (b'=', _) => (2, Op::Other),
            _ => (1, Op::Other),
        },
        b'>' => match (at(1), at(2), at(3)) {
            (b'>', b'>', b'=') => (4, Op::Assign),
            (b'>', b'>', _) => (3, Op::Other),
            (b'>', b'=', _) => (3, Op::Assign),
            (b'>', _, _) | (b'=', _, _) => (2, Op::Other),
            _ => (1, Op::Other),
        },
        // Any other byte (a stray character in invalid input): one byte.
        _ => (1, Op::Other),
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'$' || byte == b'_' || byte == b'\\' || byte >= 0x80
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NonAscii {
    /// ECMA-262 WhiteSpace outside ASCII: U+00A0, U+FEFF and category Zs.
    Space,
    /// U+2028, U+2029.
    LineTerminator,
    /// Anything else (identifier characters, or invalid input).
    Other,
}

/// Classify the non-ASCII character at the start of `rest`; returns its byte length.
fn non_ascii(rest: &[u8]) -> (usize, NonAscii) {
    let lead = rest[0];
    let len = match lead {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
    .min(rest.len());
    let kind = match &rest[..len] {
        // U+00A0
        [0xC2, 0xA0] => NonAscii::Space,
        // U+1680
        [0xE1, 0x9A, 0x80] => NonAscii::Space,
        // U+2000..=U+200A
        [0xE2, 0x80, 0x80..=0x8A] => NonAscii::Space,
        // U+2028, U+2029
        [0xE2, 0x80, 0xA8 | 0xA9] => NonAscii::LineTerminator,
        // U+202F
        [0xE2, 0x80, 0xAF] => NonAscii::Space,
        // U+205F
        [0xE2, 0x81, 0x9F] => NonAscii::Space,
        // U+3000
        [0xE3, 0x80, 0x80] => NonAscii::Space,
        // U+FEFF
        [0xEF, 0xBB, 0xBF] => NonAscii::Space,
        _ => NonAscii::Other,
    };
    (len.max(1), kind)
}

#[cfg(test)]
mod tests;
