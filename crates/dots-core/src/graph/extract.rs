//! The facts of one file (project graph spec section 4): its symbols, imports, calls and
//! bases, read from a tree-sitter tree, or from lines for Markdown. Nothing here resolves a
//! name to another file: that is `link`, which works from the stored facts.

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

/// A language the builder reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Markdown,
}

impl Lang {
    /// The language of a file name, by its extension.
    pub fn of(path: &str) -> Option<Lang> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "py" | "pyi" => Lang::Python,
            "go" => Lang::Go,
            "md" | "markdown" => Lang::Markdown,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::Markdown => "markdown",
        }
    }

    /// TypeScript, TSX and JavaScript share their import rules.
    pub fn is_script(self) -> bool {
        matches!(self, Lang::TypeScript | Lang::Tsx | Lang::JavaScript)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymKind {
    Type,
    Function,
    Method,
    Heading,
}

/// A definition in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymKind,
    /// 1-based.
    pub line: u32,
    /// The type a method belongs to (`impl X`, `class X`, a Go receiver).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The enclosing symbol, by index: a heading's parent heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// The first line of the definition, trimmed and cut.
    pub signature: String,
    /// A Rust function marked `#[tauri::command]`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub command: bool,
}

/// What an import binds: `name` is the imported name (`""` the module itself, `"*"` all of
/// it), `local` the name the file uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportItem {
    pub name: String,
    pub local: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// As written: a Rust path (`crate::a::B`), a script module (`./a`, `react`), a Python
    /// module (`a.b`, `.c`), a Go import path.
    pub spec: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ImportItem>,
    pub line: u32,
    /// A Rust `mod x;`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub module: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// The calling symbol, by index; `None` for code outside any function.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    pub name: String,
    /// `a` of `a::f()`, `a.f()`, `a.F()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// `self.f()`, `this.f()`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_self: bool,
    /// Called on a value (`x.f()`, `a.b().f()`), not by a bare or path name: only a method
    /// can be what it calls.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub method: bool,
    pub line: u32,
}

/// A type's base: `implements` for a Rust trait impl or a TS `implements`, else `extends`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// The type, by name (a Rust impl may be in another file than its type).
    pub of: String,
    pub name: String,
    pub implements: bool,
    pub line: u32,
}

/// A path a Markdown section names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub from: u32,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub imports: Vec<Import>,
    #[serde(default)]
    pub calls: Vec<Call>,
    #[serde(default)]
    pub bases: Vec<Base>,
    #[serde(default)]
    pub mentions: Vec<Mention>,
}

/// The longest signature kept, in characters.
const MAX_SIGNATURE: usize = 160;
/// The most calls kept per file: a generated file must not swamp the graph.
const MAX_CALLS: usize = 5_000;

/// The facts of `text`, a file in `lang`. `Err` when the parser cannot be made.
pub fn extract(lang: Lang, text: &str) -> Result<Facts, String> {
    if lang == Lang::Markdown {
        return Ok(markdown(text));
    }
    let language: tree_sitter::Language = match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::Markdown => unreachable!(),
    };
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| format!("the {} parser cannot start: {e}", lang.name()))?;
    let tree = parser
        .parse(text, None)
        .ok_or_else(|| format!("the {} parser gave up", lang.name()))?;
    let mut walker = Walker {
        lang,
        src: text.as_bytes(),
        facts: Facts::default(),
        owner: None,
        current: None,
    };
    walker.walk(tree.root_node(), true);
    Ok(walker.facts)
}

struct Walker<'a> {
    lang: Lang,
    src: &'a [u8],
    facts: Facts,
    /// The type whose body the walk is in.
    owner: Option<String>,
    /// The function whose body the walk is in.
    current: Option<u32>,
}

fn line_of(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &'a str {
        node.utf8_text(self.src).unwrap_or_default()
    }

    fn field_text(&self, node: Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| self.text(n).to_string())
    }

    /// The first line of `node`, cut before its body.
    fn signature(&self, node: Node) -> String {
        let text = self.text(node);
        let first = text.lines().next().unwrap_or_default();
        let first = first
            .split(" {")
            .next()
            .unwrap_or(first)
            .trim_end_matches('{');
        let first = first.trim();
        match first.char_indices().nth(MAX_SIGNATURE) {
            Some((at, _)) => format!("{}…", &first[..at]),
            None => first.to_string(),
        }
    }

    fn add_symbol(&mut self, node: Node, name: String, kind: SymKind, command: bool) -> u32 {
        let owner = (kind == SymKind::Method)
            .then(|| self.owner.clone())
            .flatten();
        self.facts.symbols.push(Symbol {
            name,
            kind,
            line: line_of(node),
            owner,
            parent: None,
            signature: self.signature(node),
            command,
        });
        (self.facts.symbols.len() - 1) as u32
    }

    fn add_call(&mut self, node: Node, name: &str, qualifier: Option<String>, on_self: bool) {
        self.push_call(node, name, qualifier, on_self, false);
    }

    /// A call on a value: `x.f()`.
    fn add_method_call(
        &mut self,
        node: Node,
        name: &str,
        qualifier: Option<String>,
        on_self: bool,
    ) {
        self.push_call(node, name, qualifier, on_self, true);
    }

    fn push_call(
        &mut self,
        node: Node,
        name: &str,
        qualifier: Option<String>,
        on_self: bool,
        method: bool,
    ) {
        if name.is_empty() || self.facts.calls.len() >= MAX_CALLS {
            return;
        }
        self.facts.calls.push(Call {
            from: self.current,
            name: name.to_string(),
            qualifier: qualifier.filter(|q| !q.is_empty()),
            on_self,
            method,
            line: line_of(node),
        });
    }

    /// Walks the children of `node`, with the function `current` and the type `owner`.
    fn walk_body(&mut self, node: Node, current: Option<u32>, owner: Option<String>) {
        let (saved_current, saved_owner) = (self.current, self.owner.clone());
        self.current = current;
        self.owner = owner;
        self.walk(node, false);
        self.current = saved_current;
        self.owner = saved_owner;
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        let mut command = false;
        for child in children {
            if self.lang == Lang::Rust && child.kind() == "attribute_item" {
                command |= self.text(child).contains("command");
                continue;
            }
            if self.lang == Lang::Rust && child.kind() == "function_item" {
                self.rust_function(child, command);
            } else {
                self.walk(child, false);
            }
            command = false;
        }
    }

    /// Walks `node` itself (`root` for the file's top node).
    fn walk(&mut self, node: Node, root: bool) {
        let handled = match self.lang {
            Lang::Rust => self.rust(node),
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => self.script(node),
            Lang::Python => self.python(node),
            Lang::Go => self.go(node),
            Lang::Markdown => false,
        };
        if !handled || root {
            self.walk_children(node);
        }
    }

    // ---- Rust ----

    fn rust(&mut self, node: Node) -> bool {
        match node.kind() {
            "function_item" => {
                self.rust_function(node, false);
                true
            }
            "function_signature_item" => {
                if let Some(name) = self.field_text(node, "name") {
                    let kind = if self.owner.is_some() {
                        SymKind::Method
                    } else {
                        SymKind::Function
                    };
                    self.add_symbol(node, name, kind, false);
                }
                true
            }
            "struct_item" | "enum_item" | "union_item" | "type_item" => {
                if let Some(name) = self.field_text(node, "name") {
                    self.add_symbol(node, name, SymKind::Type, false);
                }
                true
            }
            "trait_item" => {
                let name = self.field_text(node, "name");
                if let Some(name) = &name {
                    self.add_symbol(node, name.clone(), SymKind::Type, false);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk_body(body, None, name);
                }
                true
            }
            "impl_item" => {
                let ty = node
                    .child_by_field_name("type")
                    .map(|t| self.type_name(t))
                    .filter(|t| !t.is_empty());
                if let (Some(ty), Some(tr)) = (&ty, node.child_by_field_name("trait")) {
                    let name = self.type_name(tr);
                    if !name.is_empty() {
                        self.facts.bases.push(Base {
                            of: ty.clone(),
                            name,
                            implements: true,
                            line: line_of(node),
                        });
                    }
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk_body(body, None, ty);
                }
                true
            }
            "mod_item" => {
                let name = self.field_text(node, "name").unwrap_or_default();
                match node.child_by_field_name("body") {
                    // An inline module's items are the file's.
                    Some(body) => self.walk_body(body, self.current, None),
                    None if !name.is_empty() => self.facts.imports.push(Import {
                        spec: format!("self::{name}"),
                        items: Vec::new(),
                        line: line_of(node),
                        module: true,
                    }),
                    None => {}
                }
                true
            }
            "use_declaration" => {
                if let Some(arg) = node.child_by_field_name("argument") {
                    let mut paths = Vec::new();
                    self.use_paths(arg, &[], &mut paths);
                    for (segs, alias) in paths {
                        let Some(last) = segs.last().cloned() else {
                            continue;
                        };
                        let local = alias.unwrap_or_else(|| last.clone());
                        let (spec, local) = if last == "self" {
                            let module = segs[..segs.len() - 1].to_vec();
                            let local = if local == "self" {
                                module.last().cloned().unwrap_or_default()
                            } else {
                                local
                            };
                            (module.join("::"), local)
                        } else {
                            (segs.join("::"), local)
                        };
                        let name = if last == "*" { "*" } else { "" };
                        self.facts.imports.push(Import {
                            spec,
                            items: vec![ImportItem {
                                name: name.to_string(),
                                local,
                            }],
                            line: line_of(node),
                            module: false,
                        });
                    }
                }
                true
            }
            "call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            let name = self.text(f).to_string();
                            self.add_call(node, &name, None, false);
                        }
                        "scoped_identifier" => {
                            let name = self.field_text(f, "name").unwrap_or_default();
                            let qualifier = f.child_by_field_name("path").map(|p| {
                                let text = self.text(p);
                                let text = text.split('<').next().unwrap_or(text);
                                text.rsplit("::").next().unwrap_or(text).to_string()
                            });
                            self.add_call(node, &name, qualifier, false);
                        }
                        "field_expression" => {
                            let name = self.field_text(f, "field").unwrap_or_default();
                            let value = f.child_by_field_name("value");
                            let on_self = value.is_some_and(|v| v.kind() == "self");
                            self.add_method_call(node, &name, None, on_self);
                        }
                        "generic_function" => {
                            if let Some(inner) = f.child_by_field_name("function") {
                                let text = self.text(inner);
                                let name = text.rsplit("::").next().unwrap_or(text).to_string();
                                self.add_call(node, &name, None, false);
                            }
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn rust_function(&mut self, node: Node, command: bool) {
        let Some(name) = self.field_text(node, "name") else {
            return;
        };
        let kind = if self.owner.is_some() {
            SymKind::Method
        } else {
            SymKind::Function
        };
        let index = self.add_symbol(node, name, kind, command);
        if let Some(body) = node.child_by_field_name("body") {
            let owner = self.owner.clone();
            self.walk_body(body, Some(index), owner);
        }
    }

    /// The name of a type: `Foo` of `Foo<T>`, `a::Foo`, `&Foo`.
    fn type_name(&self, node: Node) -> String {
        match node.kind() {
            "generic_type" => node
                .child_by_field_name("type")
                .map(|t| self.type_name(t))
                .unwrap_or_default(),
            "scoped_type_identifier" | "scoped_identifier" => self
                .field_text(node, "name")
                .unwrap_or_else(|| self.text(node).to_string()),
            "reference_type" | "pointer_type" => node
                .child_by_field_name("type")
                .map(|t| self.type_name(t))
                .unwrap_or_default(),
            _ => {
                let text = self.text(node);
                let text = text.split('<').next().unwrap_or(text);
                text.rsplit("::").next().unwrap_or(text).trim().to_string()
            }
        }
    }

    /// The paths of a `use` tree, each with its alias.
    fn use_paths(
        &self,
        node: Node,
        prefix: &[String],
        out: &mut Vec<(Vec<String>, Option<String>)>,
    ) {
        let segs_of = |n: Node| -> Vec<String> {
            self.text(n)
                .split("::")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let join = |segs: Vec<String>| -> Vec<String> {
            let mut all = prefix.to_vec();
            all.extend(segs);
            all
        };
        match node.kind() {
            "scoped_use_list" => {
                let base = node
                    .child_by_field_name("path")
                    .map(|p| join(segs_of(p)))
                    .unwrap_or_else(|| prefix.to_vec());
                if let Some(list) = node.child_by_field_name("list") {
                    self.use_paths(list, &base, out);
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.use_paths(child, prefix, out);
                }
            }
            "use_as_clause" => {
                let path = node
                    .child_by_field_name("path")
                    .map(|p| join(segs_of(p)))
                    .unwrap_or_default();
                out.push((path, self.field_text(node, "alias")));
            }
            "use_wildcard" => {
                let mut segs = join(segs_of(node));
                segs.retain(|s| s != "*");
                segs.push("*".to_string());
                out.push((segs, None));
            }
            _ => out.push((join(segs_of(node)), None)),
        }
    }

    // ---- TypeScript, TSX, JavaScript ----

    fn script(&mut self, node: Node) -> bool {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    let index = self.add_symbol(node, name, SymKind::Function, false);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_body(body, Some(index), None);
                    }
                }
                true
            }
            "class_declaration" | "abstract_class_declaration" | "class" => {
                let Some(name) = self.field_text(node, "name") else {
                    return false;
                };
                self.add_symbol(node, name.clone(), SymKind::Type, false);
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "class_heritage" {
                        self.heritage(child, &name);
                    }
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk_body(body, None, Some(name));
                }
                true
            }
            "interface_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    self.add_symbol(node, name.clone(), SymKind::Type, false);
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        if child.kind() == "extends_type_clause" {
                            let mut c2 = child.walk();
                            for t in child.named_children(&mut c2) {
                                let base = self.script_type_name(t);
                                self.push_base(&name, base, false, child);
                            }
                        }
                    }
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_body(body, None, Some(name));
                    }
                }
                true
            }
            "type_alias_declaration" | "enum_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    self.add_symbol(node, name, SymKind::Type, false);
                }
                true
            }
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                if let Some(name) = self.field_text(node, "name") {
                    let index = self.add_symbol(node, name, SymKind::Method, false);
                    if let Some(body) = node.child_by_field_name("body") {
                        let owner = self.owner.clone();
                        self.walk_body(body, Some(index), owner);
                    }
                }
                true
            }
            "variable_declarator" => {
                let value = node.child_by_field_name("value");
                let is_fn = value.is_some_and(|v| {
                    matches!(
                        v.kind(),
                        "arrow_function" | "function_expression" | "function"
                    )
                });
                let name = node.child_by_field_name("name");
                if is_fn && self.current.is_none() && name.is_some_and(|n| n.kind() == "identifier")
                {
                    let name = self.text(name.unwrap()).to_string();
                    let index = self.add_symbol(node, name, SymKind::Function, false);
                    self.walk_body(value.unwrap(), Some(index), None);
                    return true;
                }
                false
            }
            "import_statement" => {
                let Some(source) = node.child_by_field_name("source") else {
                    return true;
                };
                let spec = unquote(self.text(source));
                let mut items = Vec::new();
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "import_clause" {
                        self.import_clause(child, &mut items);
                    }
                }
                self.facts.imports.push(Import {
                    spec,
                    items,
                    line: line_of(node),
                    module: false,
                });
                true
            }
            "export_statement" => {
                if let Some(source) = node.child_by_field_name("source") {
                    self.facts.imports.push(Import {
                        spec: unquote(self.text(source)),
                        items: Vec::new(),
                        line: line_of(node),
                        module: false,
                    });
                    return true;
                }
                false
            }
            "call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            let name = self.text(f).to_string();
                            if name == "require" {
                                self.dynamic_import(node);
                            } else {
                                self.add_call(node, &name, None, false);
                            }
                        }
                        "import" => self.dynamic_import(node),
                        "member_expression" => {
                            let name = self.field_text(f, "property").unwrap_or_default();
                            let object = f.child_by_field_name("object");
                            let on_self = object.is_some_and(|o| o.kind() == "this");
                            let qualifier = object
                                .filter(|o| o.kind() == "identifier")
                                .map(|o| self.text(o).to_string());
                            self.add_method_call(node, &name, qualifier, on_self);
                        }
                        _ => {}
                    }
                }
                false
            }
            "new_expression" => {
                if let Some(c) = node.child_by_field_name("constructor") {
                    if c.kind() == "identifier" {
                        let name = self.text(c).to_string();
                        self.add_call(node, &name, None, false);
                    }
                }
                false
            }
            "jsx_opening_element" | "jsx_self_closing_element" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n).to_string();
                    if n.kind() == "identifier"
                        && name.starts_with(|c: char| c.is_ascii_uppercase())
                    {
                        self.add_call(node, &name, None, false);
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn script_type_name(&self, node: Node) -> String {
        let text = self.text(node);
        let text = text.split('<').next().unwrap_or(text);
        text.rsplit('.').next().unwrap_or(text).trim().to_string()
    }

    fn push_base(&mut self, of: &str, name: String, implements: bool, at: Node) {
        if name.is_empty() || !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
            return;
        }
        self.facts.bases.push(Base {
            of: of.to_string(),
            name,
            implements,
            line: line_of(at),
        });
    }

    fn heritage(&mut self, node: Node, class: &str) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for child in children {
            match child.kind() {
                "extends_clause" => {
                    if let Some(v) = child.child_by_field_name("value") {
                        let base = self.script_type_name(v);
                        self.push_base(class, base, false, child);
                    }
                }
                "implements_clause" => {
                    let mut c2 = child.walk();
                    for t in child.named_children(&mut c2) {
                        let base = self.script_type_name(t);
                        self.push_base(class, base, true, child);
                    }
                }
                // JavaScript: `extends <expression>` directly.
                _ => {
                    let base = self.script_type_name(child);
                    self.push_base(class, base, false, child);
                }
            }
        }
    }

    fn import_clause(&self, node: Node, items: &mut Vec<ImportItem>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" => items.push(ImportItem {
                    name: "default".into(),
                    local: self.text(child).to_string(),
                }),
                "namespace_import" => {
                    let mut c2 = child.walk();
                    let first = child.named_children(&mut c2).next();
                    if let Some(id) = first {
                        items.push(ImportItem {
                            name: String::new(),
                            local: self.text(id).to_string(),
                        });
                    }
                }
                "named_imports" => {
                    let mut c2 = child.walk();
                    for spec in child.named_children(&mut c2) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let name = self
                            .field_text(spec, "name")
                            .map(|n| unquote(&n))
                            .unwrap_or_default();
                        let local = self
                            .field_text(spec, "alias")
                            .unwrap_or_else(|| name.clone());
                        items.push(ImportItem { name, local });
                    }
                }
                _ => {}
            }
        }
    }

    /// `require("x")` and `import("x")`.
    fn dynamic_import(&mut self, node: Node) {
        let Some(args) = node.child_by_field_name("arguments") else {
            return;
        };
        let mut cursor = args.walk();
        let first = args.named_children(&mut cursor).next();
        if let Some(s) = first.filter(|s| s.kind() == "string") {
            self.facts.imports.push(Import {
                spec: unquote(self.text(s)),
                items: Vec::new(),
                line: line_of(node),
                module: false,
            });
        }
    }

    // ---- Python ----

    fn python(&mut self, node: Node) -> bool {
        match node.kind() {
            "function_definition" => {
                if let Some(name) = self.field_text(node, "name") {
                    let kind = if self.owner.is_some() && self.current.is_none() {
                        SymKind::Method
                    } else {
                        SymKind::Function
                    };
                    let index = self.add_symbol(node, name, kind, false);
                    if let Some(body) = node.child_by_field_name("body") {
                        let owner = self.owner.clone();
                        self.walk_body(body, Some(index), owner);
                    }
                }
                true
            }
            "class_definition" => {
                let Some(name) = self.field_text(node, "name") else {
                    return false;
                };
                self.add_symbol(node, name.clone(), SymKind::Type, false);
                if let Some(args) = node.child_by_field_name("superclasses") {
                    let mut cursor = args.walk();
                    let bases: Vec<Node> = args.named_children(&mut cursor).collect();
                    for b in bases {
                        if matches!(b.kind(), "identifier" | "attribute") {
                            let base = self.script_type_name(b);
                            self.push_base(&name, base, false, b);
                        }
                    }
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk_body(body, None, Some(name));
                }
                true
            }
            "import_statement" => {
                let mut cursor = node.walk();
                for child in node.children_by_field_name("name", &mut cursor) {
                    let (module, alias) = match child.kind() {
                        "aliased_import" => (
                            self.field_text(child, "name").unwrap_or_default(),
                            self.field_text(child, "alias"),
                        ),
                        _ => (self.text(child).to_string(), None),
                    };
                    let local = alias.unwrap_or_else(|| {
                        module.split('.').next().unwrap_or_default().to_string()
                    });
                    self.facts.imports.push(Import {
                        spec: module,
                        items: vec![ImportItem {
                            name: String::new(),
                            local,
                        }],
                        line: line_of(node),
                        module: false,
                    });
                }
                true
            }
            "import_from_statement" => {
                let module = self.field_text(node, "module_name").unwrap_or_default();
                let mut items = Vec::new();
                let mut cursor = node.walk();
                for child in node.children_by_field_name("name", &mut cursor) {
                    let (name, alias) = match child.kind() {
                        "aliased_import" => (
                            self.field_text(child, "name").unwrap_or_default(),
                            self.field_text(child, "alias"),
                        ),
                        _ => (self.text(child).to_string(), None),
                    };
                    let local = alias.unwrap_or_else(|| name.clone());
                    items.push(ImportItem { name, local });
                }
                self.facts.imports.push(Import {
                    spec: module,
                    items,
                    line: line_of(node),
                    module: false,
                });
                true
            }
            "call" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            let name = self.text(f).to_string();
                            self.add_call(node, &name, None, false);
                        }
                        "attribute" => {
                            let name = self.field_text(f, "attribute").unwrap_or_default();
                            let object = f.child_by_field_name("object");
                            let obj = object.map(|o| self.text(o)).unwrap_or_default();
                            let on_self = obj == "self" || obj == "cls";
                            let qualifier = object
                                .filter(|o| o.kind() == "identifier" && !on_self)
                                .map(|o| self.text(o).to_string());
                            self.add_method_call(node, &name, qualifier, on_self);
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    // ---- Go ----

    fn go(&mut self, node: Node) -> bool {
        match node.kind() {
            "function_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    let index = self.add_symbol(node, name, SymKind::Function, false);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_body(body, Some(index), None);
                    }
                }
                true
            }
            "method_declaration" => {
                let owner = node
                    .child_by_field_name("receiver")
                    .and_then(|r| {
                        let mut cursor = r.walk();
                        let p = r.named_children(&mut cursor).next();
                        p
                    })
                    .and_then(|p| p.child_by_field_name("type"))
                    .map(|t| {
                        self.text(t)
                            .trim_start_matches('*')
                            .split('[')
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    });
                if let Some(name) = self.field_text(node, "name") {
                    let saved = std::mem::replace(&mut self.owner, owner.clone());
                    let index = self.add_symbol(node, name, SymKind::Method, false);
                    self.owner = saved;
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_body(body, Some(index), owner);
                    }
                }
                true
            }
            "type_spec" | "type_alias" => {
                if let Some(name) = self.field_text(node, "name") {
                    self.add_symbol(node, name, SymKind::Type, false);
                }
                true
            }
            "import_spec" => {
                let spec = self
                    .field_text(node, "path")
                    .map(|p| unquote(&p))
                    .unwrap_or_default();
                let local = self
                    .field_text(node, "name")
                    .unwrap_or_else(|| spec.rsplit('/').next().unwrap_or_default().to_string());
                self.facts.imports.push(Import {
                    spec,
                    items: vec![ImportItem {
                        name: String::new(),
                        local,
                    }],
                    line: line_of(node),
                    module: false,
                });
                true
            }
            "call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            let name = self.text(f).to_string();
                            self.add_call(node, &name, None, false);
                        }
                        "selector_expression" => {
                            let name = self.field_text(f, "field").unwrap_or_default();
                            let qualifier = f
                                .child_by_field_name("operand")
                                .filter(|o| o.kind() == "identifier")
                                .map(|o| self.text(o).to_string());
                            self.add_method_call(node, &name, qualifier, false);
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }
}

/// A string literal's text without its quotes.
fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

/// The headings of a Markdown text, each under the nearest heading of a lower level, and the
/// code paths each section names. Fenced code is skipped.
fn markdown(text: &str) -> Facts {
    let mut facts = Facts::default();
    let mut fenced = false;
    // (level, symbol index) of the open headings.
    let mut open: Vec<(usize, u32)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced && line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count();
            let rest = &line[level..];
            if (1..=6).contains(&level) && rest.starts_with(' ') {
                let title = rest.trim().trim_end_matches('#').trim().to_string();
                if !title.is_empty() {
                    while open.last().is_some_and(|(l, _)| *l >= level) {
                        open.pop();
                    }
                    let parent = open.last().map(|(_, s)| *s);
                    let mut signature = format!("{} {title}", "#".repeat(level));
                    if let Some((at, _)) = signature.char_indices().nth(MAX_SIGNATURE) {
                        signature.truncate(at);
                    }
                    facts.symbols.push(Symbol {
                        name: title,
                        kind: SymKind::Heading,
                        line: i as u32 + 1,
                        owner: None,
                        parent,
                        signature,
                        command: false,
                    });
                    open.push((level, (facts.symbols.len() - 1) as u32));
                    continue;
                }
            }
        }
        // Code paths named in the section (in code spans too).
        if let Some((_, section)) = open.last() {
            for path in code_paths(line) {
                let mention = Mention {
                    from: *section,
                    path,
                };
                if !facts.mentions.contains(&mention) {
                    facts.mentions.push(mention);
                }
            }
        }
    }
    facts
}

/// The tokens of `line` that look like a path to a source file: `a/b.rs`, `src/x.tsx`.
fn code_paths(line: &str) -> Vec<String> {
    const EXTS: &[&str] = &[
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".mjs", ".cjs",
    ];
    line.split(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')))
        .map(|t| t.trim_matches(|c| c == '.' || c == '/'))
        .filter(|t| t.contains('/') && EXTS.iter().any(|e| t.ends_with(e)))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(f: &Facts) -> Vec<(String, SymKind, Option<String>)> {
        f.symbols
            .iter()
            .map(|s| (s.name.clone(), s.kind, s.owner.clone()))
            .collect()
    }

    fn calls(f: &Facts) -> Vec<String> {
        f.calls
            .iter()
            .map(|c| {
                let from = c
                    .from
                    .map(|i| f.symbols[i as usize].name.clone())
                    .unwrap_or_default();
                let q = c
                    .qualifier
                    .clone()
                    .map(|q| format!("{q}::"))
                    .unwrap_or_default();
                let s = if c.on_self { "self." } else { "" };
                format!("{from}->{s}{q}{}", c.name)
            })
            .collect()
    }

    #[test]
    fn rust_symbols_imports_calls_and_impls() {
        let src = r#"
use crate::store::{Store, model::Dot as D};
use std::sync::Arc;
mod helpers;
pub struct Runner { store: Store }
pub trait Engine { fn start(&self); }
impl Engine for Runner {
    fn start(&self) { self.prepare(); helpers::go(); Store::open(); }
}
impl Runner {
    fn prepare(&self) { free(); }
}
#[tauri::command]
pub async fn run_now() -> u32 { free() }
fn free() -> u32 { 1 }
"#;
        let f = extract(Lang::Rust, src).unwrap();
        assert_eq!(
            names(&f),
            vec![
                ("Runner".into(), SymKind::Type, None),
                ("Engine".into(), SymKind::Type, None),
                ("start".into(), SymKind::Method, Some("Engine".into())),
                ("start".into(), SymKind::Method, Some("Runner".into())),
                ("prepare".into(), SymKind::Method, Some("Runner".into())),
                ("run_now".into(), SymKind::Function, None),
                ("free".into(), SymKind::Function, None),
            ]
        );
        assert!(f.symbols[5].command);
        assert!(!f.symbols[6].command);
        let specs: Vec<(&str, &str)> = f
            .imports
            .iter()
            .map(|i| {
                (
                    i.spec.as_str(),
                    i.items.first().map(|x| x.local.as_str()).unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            specs,
            vec![
                ("crate::store::Store", "Store"),
                ("crate::store::model::Dot", "D"),
                ("std::sync::Arc", "Arc"),
                ("self::helpers", ""),
            ]
        );
        assert!(f.imports[3].module);
        assert_eq!(
            calls(&f),
            vec![
                "start->self.prepare",
                "start->helpers::go",
                "start->Store::open",
                "prepare->free",
                "run_now->free",
            ]
        );
        assert_eq!(
            f.bases,
            vec![Base {
                of: "Runner".into(),
                name: "Engine".into(),
                implements: true,
                line: 7
            }]
        );
        assert_eq!(f.symbols[0].signature, "pub struct Runner");
    }

    #[test]
    fn typescript_classes_imports_and_jsx() {
        let src = r#"
import React, { useState as useS } from "react";
import * as api from "./api";
import { Panel } from "@/components/Panel";
export { x } from "./x";
interface Shape extends Base { area(): number }
export class Square extends Figure implements Shape {
  area() { return this.side(); }
  side() { return api.size(); }
}
export const View = () => { const [a] = useS(0); return <Panel />; };
export function main() { new Square(); }
const lazy = import("./lazy");
"#;
        let f = extract(Lang::Tsx, src).unwrap();
        assert_eq!(
            names(&f),
            vec![
                ("Shape".into(), SymKind::Type, None),
                ("area".into(), SymKind::Method, Some("Shape".into())),
                ("Square".into(), SymKind::Type, None),
                ("area".into(), SymKind::Method, Some("Square".into())),
                ("side".into(), SymKind::Method, Some("Square".into())),
                ("View".into(), SymKind::Function, None),
                ("main".into(), SymKind::Function, None),
            ]
        );
        let specs: Vec<&str> = f.imports.iter().map(|i| i.spec.as_str()).collect();
        assert_eq!(
            specs,
            vec!["react", "./api", "@/components/Panel", "./x", "./lazy"]
        );
        assert_eq!(
            f.imports[0].items,
            vec![
                ImportItem {
                    name: "default".into(),
                    local: "React".into()
                },
                ImportItem {
                    name: "useState".into(),
                    local: "useS".into()
                }
            ]
        );
        assert_eq!(
            calls(&f),
            vec![
                "area->self.side",
                "side->api::size",
                "View->useS",
                "View->Panel",
                "main->Square",
            ]
        );
        let bases: Vec<(&str, &str, bool)> = f
            .bases
            .iter()
            .map(|b| (b.of.as_str(), b.name.as_str(), b.implements))
            .collect();
        assert_eq!(
            bases,
            vec![
                ("Shape", "Base", false),
                ("Square", "Figure", false),
                ("Square", "Shape", true)
            ]
        );
    }

    #[test]
    fn javascript_require_and_classes() {
        let src = "const fs = require('fs');\nclass A extends B { go() { helper(); } }\nfunction helper() {}\n";
        let f = extract(Lang::JavaScript, src).unwrap();
        assert_eq!(f.imports[0].spec, "fs");
        assert_eq!(
            names(&f),
            vec![
                ("A".into(), SymKind::Type, None),
                ("go".into(), SymKind::Method, Some("A".into())),
                ("helper".into(), SymKind::Function, None),
            ]
        );
        assert_eq!(f.bases[0].name, "B");
        assert_eq!(calls(&f), vec!["go->helper"]);
    }

    #[test]
    fn python_classes_imports_and_calls() {
        let src = r#"
import os.path as p
from .models import User, Group as G
class Admin(User):
    def promote(self):
        self.save()
        p.join("a")
def main():
    Admin().promote()
"#;
        let f = extract(Lang::Python, src).unwrap();
        assert_eq!(
            names(&f),
            vec![
                ("Admin".into(), SymKind::Type, None),
                ("promote".into(), SymKind::Method, Some("Admin".into())),
                ("main".into(), SymKind::Function, None),
            ]
        );
        assert_eq!(f.imports[0].spec, "os.path");
        assert_eq!(f.imports[0].items[0].local, "p");
        assert_eq!(f.imports[1].spec, ".models");
        assert_eq!(f.imports[1].items[1].name, "Group");
        assert_eq!(f.imports[1].items[1].local, "G");
        assert_eq!(f.bases[0].name, "User");
        assert_eq!(
            calls(&f),
            vec![
                "promote->self.save",
                "promote->p::join",
                "main->promote",
                "main->Admin"
            ]
        );
    }

    #[test]
    fn go_types_methods_and_imports() {
        let src = r#"package main
import (
    "fmt"
    st "example.com/app/store"
)
type Server struct{}
func (s *Server) Start() { fmt.Println("x"); helper() }
func helper() {}
func main() { st.Open() }
"#;
        let f = extract(Lang::Go, src).unwrap();
        assert_eq!(
            names(&f),
            vec![
                ("Server".into(), SymKind::Type, None),
                ("Start".into(), SymKind::Method, Some("Server".into())),
                ("helper".into(), SymKind::Function, None),
                ("main".into(), SymKind::Function, None),
            ]
        );
        assert_eq!(f.imports[1].spec, "example.com/app/store");
        assert_eq!(f.imports[1].items[0].local, "st");
        assert_eq!(
            calls(&f),
            vec!["Start->fmt::Println", "Start->helper", "main->st::Open"]
        );
    }

    #[test]
    fn markdown_headings_and_mentions() {
        let src = "# Title\n\nSee `crates/a/src/lib.rs`.\n\n## Part\n\n```\n# not a heading\n```\nand ui/src/x.tsx\n";
        let f = extract(Lang::Markdown, src).unwrap();
        assert_eq!(
            names(&f),
            vec![
                ("Title".into(), SymKind::Heading, None),
                ("Part".into(), SymKind::Heading, None),
            ]
        );
        assert_eq!(f.symbols[1].parent, Some(0));
        assert_eq!(
            f.mentions,
            vec![
                Mention {
                    from: 0,
                    path: "crates/a/src/lib.rs".into()
                },
                Mention {
                    from: 1,
                    path: "ui/src/x.tsx".into()
                }
            ]
        );
    }

    #[test]
    fn languages_by_extension() {
        assert_eq!(Lang::of("a/b.RS"), Some(Lang::Rust));
        assert_eq!(Lang::of("x.tsx"), Some(Lang::Tsx));
        assert_eq!(Lang::of("x.mjs"), Some(Lang::JavaScript));
        assert_eq!(Lang::of("README"), None);
        assert_eq!(Lang::of("x.json"), None);
    }
}
