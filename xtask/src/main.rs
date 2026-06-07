use similar::ChangeTag;
use similar::TextDiff;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const SOURCE_FILE: &str = "src/app_config.rs";
const DOCS_FILE: &str = "CONFIG.md";

#[derive(Debug, Clone, PartialEq, Eq)]
enum EvalResult {
    Literal(String),
    Other,
}

struct StructDoc {
    name: String,
    doc: String,
    fields: Vec<FieldDoc>,
    is_hide_defaults: bool,
}

struct FieldDoc {
    name: String,
    type_str: String,
    doc: String,
    default: String,
}

struct EnumDoc {
    name: String,
    doc: String,
    variants: Vec<VariantDoc>,
    is_untagged: bool,
}

struct VariantDoc {
    name: String,
    rust_name: String,
    doc: String,
    payload: String,
    is_default: bool,
}

enum Section {
    Struct(StructDoc),
    Enum(EnumDoc),
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let check_mode = args.iter().any(|a| a == "--check");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask should be in workspace subdirectory")
        .to_path_buf();

    let md = generate_config_docs(&manifest_dir);
    let docs_path = manifest_dir.join(DOCS_FILE);

    if check_mode {
        let existing = fs::read_to_string(&docs_path).unwrap_or_default();
        if md == existing {
            println!("{} is up-to-date.", DOCS_FILE);
            return;
        }

        show_diff(&existing, &md);
        std::process::exit(1);
    } else {
        fs::write(&docs_path, &md).unwrap_or_else(|_| panic!("failed to write {}", DOCS_FILE));
        println!("{} successfully generated.", DOCS_FILE);
    }
}

fn show_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);

    for group in diff.grouped_ops(3) {
        let line_no = group
            .first()
            .map(|op| op.old_range().start + 1)
            .unwrap_or(1);
        eprintln!("Diff in {}:{}:", DOCS_FILE, line_no);

        for op in &group {
            for change in diff.iter_changes(op) {
                let val = change.value().strip_suffix('\n').unwrap_or(change.value());
                match change.tag() {
                    ChangeTag::Equal => eprintln!(" {}", val),
                    ChangeTag::Delete => eprintln!("\x1b[31m-{}\x1b[0m", val),
                    ChangeTag::Insert => eprintln!("\x1b[32m+{}\x1b[0m", val),
                }
            }
        }
    }
}

fn generate_config_docs(manifest_dir: &Path) -> String {
    let source_path = manifest_dir.join(SOURCE_FILE);
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("failed to read {}", SOURCE_FILE));
    let syntax =
        syn::parse_file(&source).unwrap_or_else(|_| panic!("failed to parse {}", SOURCE_FILE));

    let mut default_fns: HashMap<String, EvalResult> = HashMap::new();
    for item in &syntax.items {
        if let syn::Item::Fn(f) = item {
            if f.sig.ident.to_string().starts_with("default_") {
                let func_name = f.sig.ident.to_string();
                let body = f.block.stmts.first().and_then(|s| match s {
                    syn::Stmt::Expr(e, _) => Some(e),
                    _ => None,
                });
                let empty_fns = HashMap::new();
                let value = body
                    .and_then(|e| eval_literal_expr(e, &empty_fns))
                    .unwrap_or(EvalResult::Other);
                if value == EvalResult::Other {
                    eprintln!(
                        "warning: could not evaluate default value for `{}`",
                        func_name
                    );
                }
                default_fns.insert(func_name, value);
            }
        }
    }

    let struct_defaults = collect_struct_defaults(&syntax, &default_fns);

    let mut sections: Vec<Section> = Vec::new();
    for item in &syntax.items {
        match item {
            syn::Item::Struct(s)
                if has_serde_derive(&s.attrs) && !has_config_doc_tag(&s.attrs, "skip") =>
            {
                let struct_name = s.ident.to_string();
                let doc = extract_doc(&s.attrs);
                let mut fields = Vec::new();
                if let syn::Fields::Named(ref named) = s.fields {
                    for field in &named.named {
                        let rust_ident = field
                            .ident
                            .as_ref()
                            .map(|i| i.to_string())
                            .unwrap_or_default();
                        let display_ident =
                            get_serde_rename(&field.attrs).unwrap_or_else(|| rust_ident.clone());
                        let ty = &field.ty;
                        let fd = extract_doc(&field.attrs);
                        let default_val = extract_default(
                            &field.attrs,
                            ty,
                            &default_fns,
                            &struct_defaults,
                            &struct_name,
                            &rust_ident,
                        );
                        let raw_type = pretty_type(ty);
                        let (field_name, type_str) = if let Some(val_type) = is_string_keyed_map(ty)
                        {
                            (format!("{}.\"<name>\"", display_ident), val_type)
                        } else {
                            (display_ident, raw_type)
                        };
                        fields.push(FieldDoc {
                            name: field_name,
                            type_str,
                            doc: fd,
                            default: default_val,
                        });
                    }
                }
                sections.push(Section::Struct(StructDoc {
                    name: struct_name,
                    doc,
                    fields,
                    is_hide_defaults: has_config_doc_tag(&s.attrs, "hide_defaults"),
                }));
            }
            syn::Item::Enum(e)
                if has_serde_derive(&e.attrs) && !has_config_doc_tag(&e.attrs, "skip") =>
            {
                let doc = extract_doc(&e.attrs);
                let mut variants = Vec::new();
                for v in &e.variants {
                    let vd = extract_doc(&v.attrs);
                    let rust_name = v.ident.to_string();
                    let variant_name =
                        get_serde_rename(&v.attrs).unwrap_or_else(|| rust_name.clone());
                    let payload = match &v.fields {
                        syn::Fields::Named(f) => {
                            let parts: Vec<String> = f
                                .named
                                .iter()
                                .map(|fld| {
                                    let name = fld
                                        .ident
                                        .as_ref()
                                        .map(|i| i.to_string())
                                        .unwrap_or_default();
                                    let ft = &fld.ty;
                                    format!("{}: {}", name, pretty_type(ft))
                                })
                                .collect();
                            parts.join(", ")
                        }
                        syn::Fields::Unnamed(f) => {
                            let parts: Vec<String> =
                                f.unnamed.iter().map(|fld| pretty_type(&fld.ty)).collect();
                            parts.join(", ")
                        }
                        syn::Fields::Unit => String::new(),
                    };
                    variants.push(VariantDoc {
                        name: variant_name,
                        rust_name,
                        doc: vd,
                        payload,
                        is_default: v.attrs.iter().any(|a| a.path().is_ident("default")),
                    });
                }
                let is_untagged = has_serde_untagged(&e.attrs);
                sections.push(Section::Enum(EnumDoc {
                    name: e.ident.to_string(),
                    doc,
                    variants,
                    is_untagged,
                }));
            }
            _ => {}
        }
    }

    let enum_map: EnumMap = build_enum_map(&sections);
    let enum_defaults = build_enum_defaults(&sections);

    for section in &mut sections {
        if let Section::Struct(st) = section {
            for field in &mut st.fields {
                field.default =
                    tomlize_default(&field.default, &enum_map, &enum_defaults, &struct_defaults);
            }
        }
    }

    let section_map = build_struct_section_map(&syntax);
    render(&sections, &section_map)
}

fn has_config_doc_tag(attrs: &[syn::Attribute], tag: &str) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("config_doc") {
            if let Ok(meta_list) = attr.meta.require_list() {
                return meta_list.tokens.to_string().contains(tag);
            }
        }
        if attr.path().is_ident("cfg_attr") {
            if let Ok(meta_list) = attr.meta.require_list() {
                let tokens = meta_list.tokens.to_string();
                if tokens.contains("config_doc") && tokens.contains(tag) {
                    return true;
                }
            }
        }
        false
    })
}

fn parse_serde_metas(attr: &syn::Attribute) -> Vec<syn::Meta> {
    attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map(|p| p.into_iter().collect())
        .unwrap_or_default()
}

fn extract_default(
    attrs: &[syn::Attribute],
    ty: &syn::Type,
    default_fns: &HashMap<String, EvalResult>,
    struct_defaults: &HashMap<String, BTreeMap<String, String>>,
    struct_name: &str,
    field_name: &str,
) -> String {
    let mut serde_default_fn = None;
    let mut has_serde_default = false;

    for attr in attrs {
        if attr.path().is_ident("serde") {
            let metas = parse_serde_metas(attr);
            for meta in &metas {
                if meta.path().is_ident("default") {
                    match meta {
                        syn::Meta::NameValue(nv) => {
                            if let syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(s),
                                ..
                            }) = &nv.value
                            {
                                serde_default_fn = Some(s.value());
                            }
                        }
                        syn::Meta::Path(_) => {
                            has_serde_default = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Some(fn_name) = serde_default_fn {
        if let Some(result) = default_fns.get(&fn_name) {
            return match result {
                EvalResult::Literal(v) => format!("`{}`", v),
                EvalResult::Other => format!("`{}()`", fn_name),
            };
        }
        return format!("`{}()`", fn_name);
    }

    if has_serde_default {
        if let Some(field_defaults) = struct_defaults.get(struct_name) {
            if let Some(value) = field_defaults.get(field_name) {
                if value == "None" {
                    return "`—`".to_string();
                }
                return format!("`{}`", value);
            }
        }
        return match ty {
            syn::Type::Path(tp) => {
                let ident = tp
                    .path
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default();
                match ident.as_str() {
                    "Option" | "HashMap" | "BTreeMap" => "`—`".to_string(),
                    "Vec" => "`[]`".to_string(),
                    "bool" => "`false`".to_string(),
                    "f32" | "f64" => "`0.0`".to_string(),
                    "u32" | "usize" | "i32" | "u64" => "`0`".to_string(),
                    "String" => "empty string".to_string(),
                    _ => "Default".to_string(),
                }
            }
            syn::Type::Array(_) => "`[]`".to_string(),
            _ => "Default".to_string(),
        };
    }

    if let syn::Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Option" {
                return "`—`".to_string();
            }
        }
    }

    "Required".to_string()
}

/// HashMap<enum_name, HashMap<variant_rust_name, variant_config_name>>
type EnumMap = HashMap<String, HashMap<String, String>>;

fn build_enum_map(sections: &[Section]) -> EnumMap {
    let mut map: EnumMap = HashMap::new();
    for section in sections {
        if let Section::Enum(e) = section {
            let variants: HashMap<String, String> = e
                .variants
                .iter()
                .map(|v| (v.rust_name.clone(), v.name.clone()))
                .collect();
            map.insert(e.name.clone(), variants);
        }
    }
    map
}

fn tomlize_inner(value: &str, enum_map: &HashMap<String, HashMap<String, String>>) -> String {
    if let Some(pos) = value.find("::") {
        let type_name = &value[..pos];
        let after = &value[pos + 2..];
        let variant_end = after
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let variant_name = &after[..variant_end];
        if let Some(variants) = enum_map.get(type_name) {
            if let Some(serde_name) = variants.get(variant_name) {
                let rest = &after[variant_end..];
                if let Some(table) = payload_to_toml(rest) {
                    return table;
                }
                return format!("\"{}\"{}", serde_name, rest);
            }
        }
    }
    value.to_string()
}

fn payload_to_toml(payload: &str) -> Option<String> {
    let payload = payload.trim();
    if payload.starts_with('(') && payload.ends_with(')') {
        let inner = &payload[1..payload.len() - 1].trim();
        if let Some((_, fields_str)) = inner.split_once('{') {
            if let Some(fields) = fields_str.strip_suffix('}') {
                let fields = fields.trim();
                if fields.is_empty() {
                    return Some(" { }".to_string());
                }
                return Some(format!(" {{ {} }}", fields_to_toml(fields)));
            }
        }
    }
    None
}

fn fields_to_toml(fields: &str) -> String {
    split_top_level(fields, ',')
        .iter()
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let Some((name, value)) = part.split_once(':') {
                let name = name.trim();
                let mut value = value.trim().to_string();
                if let Some(inner) = value
                    .strip_prefix("Some(")
                    .and_then(|s| s.strip_suffix(')'))
                    .map(|s| s.trim().to_string())
                {
                    if inner != "None" {
                        value = inner;
                    }
                }
                Some(format!("{} = {}", name, value))
            } else {
                Some(part.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_top_level(s: &str, delimiter: char) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            _ => {}
        }
        if depth == 0 && c == delimiter {
            result.push(s[start..i].to_string());
            start = i + 1;
        }
    }
    if start < s.len() {
        result.push(s[start..].to_string());
    }
    result
}

fn build_enum_defaults(sections: &[Section]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for section in sections {
        if let Section::Enum(e) = section {
            for v in &e.variants {
                if v.is_default {
                    map.insert(e.name.clone(), v.name.clone());
                    break;
                }
            }
        }
    }
    map
}

fn tomlize_default(
    default: &str,
    enum_map: &HashMap<String, HashMap<String, String>>,
    enum_defaults: &HashMap<String, String>,
    struct_defaults: &HashMap<String, BTreeMap<String, String>>,
) -> String {
    let stripped = default.trim_start_matches('`').trim_end_matches('`');
    if matches!(stripped, "None" | "—" | "Required" | "Default" | "") {
        return default.to_string();
    }

    format!(
        "`{}`",
        tomlize_resolve(stripped, enum_map, enum_defaults, struct_defaults)
    )
}

fn tomlize_resolve(
    value: &str,
    enum_map: &HashMap<String, HashMap<String, String>>,
    enum_defaults: &HashMap<String, String>,
    struct_defaults: &HashMap<String, BTreeMap<String, String>>,
) -> String {
    let value = value
        .strip_prefix("Some(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(value);

    if let Some(type_name) = value.strip_suffix("::default()") {
        if let Some(default_var) = enum_defaults.get(type_name) {
            return format!("\"{}\"", default_var);
        }
        if let Some(fields) = struct_defaults.get(type_name) {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, v)| {
                    let resolved = tomlize_resolve(v, enum_map, enum_defaults, struct_defaults);
                    format!("{} = {}", k, resolved)
                })
                .collect();
            return format!("{{ {} }}", parts.join(", "));
        }
    }

    tomlize_inner(value, enum_map)
}

fn get_serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("serde") {
            let metas = parse_serde_metas(attr);
            for meta in &metas {
                if meta.path().is_ident("rename") {
                    if let syn::Meta::NameValue(nv) = meta {
                        if let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(s),
                            ..
                        }) = &nv.value
                        {
                            return Some(s.value());
                        }
                    }
                }
            }
        }
    }
    None
}

fn collect_struct_defaults(
    syntax: &syn::File,
    default_fns: &HashMap<String, EvalResult>,
) -> HashMap<String, BTreeMap<String, String>> {
    let mut result: HashMap<String, BTreeMap<String, String>> = HashMap::new();

    for item in &syntax.items {
        if let syn::Item::Impl(imp) = item {
            let is_default = imp
                .trait_
                .as_ref()
                .and_then(|(_, path, _)| path.segments.last())
                .map(|s| s.ident == "Default")
                .unwrap_or(false);
            if !is_default {
                continue;
            }

            let struct_name = if let syn::Type::Path(tp) = imp.self_ty.as_ref() {
                tp.path.segments.last().map(|s| s.ident.to_string())
            } else {
                None
            };
            let struct_name = match struct_name {
                Some(n) => n,
                None => continue,
            };

            for item in &imp.items {
                if let syn::ImplItem::Fn(method) = item {
                    if method.sig.ident == "default" {
                        let mut field_values: BTreeMap<String, String> = BTreeMap::new();
                        if let Some(stmt) = method.block.stmts.first() {
                            let expr = match stmt {
                                syn::Stmt::Expr(e, _) => Some(e),
                                _ => None,
                            };
                            if let Some(syn::Expr::Struct(expr_struct)) = expr {
                                for f in &expr_struct.fields {
                                    if let syn::Member::Named(ident) = &f.member {
                                        if let Some(EvalResult::Literal(v)) =
                                            eval_literal_expr(&f.expr, default_fns)
                                        {
                                            field_values.insert(ident.to_string(), v);
                                        }
                                    }
                                }
                            }
                        }
                        if !field_values.is_empty() {
                            result.insert(struct_name.clone(), field_values);
                        }
                    }
                }
            }
        }
    }

    result
}

fn pretty_type(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            let path = &type_path.path;
            if let Some(segment) = path.segments.last() {
                let ident_str = segment.ident.to_string();
                match ident_str.as_str() {
                    "Option" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                            if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                                return format!("?{}", pretty_type(inner_ty));
                            }
                        }
                    }
                    "Vec" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                            if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                                return format!("{}[]", pretty_type(inner_ty));
                            }
                        }
                    }
                    "HashMap" | "BTreeMap" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                            let args_vec: Vec<_> = args
                                .args
                                .iter()
                                .filter_map(|a| {
                                    if let syn::GenericArgument::Type(t) = a {
                                        Some(pretty_type(t))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            return format!("map[{}]", args_vec.join(", "));
                        }
                    }
                    "String" => return "string".to_string(),
                    "PathBuf" => return "path".to_string(),
                    "bool" => return "boolean".to_string(),
                    "u32" | "u64" | "usize" => return "unsigned integer".to_string(),
                    "f32" | "f64" => return "float".to_string(),
                    "i32" => return "integer".to_string(),
                    _ => {}
                }
                return ident_str;
            }
            "unknown".to_string()
        }
        syn::Type::Array(type_array) => {
            let elem = pretty_type(&type_array.elem);
            let len = match &type_array.len {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(lit_int),
                    ..
                }) => lit_int.base10_digits().to_string(),
                _ => "N".to_string(),
            };
            format!("[{}; {}]", elem, len)
        }
        syn::Type::Reference(type_ref) => pretty_type(&type_ref.elem),
        _ => "unknown".to_string(),
    }
}

fn is_string_keyed_map(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                match segment.ident.to_string().as_str() {
                    "HashMap" | "BTreeMap" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                            let mut types = args.args.iter().filter_map(|a| {
                                if let syn::GenericArgument::Type(t) = a {
                                    Some(t)
                                } else {
                                    None
                                }
                            });
                            if let Some(key_ty) = types.next() {
                                if let Some(val_ty) = types.next() {
                                    if let syn::Type::Path(key_path) = key_ty {
                                        if key_path
                                            .path
                                            .segments
                                            .last()
                                            .is_some_and(|s| s.ident == "String")
                                        {
                                            return Some(pretty_type(val_ty));
                                        }
                                    }
                                }
                            }
                        }
                        None
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn eval_literal_expr(
    expr: &syn::Expr,
    default_fns: &HashMap<String, EvalResult>,
) -> Option<EvalResult> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Int(i) => Some(EvalResult::Literal(i.to_string())),
            syn::Lit::Float(f) => Some(EvalResult::Literal(f.to_string())),
            syn::Lit::Bool(b) => Some(EvalResult::Literal(b.value().to_string())),
            syn::Lit::Str(s) => Some(EvalResult::Literal(format!("\"{}\"", s.value()))),
            _ => None,
        },
        syn::Expr::Path(p) => {
            let s = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            Some(EvalResult::Literal(s))
        }
        syn::Expr::Array(arr) => {
            let els: Vec<String> = arr
                .elems
                .iter()
                .filter_map(|e| {
                    eval_literal_expr(e, default_fns).and_then(|r| match r {
                        EvalResult::Literal(v) => Some(v),
                        EvalResult::Other => None,
                    })
                })
                .collect();
            if els.len() == arr.elems.len() && !els.is_empty() {
                Some(EvalResult::Literal(format!("[{}]", els.join(", "))))
            } else {
                None
            }
        }
        syn::Expr::MethodCall(mc) => {
            if mc.method == "to_string" && mc.args.is_empty() {
                eval_literal_expr(&mc.receiver, default_fns)
            } else {
                None
            }
        }
        syn::Expr::Macro(m) => {
            let mac_str = m
                .mac
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if mac_str == "vec" {
                let content = m.mac.tokens.to_string();
                let trimmed = content.trim();
                if trimmed.len() < 60 {
                    Some(EvalResult::Literal(trimmed.to_string()))
                } else {
                    Some(EvalResult::Literal(format!("{} ...", &trimmed[..57])))
                }
            } else {
                None
            }
        }
        syn::Expr::Call(call) => {
            let callee = match &*call.func {
                syn::Expr::Path(p) => p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
                _ => String::new(),
            };
            if let Some(result) = default_fns.get(&callee) {
                return Some(result.clone());
            }
            if callee == "Some" {
                let arg = call
                    .args
                    .first()
                    .and_then(|a| eval_literal_expr(a, default_fns));
                match arg {
                    Some(EvalResult::Literal(v)) => {
                        Some(EvalResult::Literal(format!("Some({})", v)))
                    }
                    _ => None,
                }
            } else if callee == "String::new" && call.args.is_empty() {
                None
            } else if (callee == "Vec::new"
                || callee == "BTreeMap::new"
                || callee == "HashMap::new")
                && call.args.is_empty()
            {
                Some(EvalResult::Literal("[]".to_string()))
            } else if !callee.is_empty() {
                let args: Vec<String> = call
                    .args
                    .iter()
                    .filter_map(|a| {
                        eval_literal_expr(a, default_fns).and_then(|r| match r {
                            EvalResult::Literal(v) => Some(v),
                            EvalResult::Other => None,
                        })
                    })
                    .collect();
                if args.len() == call.args.len() {
                    Some(EvalResult::Literal(format!(
                        "{}({})",
                        callee,
                        args.join(", ")
                    )))
                } else {
                    None
                }
            } else {
                None
            }
        }
        syn::Expr::Struct(expr_struct) => {
            let name = expr_struct
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            let fields: Vec<String> = expr_struct
                .fields
                .iter()
                .filter_map(|f| {
                    let fname = match &f.member {
                        syn::Member::Named(ident) => ident.to_string(),
                        _ => return None,
                    };
                    eval_literal_expr(&f.expr, default_fns).and_then(|r| match r {
                        EvalResult::Literal(v) => Some(format!("{}: {}", fname, v)),
                        EvalResult::Other => None,
                    })
                })
                .collect();
            if fields.len() == expr_struct.fields.len() && !fields.is_empty() {
                Some(EvalResult::Literal(format!(
                    "{} {{ {} }}",
                    name,
                    fields.join(", ")
                )))
            } else {
                Some(EvalResult::Literal(name))
            }
        }
        _ => None,
    }
}

fn has_serde_derive(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("derive") {
            let raw = attr
                .meta
                .require_list()
                .map(|m| m.tokens.to_string())
                .unwrap_or_default();
            raw.contains("Serialize") || raw.contains("Deserialize")
        } else {
            false
        }
    })
}

fn has_serde_untagged(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("serde") {
            if let Ok(meta_list) = attr.meta.require_list() {
                meta_list.tokens.to_string().contains("untagged")
            } else {
                false
            }
        } else {
            false
        }
    })
}

fn extract_inner_type(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            if let Some(seg) = tp.path.segments.last() {
                match seg.ident.to_string().as_str() {
                    "Option" | "HashMap" | "BTreeMap" => {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            let target = if seg.ident == "Option" {
                                args.args.first()
                            } else {
                                args.args.iter().nth(1)
                            };
                            if let Some(syn::GenericArgument::Type(inner_ty)) = target {
                                return extract_inner_type(inner_ty);
                            }
                        }
                        None
                    }
                    _ => Some(seg.ident.to_string()),
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn build_struct_section_map(syntax: &syn::File) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for item in &syntax.items {
        if let syn::Item::Struct(s) = item {
            if has_config_doc_tag(&s.attrs, "root") {
                if let syn::Fields::Named(ref named) = s.fields {
                    for field in &named.named {
                        let field_name = field
                            .ident
                            .as_ref()
                            .map(|i| i.to_string())
                            .unwrap_or_default();
                        let display = get_serde_rename(&field.attrs).unwrap_or(field_name);
                        if let Some(type_name) = extract_inner_type(&field.ty) {
                            let section_key = if is_string_keyed_map(&field.ty).is_some() {
                                format!("{}.\"<name>\"", display)
                            } else {
                                display
                            };
                            map.insert(type_name, section_key);
                        }
                    }
                }
            }
        }
    }

    map
}

fn extract_doc(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(ref nv) = attr.meta {
                if let syn::Expr::Lit(ref lit) = &nv.value {
                    if let syn::Lit::Str(ref s) = &lit.lit {
                        let line = s.value().trim().to_string();
                        if !line.is_empty() {
                            lines.push(line);
                        }
                    }
                }
            }
        }
    }
    lines.join(" ")
}

fn render(sections: &[Section], section_map: &HashMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str("# cava-bg Configuration Reference\n");
    out.push('\n');
    out.push_str(&format!(
        "<!-- Auto-generated from {}. Do not edit by hand. -->\n",
        SOURCE_FILE
    ));
    out.push('\n');
    out.push_str("> **Type notation:** `?T` = optional `T`, `T[]` = array of `T`, `map[K, V]` = table keyed by `K` with value type `V`.\n");
    out.push_str(">\n");
    out.push_str("> **Legend:** `—` indicates that the field is optional and has no default value (it will be `None` or absent in TOML).\n");
    out.push('\n');

    let mut sections_with_hidden_defaults: Vec<&Section> = Vec::new();
    let mut config_sections: Vec<&Section> = Vec::new();
    let mut data_types: Vec<&Section> = Vec::new();

    for s in sections {
        match s {
            Section::Struct(st) if st.is_hide_defaults => {
                sections_with_hidden_defaults.push(s);
            }
            Section::Struct(st) if section_map.contains_key(&st.name) => {
                config_sections.push(s);
            }
            _ => {
                data_types.push(s);
            }
        }
    }

    out.push_str("## Table of Contents\n");
    out.push('\n');

    out.push_str("### Configuration Sections\n");
    out.push('\n');
    for s in sections_with_hidden_defaults
        .iter()
        .chain(config_sections.iter())
    {
        if let Section::Struct(st) = s {
            let display = section_map
                .get(&st.name)
                .map(|n| format!("[{}]", n))
                .unwrap_or(st.name.clone());
            out.push_str(&format!("- [`{}`](#{})\n", display, st.name.to_lowercase()));
        }
    }
    out.push('\n');

    out.push_str("### Data Types\n");
    out.push('\n');
    for s in &data_types {
        match s {
            Section::Struct(st) => {
                out.push_str(&format!("- [`{}`](#{})\n", st.name, st.name.to_lowercase()))
            }
            Section::Enum(e) => {
                out.push_str(&format!("- [`{}`](#{})\n", e.name, e.name.to_lowercase()))
            }
        }
    }
    out.push('\n');

    let type_names: Vec<String> = sections
        .iter()
        .map(|s| match s {
            Section::Struct(st) => st.name.clone(),
            Section::Enum(e) => e.name.clone(),
        })
        .collect();

    out.push_str("---\n");
    out.push('\n');
    out.push_str("## Configuration Sections\n");
    out.push('\n');
    for s in &sections_with_hidden_defaults {
        if let Section::Struct(st) = s {
            render_struct(st, &mut out, &type_names, section_map);
        }
    }
    for s in &config_sections {
        if let Section::Struct(st) = s {
            render_struct(st, &mut out, &type_names, section_map);
        }
    }

    out.push_str("---\n");
    out.push('\n');
    out.push_str("## Data Types\n");
    out.push('\n');
    for s in &data_types {
        match s {
            Section::Struct(st) => render_struct(st, &mut out, &type_names, section_map),
            Section::Enum(e) => render_enum(e, &mut out, &type_names, section_map),
        }
    }

    out
}

fn render_struct(
    s: &StructDoc,
    out: &mut String,
    type_names: &[String],
    section_map: &HashMap<String, String>,
) {
    if let Some(section) = section_map.get(&s.name) {
        out.push_str(&format!("<a name=\"{}\"></a>\n", s.name.to_lowercase()));
        out.push_str(&format!("## `[{}]` ({})\n", section, s.name));
    } else {
        out.push_str(&format!("## `{}`\n", s.name));
    }
    out.push('\n');
    if !s.doc.is_empty() {
        out.push_str(&format!("{}\n", s.doc));
        out.push('\n');
    }
    if s.fields.is_empty() {
        out.push_str("_No fields._\n");
        return;
    }

    if s.is_hide_defaults {
        out.push_str("| Section / Field | Type | Description |\n");
        out.push_str("| --- | --- | --- |\n");
    } else {
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("| --- | --- | --- | --- |\n");
    }

    for f in &s.fields {
        if f.name.starts_with("_legacy") || f.name.starts_with("legacy") {
            continue;
        }
        let ty = link_type(&f.type_str, type_names, section_map);
        let desc = if f.doc.is_empty() {
            "*No description*"
        } else {
            &f.doc
        };

        if s.is_hide_defaults {
            out.push_str(&format!("| `{}` | {} | {} |\n", f.name, ty, desc));
        } else {
            let default = &f.default;
            if default == "Required" && ty.starts_with('?') {
                out.push_str(&format!("| `{}` | {} | `—` | {} |\n", f.name, ty, desc));
            } else {
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    f.name, ty, default, desc
                ));
            }
        }
    }
    out.push('\n');
}

fn render_enum(
    e: &EnumDoc,
    out: &mut String,
    type_names: &[String],
    section_map: &HashMap<String, String>,
) {
    out.push_str(&format!("## `{}`\n", e.name));
    out.push('\n');
    if !e.doc.is_empty() {
        out.push_str(&format!("{}\n", e.doc));
        out.push('\n');
    }
    if e.variants.is_empty() {
        out.push_str("_No variants._\n");
        out.push('\n');
        return;
    }

    let has_payload = e.variants.iter().any(|v| !v.payload.is_empty());

    if has_payload {
        out.push_str("| Variant | Fields | Description |\n");
        out.push_str("| --- | --- | --- |\n");
    } else {
        out.push_str("| Variant | Description |\n");
        out.push_str("| --- | --- |\n");
    }

    for v in &e.variants {
        let label = if e.is_untagged && !v.payload.is_empty() {
            link_type(&v.payload, type_names, section_map)
        } else {
            format!("`{}`", v.name)
        };

        let fields_cell = if v.payload.is_empty() {
            String::new()
        } else {
            link_type(&v.payload, type_names, section_map).to_string()
        };

        let desc = if v.doc.is_empty() {
            if v.is_default {
                "(Default)".to_string()
            } else {
                "*No description*".to_string()
            }
        } else if v.is_default {
            format!("{} (Default)", v.doc)
        } else {
            v.doc.clone()
        };

        if has_payload {
            out.push_str(&format!("| {} | {} | {} |\n", label, fields_cell, desc));
        } else {
            out.push_str(&format!("| {} | {} |\n", label, desc));
        }
    }
    out.push('\n');
}

fn link_type(ty: &str, type_names: &[String], section_map: &HashMap<String, String>) -> String {
    let ty = ty.trim();
    if let Some(inner) = ty.strip_prefix('?') {
        return format!("?{}", link_type(inner, type_names, section_map));
    }
    if let Some(inner) = ty.strip_suffix("[]") {
        return format!("{}[]", link_type(inner, type_names, section_map));
    }
    if let Some(inner) = ty.strip_prefix("map[").and_then(|s| s.strip_suffix(']')) {
        let parts: Vec<&str> = inner.splitn(2, ", ").collect();
        let linked: Vec<String> = parts
            .iter()
            .map(|p| link_type(p.trim(), type_names, section_map))
            .collect();
        return format!("map[{}]", linked.join(", "));
    }
    if type_names.iter().any(|n| n == ty) {
        let text = section_map
            .get(ty)
            .map(|s| format!("[{}]", s))
            .unwrap_or_else(|| ty.to_string());
        return format!("[{}](#{})", text, ty.to_lowercase());
    }
    ty.to_string()
}
