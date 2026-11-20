//! The code lists of the record (H37-Q1, H37-Q2). Each list is ep9-owned and append-only: a
//! code keeps its meaning for ever; an append bumps `HINT_SCHEMA_VERSION` (crate docs).

macro_rules! code_list {
    (
        $(#[$meta:meta])*
        $name:ident: $ty:ty {
            $( $(#[$vmeta:meta])* $variant:ident = $code:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr($ty)]
        pub enum $name {
            $( $(#[$vmeta])* $variant = $code, )+
        }

        impl $name {
            /// Every value of the list, in code order.
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            /// The code of this value in the file.
            pub const fn code(self) -> $ty {
                self as $ty
            }

            /// The value of a code, or `None` for a code that the list does not have.
            pub const fn from_code(code: $ty) -> Option<$name> {
                match code {
                    $( $code => Some($name::$variant), )+
                    _ => None,
                }
            }

            /// The stable name of this value (for reports).
            pub const fn name(self) -> &'static str {
                match self {
                    $( $name::$variant => stringify!($variant), )+
                }
            }
        }
    };
}

code_list! {
    /// The node kind of a record key (H37-Q2): an ep9-owned list. `ep9_frontend` maps oxc AST
    /// kinds to it and `ep9-typecheck` maps tsrs kinds to it. The names are the oxc AST node
    /// names, because the key is the oxc span (S1). Code 0 is not a kind, so a zeroed record is
    /// a bad record.
    NodeKind: u16 {
        /// A read of a name (`IdentifierReference`).
        IdentifierReference = 1,
        /// A name that a declaration binds (`BindingIdentifier`).
        BindingIdentifier = 2,
        /// `a.b`.
        StaticMemberExpression = 3,
        /// `a[b]`.
        ComputedMemberExpression = 4,
        /// `a.#b`.
        PrivateFieldExpression = 5,
        /// `f()`.
        CallExpression = 6,
        /// `new C()`.
        NewExpression = 7,
        /// `(e)`: the key of a JSDoc cast (S4).
        ParenthesizedExpression = 8,
        /// A function declaration or expression.
        Function = 9,
        /// `() => e`.
        ArrowFunctionExpression = 10,
        /// A class declaration or expression.
        Class = 11,
        /// A class field.
        PropertyDefinition = 12,
        /// A class method, getter, setter or constructor.
        MethodDefinition = 13,
        /// A function parameter.
        FormalParameter = 14,
        /// `{ ... }` as an expression.
        ObjectExpression = 15,
        /// `[ ... ]` as an expression.
        ArrayExpression = 16,
        /// `this`.
        ThisExpression = 17,
        /// One declarator of a `var`, `let` or `const`.
        VariableDeclarator = 18,
        /// `a + b` and the other binary operators.
        BinaryExpression = 19,
        /// `a && b`, `a || b`, `a ?? b`.
        LogicalExpression = 20,
        /// `a = b` and the compound assignments.
        AssignmentExpression = 21,
        /// `-a`, `!a`, `typeof a` and the other unary operators.
        UnaryExpression = 22,
        /// `a++`, `--a`.
        UpdateExpression = 23,
        /// `c ? a : b`.
        ConditionalExpression = 24,
        /// A number literal.
        NumericLiteral = 25,
        /// A string literal.
        StringLiteral = 26,
        /// `` `...${e}...` ``.
        TemplateLiteral = 27,
        /// `await e`.
        AwaitExpression = 28,
        /// `e as T`.
        TSAsExpression = 29,
        /// `e!`.
        TSNonNullExpression = 30,
    }
}

code_list! {
    /// The repr category of a record and of its element type (H37-Q1). Code 0 is the null
    /// value: no hint.
    Repr: u8 {
        /// No hint (the null value).
        None = 0,
        /// A number.
        Number = 1,
        /// A string.
        String = 2,
        /// A boolean.
        Boolean = 3,
        /// A BigInt.
        BigInt = 4,
        /// A symbol.
        Symbol = 5,
        /// `undefined` (also TS `void`).
        Undefined = 6,
        /// `null`.
        Null = 7,
        /// An object that is not an array, a function or a class instance.
        Object = 8,
        /// An array.
        Array = 9,
        /// A function.
        Function = 10,
        /// An instance of a class. On a kind that is not a member access, only this repr can
        /// have a class id, and the id names the class (else `BadRecord`). On a member-access
        /// kind, the class id is the receiver's class with any repr, not the value's class.
        ClassInstance = 11,
    }
}
