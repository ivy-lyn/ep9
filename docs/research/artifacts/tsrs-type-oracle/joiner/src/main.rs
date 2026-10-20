//! Dump every oxc_parser AST node (kind, span) for a list of files as JSONL, for joining with the tsrs side table.
//! usage: oxc_span_dump <out.jsonl> <file>...   (or file list on stdin with `-`)
use std::io::{BufRead, Write};

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType};
use serde_json::json;

struct Collector {
    out: Vec<(String, u32, u32, bool, String)>,
    stack: Vec<(bool, String)>, // in-type flag + kind per open node
}

fn is_type_kind(name: &str) -> bool {
    name.starts_with("TS")
        && (name.ends_with("Type")
            || matches!(
                name,
                "TSTypeAnnotation" | "TSTypeReference" | "TSQualifiedName" | "TSTypeAliasDeclaration"
                    | "TSInterfaceDeclaration" | "TSInterfaceBody" | "TSInterfaceHeritage" | "TSClassImplements"
                    | "TSTypeParameter" | "TSTypeParameterDeclaration" | "TSTypeParameterInstantiation"
                    | "TSPropertySignature" | "TSMethodSignature" | "TSIndexSignature" | "TSCallSignatureDeclaration"
                    | "TSConstructSignatureDeclaration" | "TSTypeQuery" | "TSTypePredicate" | "TSImportType"
                    | "TSTupleElement" | "TSNamedTupleMember" | "TSTemplateLiteralType" | "TSLiteralType"
            ))
}

impl<'a> Visit<'a> for Collector {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        let sp = kind.span();
        let mut name = format!("{:?}", kind.ty());
        match kind {
            AstKind::Function(f) => {
                name = if f.is_expression() { "FunctionExpression".into() } else { "FunctionDeclaration".into() }
            }
            AstKind::ObjectProperty(p) => {
                if p.method || p.kind != oxc_ast::ast::PropertyKind::Init {
                    name = "ObjectPropertyMethod".into();
                }
            }
            AstKind::Class(c) => {
                name = if c.is_expression() { "ClassExpression".into() } else { "ClassDeclaration".into() }
            }
            _ => {}
        }
        let parent = self.stack.last().map(|p| p.1.clone()).unwrap_or_default();
        let in_type = self.stack.last().is_some_and(|p| p.0) || is_type_kind(&name);
        self.stack.push((in_type, name.clone()));
        self.out.push((name, sp.start, sp.end, in_type, parent));
    }
    fn leave_node(&mut self, _kind: AstKind<'a>) {
        self.stack.pop();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut files: Vec<String> = args[2..].to_vec();
    if files.len() == 1 && files[0] == "-" {
        files = std::io::stdin().lock().lines().map(|l| l.unwrap()).collect();
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(&args[1]).unwrap());
    let mut total_ns = 0u128;
    for f in &files {
        let raw = std::fs::read_to_string(f).unwrap();
        // tsrs (like tsc's sys.readFile) strips a leading UTF-8 BOM, so its offsets are relative to the stripped text.
        let bom = raw.starts_with('\u{feff}');
        let text = if bom { raw['\u{feff}'.len_utf8()..].to_string() } else { raw };
        let st = SourceType::from_path(f).unwrap_or_default();
        let alloc = Allocator::default();
        let t0 = std::time::Instant::now();
        let ret = Parser::new(&alloc, &text, st)
            .with_options(ParseOptions { preserve_parens: true, ..ParseOptions::default() })
            .parse();
        total_ns += t0.elapsed().as_nanos();
        let mut c = Collector { out: Vec::new(), stack: Vec::new() };
        c.visit_program(&ret.program);
        let nodes: Vec<_> = c.out.iter().map(|(k, s, e, t, p)| json!([k, s, e, u8::from(*t), p])).collect();
        serde_json::to_writer(
            &mut w,
            &json!({"file": f, "errors": ret.diagnostics.len(), "bom": bom, "fatal": ret.fatal_error,
                    "module": st.is_module(), "ts": st.is_typescript(), "nodes": nodes}),
        )
        .unwrap();
        w.write_all(b"\n").unwrap();
    }
    eprintln!("oxc parse total {:.2} ms for {} files", total_ns as f64 / 1e6, files.len());
}
