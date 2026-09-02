use super::*;
use crate::artifact::rust_ident;

#[derive(Debug, Clone)]
pub(super) struct RustStructFact {
    pub(super) fields: Vec<TemplateFieldFact>,
    pub(super) derives_hemplate: bool,
}

pub(super) fn context_type_for_heml_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut out = String::new();
    for word in stem.split(['_', '-']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        out.extend(chars.next()?.to_uppercase());
        out.extend(chars);
    }
    (!out.is_empty()).then_some(out)
}

pub(super) fn nearest_dir_with(path: &Path, file_name: &str) -> Option<PathBuf> {
    for dir in path.ancestors().filter(|path| path.is_dir()) {
        if dir.join(file_name).is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

pub(super) fn rust_struct_facts_in(root: &Path) -> io::Result<HashMap<String, RustStructFact>> {
    let mut facts = HashMap::new();
    collect_rust_struct_facts(&root.join("src"), &mut facts)?;
    Ok(facts)
}

fn collect_rust_struct_facts(
    dir: &Path,
    facts: &mut HashMap<String, RustStructFact>,
) -> io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_struct_facts(&path, facts)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            collect_rust_struct_facts_from_file(&path, facts)?;
        }
    }
    Ok(())
}

pub(super) fn collect_rust_struct_facts_from_file(
    path: &Path,
    facts: &mut HashMap<String, RustStructFact>,
) -> io::Result<()> {
    let source = std::fs::read_to_string(path)?;
    let file = match syn::parse_file(&source) {
        Ok(file) => file,
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    };
    collect_rust_struct_facts_from_items(&file.items, facts);
    Ok(())
}

fn collect_rust_struct_facts_from_items(
    items: &[syn::Item],
    facts: &mut HashMap<String, RustStructFact>,
) {
    for item in items {
        match item {
            syn::Item::Struct(item) => {
                if let syn::Fields::Named(fields) = &item.fields {
                    facts.insert(
                        item.ident.to_string(),
                        RustStructFact {
                            fields: fields
                                .named
                                .iter()
                                .map(|field| TemplateFieldFact {
                                    name: field.ident.as_ref().unwrap().to_string(),
                                    type_name: compact_tokens(&field.ty),
                                })
                                .collect(),
                            derives_hemplate: derives_hemplate(&item.attrs),
                        },
                    );
                }
            }
            syn::Item::Mod(item) => {
                if let Some((_, items)) = &item.content {
                    collect_rust_struct_facts_from_items(items, facts);
                }
            }
            _ => {}
        }
    }
}

fn derives_hemplate(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .filter_map(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .ok()
        })
        .flatten()
        .any(|path| {
            path.segments
                .last()
                .is_some_and(|segment| segment.ident == "Hemplate")
        })
}

pub(super) fn compact_tokens(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace(", ", ",")
        .replace(" & ", "&")
        .replace("& ", "&")
}

pub(super) fn loop_locals_for_surface(
    surface: &SurfaceDocument,
    self_fields: &[TemplateFieldFact],
    structs: &HashMap<String, RustStructFact>,
) -> Vec<TemplateLocalFact> {
    let mut locals = Vec::new();
    for node in &surface.nodes {
        let Some(value) = node
            .attrs
            .iter()
            .find(|attr| attr.name == "h-for")
            .and_then(|attr| attr.value.as_deref())
        else {
            continue;
        };
        let Some((local, field)) = h_for_local_and_self_field(value) else {
            continue;
        };
        let Some(self_field) = self_fields.iter().find(|candidate| candidate.name == field) else {
            continue;
        };
        let Some(type_name) = vec_element_type(&self_field.type_name) else {
            continue;
        };
        let fields = structs
            .get(&type_name)
            .map(|fact| fact.fields.clone())
            .unwrap_or_default();
        if !locals
            .iter()
            .any(|existing: &TemplateLocalFact| existing.name == local)
        {
            locals.push(TemplateLocalFact {
                name: local,
                type_name,
                fields,
            });
        }
    }
    locals
}

pub(super) fn h_for_local_and_self_field(value: &str) -> Option<(String, String)> {
    let (local, expr) = value.split_once(" in ")?;
    let local = rust_ident(local.trim())?;
    let expr = expr.trim().strip_prefix('&').unwrap_or(expr.trim()).trim();
    let field = expr
        .strip_prefix("self.")?
        .split(['.', '(', '['])
        .next()
        .filter(|field| !field.is_empty())?;
    Some((local, field.to_owned()))
}

pub(super) fn vec_element_type(type_name: &str) -> Option<String> {
    let inner = type_name
        .strip_prefix("Vec<")
        .or_else(|| type_name.strip_prefix("std::vec::Vec<"))?
        .strip_suffix('>')?;
    Some(inner.trim().to_owned())
}
