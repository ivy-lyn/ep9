#![allow(dead_code)] // m3-stub
//! Module code: the module records (ABI 5 section 5), the import and export declarations,
//! `export default`, `import()` and `import.meta` (ECMA-262 §16.2).
//!
//! M3-P0 routing stub (P0-D7, V8-C10): the statement and expression entries give today's
//! result (an import or export declaration emits nothing; `unsupported: import()`,
//! `unsupported: import.meta`), so no test result changes. The M3-P2 MODULE role owns this file
//! whole and fills it (ABI 5 runtime ids 42-43 `GetImportMeta`, `DynamicImport`; the loader
//! reads [`module_records`]); M3-P3 IMPORT-TLA builds on it.
//!
//! ```text
//! caller (site at b6109b3)                         entry here
//! mod.rs    generate (module code)                 module_records     section 5 of the unit
//! stmt.rs   import / export arms (:130-132, :135)  module_declaration nothing (bindings are cells)
//! stmt.rs   ExportDeclaration (:133)               export_declaration the declaration
//! stmt.rs   ExportDefaultDeclaration (:134, :300)  export_default     the value -> `*default*`
//! expr.rs   ImportExpression (:336)                import_call        `import(s, o)`
//! expr.rs   ImportMeta (:358)                      import_meta        `import.meta`
//! ```
//!
//! [`module_records`] is the ParseModule part of §16.2.1.6: the requested modules (one per
//! distinct specifier and attribute list, in source order), the import entries, and the export
//! entries sorted into local, indirect and star entries (step 10). Each import and local
//! export entry names its module cell (`module_cells`: exports `1, 2, ...`, imports `-1, -2,
//! ...`). Import attribute keys are sorted (WithClauseToAttributes). The `source` and `defer`
//! import phases (stage 3 proposals) are not recorded.

use std::collections::HashMap;

use ep9_bytecode::{
    ExportImportName, ImportEntry, ImportName, IndirectExportEntry, LocalExportEntry,
    ModuleRecords, ModuleRequest, UnitString,
};
use oxc_ast::ast::{
    ArrowFunctionExpression, AwaitExpression, Declaration, ExportDeclaration,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, ForOfStatement, Function,
    ImportAttributeKey, ImportDeclarationSpecifier, ImportExpression, ImportMeta, ModuleExportName,
    Program, PropertyDefinition, Statement, StaticBlock, StringLiteral, VariableDeclaration,
    VariableDeclarationKind, WithClause,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::ScopeFlags;

use super::func::{FnGen, Gen};
use super::literal::js_string;
use crate::scope::{BindingId, ScopeAnalysis, ScopeId};

/// The local name of an anonymous default export (§16.2.3.7).
const DEFAULT_LOCAL: &str = "*default*";

impl<'a> FnGen<'a, '_> {
    /// An import declaration, `export { ... }`, `export { ... } from`, `export * from`: no
    /// code. The bindings are module cells; the records are in section 5.
    pub(crate) fn module_declaration(&mut self, s: &'a Statement<'a>) -> Gen {
        let _ = s;
        Ok(())
    }

    /// `export var|let|const|function|class ...`: the declaration itself.
    pub(crate) fn export_declaration(&mut self, e: &'a ExportDeclaration<'a>) -> Gen {
        match &e.declaration {
            Declaration::VariableDeclaration(v) => self.var_decl(v),
            Declaration::FunctionDeclaration(f) => self.function_declaration(f),
            Declaration::ClassDeclaration(c) => self.class_declaration(c, None),
            _ => self.unsupported("TypeScript declarations", e.span),
        }
    }

    /// `export default ...`: a function or class declaration, or an expression stored into
    /// the `*default*` binding (NamedEvaluation with "default").
    pub(crate) fn export_default(&mut self, e: &'a ExportDefaultDeclaration<'a>) -> Gen {
        match &e.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => self.function_declaration(f),
            ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                let default = c.id.is_none().then(|| self.default_binding()).transpose()?;
                self.class_declaration(c, default)
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => Ok(()),
            other => {
                let Some(expr) = other.as_expression() else {
                    return self.unsupported("export default form", e.span);
                };
                self.stmt_pos(e.span);
                self.named_value(expr, Some(UnitString::new("default")))?;
                let b = self.default_binding()?;
                self.store_binding_init(b);
                Ok(())
            }
        }
    }

    /// The `*default*` binding of a module.
    pub(crate) fn default_binding(&self) -> Gen<BindingId> {
        self.sa
            .scope(ScopeId(0))
            .bindings
            .iter()
            .copied()
            .find(|&b| self.sa.name(self.binding(b).name) == DEFAULT_LOCAL)
            .ok_or_else(|| super::internal("no *default* binding"))
    }

    /// `import(specifier, options)` (§13.3.10).
    pub(crate) fn import_call(&mut self, i: &'a ImportExpression<'a>) -> Gen {
        self.unsupported("import()", i.span)
    }

    /// `import.meta` (§13.3.12).
    pub(crate) fn import_meta(&mut self, m: &'a ImportMeta) -> Gen {
        self.unsupported("import.meta", m.span)
    }
}

/// One ExportEntry Record before ParseModule step 10 sorts it.
struct ExportEntry {
    export_name: Option<UnitString>,
    module_request: Option<u32>,
    import_name: ExportImport,
    local_name: Option<UnitString>,
}

/// `[[ImportName]]` of an ExportEntry Record.
enum ExportImport {
    /// null (a local export).
    None,
    /// `all` (`export * as ns from`).
    All,
    /// `all-but-default` (`export * from`).
    AllButDefault,
    /// A name (`export { a } from`).
    Name(UnitString),
}

/// The records in the making.
struct Collector<'s> {
    /// Module cell per top-level binding name (imports and exported bindings).
    cells: HashMap<&'s str, i32>,
    records: ModuleRecords<UnitString>,
    exports: Vec<ExportEntry>,
}

impl Collector<'_> {
    /// The index of the ModuleRequest Record of `source` with `with` (ModuleRequests: a new
    /// one unless an equal one is already listed; ModuleRequestsEqual compares the specifier
    /// and the attributes, which are sorted by key).
    fn request(&mut self, source: &StringLiteral<'_>, with: Option<&WithClause<'_>>) -> u32 {
        let specifier = js_string(source.value.encode_utf16());
        let mut attributes: Vec<(UnitString, UnitString)> = with
            .map(|w| {
                w.with_entries
                    .iter()
                    .map(|a| {
                        let key = match &a.key {
                            ImportAttributeKey::Identifier(id) => UnitString::new(id.name.as_str()),
                            ImportAttributeKey::StringLiteral(s) => {
                                js_string(s.value.encode_utf16())
                            }
                        };
                        (key, js_string(a.value.value.encode_utf16()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // WithClauseToAttributes sorts by key; duplicate keys are an early error.
        attributes.sort_by(|a, b| a.0.cmp_code_units(&b.0));
        attributes.dedup_by(|a, b| a.0 == b.0);
        let request = ModuleRequest {
            specifier,
            attributes,
        };
        let list = &mut self.records.requested_modules;
        match list.iter().position(|r| *r == request) {
            Some(i) => i as u32,
            None => {
                list.push(request);
                list.len() as u32 - 1
            }
        }
    }

    fn cell(&self, name: &UnitString) -> Result<i32, Box<super::CodegenError>> {
        let text = String::from_utf16_lossy(&name.to_utf16());
        self.cells
            .get(text.as_str())
            .copied()
            .ok_or_else(|| super::internal(format!("module records: no module cell for {text}")))
    }

    fn local_export(&mut self, export_name: UnitString, local_name: UnitString) {
        self.exports.push(ExportEntry {
            export_name: Some(export_name),
            module_request: None,
            import_name: ExportImport::None,
            local_name: Some(local_name),
        });
    }

    fn item(&mut self, s: &Statement<'_>) {
        match s {
            Statement::ImportDeclaration(d) => {
                if d.import_kind.is_type() {
                    return;
                }
                let request = self.request(&d.source, d.with_clause.as_deref());
                for spec in d.specifiers.iter().flatten() {
                    let (import_name, local) = match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            if s.import_kind.is_type() {
                                continue;
                            }
                            (ImportName::Name(export_name(&s.imported)), &s.local)
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (ImportName::Name(UnitString::new("default")), &s.local)
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            (ImportName::Namespace, &s.local)
                        }
                    };
                    self.records.import_entries.push(ImportEntry {
                        module_request: request,
                        import_name,
                        local_name: UnitString::new(local.name.as_str()),
                        cell: 0, // filled below
                    });
                }
            }
            Statement::ExportNamedDeclaration(d) => {
                if d.export_kind.is_type() {
                    return;
                }
                for spec in d.specifiers.iter().filter(|s| !s.export_kind.is_type()) {
                    self.local_export(export_name(&spec.exported), export_name(&spec.local));
                }
            }
            Statement::ExportFromDeclaration(d) => {
                if d.export_kind.is_type() {
                    return;
                }
                let request = self.request(&d.source, d.with_clause.as_deref());
                for spec in d.specifiers.iter().filter(|s| !s.export_kind.is_type()) {
                    self.exports.push(ExportEntry {
                        export_name: Some(export_name(&spec.exported)),
                        module_request: Some(request),
                        import_name: ExportImport::Name(export_name(&spec.local)),
                        local_name: None,
                    });
                }
            }
            Statement::ExportAllDeclaration(d) => {
                if d.export_kind.is_type() {
                    return;
                }
                let request = self.request(&d.source, d.with_clause.as_deref());
                let (export_name, import_name) = match &d.exported {
                    Some(n) => (Some(export_name(n)), ExportImport::All),
                    None => (None, ExportImport::AllButDefault),
                };
                self.exports.push(ExportEntry {
                    export_name,
                    module_request: Some(request),
                    import_name,
                    local_name: None,
                });
            }
            Statement::ExportDeclaration(d) => {
                let mut names = Vec::new();
                match &d.declaration {
                    Declaration::VariableDeclaration(v) => bound_names(v, &mut names),
                    Declaration::FunctionDeclaration(f) => {
                        names.extend(f.id.as_ref().map(|id| id.name.as_str()));
                    }
                    Declaration::ClassDeclaration(c) => {
                        names.extend(c.id.as_ref().map(|id| id.name.as_str()));
                    }
                    _ => {}
                }
                for n in names {
                    self.local_export(UnitString::new(n), UnitString::new(n));
                }
            }
            Statement::ExportDefaultDeclaration(d) => {
                let local = match &d.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        f.id.as_ref().map_or(DEFAULT_LOCAL, |id| id.name.as_str())
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        c.id.as_ref().map_or(DEFAULT_LOCAL, |id| id.name.as_str())
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => return,
                    _ => DEFAULT_LOCAL,
                };
                self.local_export(UnitString::new("default"), UnitString::new(local));
            }
            _ => {}
        }
    }
}

/// The StringValue of a ModuleExportName.
fn export_name(n: &ModuleExportName<'_>) -> UnitString {
    match n {
        ModuleExportName::IdentifierName(id) => UnitString::new(id.name.as_str()),
        ModuleExportName::IdentifierReference(id) => UnitString::new(id.name.as_str()),
        ModuleExportName::StringLiteral(s) => js_string(s.value.encode_utf16()),
    }
}

/// BoundNames of a variable declaration, in source order.
fn bound_names<'b>(v: &'b VariableDeclaration<'_>, out: &mut Vec<&'b str>) {
    for d in &v.declarations {
        out.extend(
            d.id.get_binding_identifiers()
                .iter()
                .map(|id| id.name.as_str()),
        );
    }
}

/// `[[HasTLA]]`: an `await` (or `for await`, `await using`) that the module body itself
/// evaluates. Functions, arrows, class field initializers and static blocks are their own
/// code; class computed keys and heritage run in the module body.
struct TopLevelAwait(bool);

impl<'a> Visit<'a> for TopLevelAwait {
    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.0 = true;
        walk::walk_await_expression(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.0 |= it.r#await;
        walk::walk_for_of_statement(self, it);
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.0 |= it.kind == VariableDeclarationKind::AwaitUsing;
        walk::walk_variable_declaration(self, it);
    }

    fn visit_function(&mut self, _it: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _it: &ArrowFunctionExpression<'a>) {}

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.computed {
            self.visit_property_key(&it.key);
        }
    }

    fn visit_static_block(&mut self, _it: &StaticBlock<'a>) {}
}

/// The module records of a module (ParseModule, §16.2.1.6 steps 4-10): `cells` is the module
/// cell of each binding (`module_cells`). An import or local export whose binding has no cell
/// is a BytecodeGen bug.
pub(crate) fn module_records(
    program: &Program<'_>,
    sa: &ScopeAnalysis,
    cells: &[i32],
) -> Result<ModuleRecords<UnitString>, Box<super::CodegenError>> {
    let mut by_name = HashMap::new();
    for &b in &sa.scope(ScopeId(0)).bindings {
        if cells[b.index()] != 0 {
            by_name.insert(sa.name(sa.binding(b).name), cells[b.index()]);
        }
    }
    let mut c = Collector {
        cells: by_name,
        records: ModuleRecords::default(),
        exports: Vec::new(),
    };
    for s in &program.body {
        c.item(s);
    }
    for i in 0..c.records.import_entries.len() {
        let cell = c.cell(&c.records.import_entries[i].local_name)?;
        c.records.import_entries[i].cell = cell;
    }
    // Step 10: sort the export entries.
    for ee in std::mem::take(&mut c.exports) {
        match (
            ee.module_request,
            ee.import_name,
            ee.export_name,
            ee.local_name,
        ) {
            (None, _, Some(export_name), Some(local_name)) => {
                let import = c
                    .records
                    .import_entries
                    .iter()
                    .find(|ie| ie.local_name == local_name);
                match import {
                    // A re-export of an imported binding: an indirect entry.
                    Some(ie) if ie.import_name != ImportName::Namespace => {
                        let import_name = match &ie.import_name {
                            ImportName::Name(n) => ExportImportName::Name(n.clone()),
                            ImportName::Namespace => unreachable!("matched above"),
                        };
                        c.records.indirect_export_entries.push(IndirectExportEntry {
                            export_name,
                            module_request: ie.module_request,
                            import_name,
                        });
                    }
                    // A local binding, or a re-export of an imported namespace object.
                    _ => {
                        let cell = c.cell(&local_name)?;
                        c.records.local_export_entries.push(LocalExportEntry {
                            export_name,
                            local_name,
                            cell,
                        });
                    }
                }
            }
            (Some(r), ExportImport::AllButDefault, None, None) => {
                c.records.star_export_entries.push(r);
            }
            (Some(r), ExportImport::All, Some(export_name), None) => {
                c.records.indirect_export_entries.push(IndirectExportEntry {
                    export_name,
                    module_request: r,
                    import_name: ExportImportName::All,
                });
            }
            (Some(r), ExportImport::Name(n), Some(export_name), None) => {
                c.records.indirect_export_entries.push(IndirectExportEntry {
                    export_name,
                    module_request: r,
                    import_name: ExportImportName::Name(n),
                });
            }
            _ => return Err(super::internal("module records: malformed export entry")),
        }
    }
    let mut tla = TopLevelAwait(false);
    for s in &program.body {
        tla.visit_statement(s);
    }
    c.records.has_tla = tla.0;
    Ok(c.records)
}
