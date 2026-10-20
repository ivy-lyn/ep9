//! Nesting forms for the depth tests and the stack probe (test only).
//!
//! `M0` forms are M0 exit item 7; `M1` forms are M1 exit item 8; `Extra` forms cover the other
//! recursive productions of oxc's parser, so the stack sizing holds for them too.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    M0,
    M1,
    Extra,
}

pub(crate) struct Form {
    pub(crate) name: &'static str,
    pub(crate) gate: Gate,
    pub(crate) module: bool,
    pub(crate) build: fn(usize) -> String,
}

fn wrap(open: &str, mid: &str, close: &str, n: usize) -> String {
    let mut s = String::with_capacity((open.len() + close.len()) * n + mid.len());
    for _ in 0..n {
        s.push_str(open);
    }
    s.push_str(mid);
    for _ in 0..n {
        s.push_str(close);
    }
    s
}

fn chain(head: &str, link: &str, n: usize) -> String {
    let mut s = String::with_capacity(head.len() + link.len() * n);
    s.push_str(head);
    for _ in 0..n {
        s.push_str(link);
    }
    s
}

pub(crate) const FORMS: &[Form] = &[
    // ---- M0 exit item 7 ----
    Form {
        name: "bracket-array",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("[", "0", "]", n),
    },
    Form {
        name: "bracket-paren",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("(", "0", ")", n),
    },
    Form {
        name: "bracket-object",
        gate: Gate::M0,
        module: false,
        build: |n| format!("x={}", wrap("{a:", "0", "}", n)),
    },
    Form {
        name: "bracket-block",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("{", "", "}", n),
    },
    Form {
        name: "bracket-function",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("function f(){", "", "}", n),
    },
    Form {
        name: "binary",
        gate: Gate::M0,
        module: false,
        build: |n| chain("1", "+1", n),
    },
    Form {
        name: "member",
        gate: Gate::M0,
        module: false,
        build: |n| chain("a", ".b", n),
    },
    Form {
        name: "call",
        gate: Gate::M0,
        module: false,
        build: |n| chain("f", "()", n),
    },
    Form {
        name: "comma-nested",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("(0,", "0", ")", n),
    },
    Form {
        name: "template",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("`${", "0", "}`", n),
    },
    Form {
        name: "tagged-template",
        gate: Gate::M0,
        module: false,
        build: |n| chain("f", "`x`", n),
    },
    Form {
        name: "class-body",
        gate: Gate::M0,
        module: false,
        build: |n| wrap("class A{m(){", "", "}}", n),
    },
    Form {
        name: "class-heritage",
        gate: Gate::M0,
        module: false,
        build: |n| format!("x={}", wrap("class extends ", "Object", "{}", n)),
    },
    // ---- M1 exit item 8 ----
    Form {
        name: "conditional",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "a?a:", n) + "a",
    },
    Form {
        name: "conditional-consequent",
        gate: Gate::M1,
        module: false,
        build: |n| wrap("a?", "a", ":a", n),
    },
    Form {
        name: "assignment",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "a=", n) + "0",
    },
    Form {
        name: "unary",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "!", n) + "a",
    },
    Form {
        name: "unary-typeof",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "typeof ", n) + "a",
    },
    Form {
        name: "arrow",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "x=>", n) + "0",
    },
    Form {
        name: "exponent",
        gate: Gate::M1,
        module: false,
        build: |n| chain("", "a**", n) + "a",
    },
    // ---- other recursive productions ----
    Form {
        name: "optional-chain",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("a", "?.b", n),
    },
    Form {
        name: "new",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("", "new ", n) + "X",
    },
    Form {
        name: "spread",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("[...", "a", "]", n),
    },
    Form {
        name: "if",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("", "if(a)", n) + ";",
    },
    Form {
        name: "else-if",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("if(a);", "else if(a);", n),
    },
    Form {
        name: "label",
        gate: Gate::Extra,
        module: false,
        build: |n| (0..n).map(|i| format!("l{i}:")).collect::<String>() + ";",
    },
    Form {
        name: "do-while",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("do ", ";", " while(0)", n),
    },
    Form {
        name: "for",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("", "for(;;)", n) + ";",
    },
    Form {
        name: "with",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("", "with(a)", n) + ";",
    },
    Form {
        name: "try",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("try{", "", "}finally{}", n),
    },
    Form {
        name: "switch",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("switch(a){case 0:", "", "}", n),
    },
    Form {
        name: "arrow-block",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("()=>{", "", "}", n),
    },
    Form {
        name: "async-arrow",
        gate: Gate::Extra,
        module: false,
        build: |n| chain("", "async x=>", n) + "0",
    },
    Form {
        name: "array-pattern",
        gate: Gate::Extra,
        module: false,
        build: |n| wrap("[", "a", "]", n) + "=x",
    },
    Form {
        name: "object-pattern",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("var {}=x", wrap("{a:", "b", "}", n)),
    },
    Form {
        name: "param-pattern",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("function f({}){{}}", wrap("[", "a", "]", n)),
    },
    Form {
        name: "await",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("async function f(){{{}x}}", "await ".repeat(n)),
    },
    Form {
        name: "yield",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("function*g(){{{}x}}", "yield ".repeat(n)),
    },
    Form {
        name: "regex-group",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/", wrap("(", "a", ")", n)),
    },
    Form {
        name: "regex-class-v",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/v", wrap("[", "a", "]", n)),
    },
    // With `v`, a nested class can follow a closed one: the class stays open after its first `]`.
    Form {
        name: "regex-class-v-intersection",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/v", wrap("[[a]&&", "[a]", "]", n)),
    },
    Form {
        name: "regex-class-v-subtraction",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/v", wrap("[[a]--", "[a]", "]", n)),
    },
    Form {
        name: "regex-class-v-union",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/v", wrap("[\\q{a}", "a", "]", n)),
    },
    // Group kinds take different paths through oxc's regex parser.
    Form {
        name: "regex-group-u",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/u", wrap("(", "a", ")", n)),
    },
    Form {
        name: "regex-named-group",
        gate: Gate::Extra,
        module: false,
        build: |n| {
            let open = (0..n).map(|i| format!("(?<g{i}>")).collect::<String>();
            format!("/{open}a{}/", ")".repeat(n))
        },
    },
    Form {
        name: "regex-ignore-group",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/", wrap("(?:", "a", ")", n)),
    },
    Form {
        name: "regex-lookahead",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/", wrap("(?=", "a", ")", n)),
    },
    Form {
        name: "regex-lookbehind",
        gate: Gate::Extra,
        module: false,
        build: |n| format!("/{}/", wrap("(?<=", "a", ")", n)),
    },
    Form {
        name: "module-block",
        gate: Gate::Extra,
        module: true,
        build: |n| wrap("{", "", "}", n),
    },
];
