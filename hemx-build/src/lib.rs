use hemplate_core::ast::build_ast;
use hemplate_core::surface::{
    extract_surface, AttributeOrigin, ControlKind, ScopeId, ScopeKind, SurfaceAttribute,
    SurfaceDocument, SurfaceNodeKind, SurfaceScope,
};
use hemx_core::{EFFECT_BATCH_ABI_VERSION, RUNTIME_ABI_VERSION, SURFACE_SCHEMA_VERSION};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    UnkeyedGeneratedTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub file: PathBuf,
    pub directive: String,
    pub target: String,
    pub message: String,
    pub expected: String,
    pub repair: String,
}

impl Diagnostic {
    pub fn to_io_error(&self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, self.to_string())
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}; expected {}; repair: {}",
            self.file.display(),
            self.message,
            self.expected,
            self.repair
        )
    }
}

#[derive(Clone, Debug)]
pub struct AppBuilder {
    out_dir: Option<PathBuf>,
    template_dir: PathBuf,
    global_exports: bool,
    surfaces: Vec<(PathBuf, SurfaceDocument)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedTarget {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateContextFacts {
    pub context_type: String,
    pub self_fields: Vec<TemplateFieldFact>,
    pub locals: Vec<TemplateLocalFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFieldFact {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateLocalFact {
    pub name: String,
    pub type_name: String,
    pub fields: Vec<TemplateFieldFact>,
}

#[derive(Debug, Clone)]
struct RustStructFact {
    fields: Vec<TemplateFieldFact>,
    derives_hemplate: bool,
}

pub fn diagnostics_for_heml_file(path: impl AsRef<Path>) -> io::Result<Vec<Diagnostic>> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)?;
    diagnostics_for_heml_source(path, source)
}

pub fn diagnostics_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Vec<Diagnostic>> {
    let path = path.as_ref();
    let surface = surface_for_heml_source(path, source.into())?;
    Ok(unkeyed_generated_target_diagnostics(path, &surface))
}

pub fn generated_targets_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Vec<GeneratedTarget>> {
    let path = path.as_ref();
    let surface = surface_for_heml_source(path, source.into())?;
    Ok(generated_targets(&surface))
}

pub fn template_context_facts_for_heml_file(
    path: impl AsRef<Path>,
) -> io::Result<Option<TemplateContextFacts>> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)?;
    template_context_facts_for_heml_source(path, source)
}

pub fn template_context_facts_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Option<TemplateContextFacts>> {
    let path = path.as_ref();
    let source = source.into();
    let Some(context_type) = context_type_for_heml_path(path) else {
        return Ok(None);
    };
    let Some(root) = nearest_dir_with(path, "Cargo.toml") else {
        return Ok(None);
    };
    let structs = rust_struct_facts_in(&root)?;
    let Some(context) = structs.get(&context_type) else {
        return Ok(None);
    };
    if !context.derives_hemplate {
        return Ok(None);
    }

    let surface = surface_for_heml_source(path, source)?;
    let locals = loop_locals_for_surface(&surface, &context.fields, &structs);
    Ok(Some(TemplateContextFacts {
        context_type,
        self_fields: context.fields.clone(),
        locals,
    }))
}

fn surface_for_heml_source(path: &Path, source: String) -> io::Result<SurfaceDocument> {
    let doc = build_ast(Arc::new(source)).map_err(|err| parse_error(path, err))?;
    let Some(doc) = doc else {
        return Ok(SurfaceDocument {
            nodes: Vec::new(),
            scopes: vec![SurfaceScope {
                parent: None,
                kind: ScopeKind::Root,
            }],
            forms: Vec::new(),
        });
    };
    Ok(extract_surface(&doc))
}

impl AppBuilder {
    pub fn out_dir(mut self, out_dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(out_dir.into());
        self
    }

    pub fn template_dir(mut self, template_dir: impl Into<PathBuf>) -> Self {
        self.template_dir = template_dir.into();
        self
    }

    pub fn global_exports(mut self, enabled: bool) -> Self {
        self.global_exports = enabled;
        self
    }

    pub fn surface(mut self, path: impl Into<PathBuf>, surface: SurfaceDocument) -> Self {
        self.surfaces.push((path.into(), surface));
        self
    }

    pub fn run(self) -> io::Result<()> {
        println!("cargo:rerun-if-changed={}", self.template_dir.display());

        let out_dir = self
            .out_dir
            .or_else(|| std::env::var_os("OUT_DIR").map(PathBuf::from));

        let Some(out_dir) = out_dir else {
            return Ok(());
        };

        std::fs::create_dir_all(&out_dir)?;

        let mut resources = Resources::default();
        let input_paths = collect_input_files(&self.template_dir)?;
        if self.surfaces.is_empty() {
            for path in input_paths.iter().filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("heml")
            }) {
                let source = std::fs::read_to_string(path)?;
                let surface = surface_for_heml_source(path, source)?;
                resources.add_surface(&self.template_dir, path, &surface)?;
            }
        } else {
            for (path, surface) in &self.surfaces {
                resources.add_surface(&self.template_dir, path, surface)?;
            }
        }
        for path in input_paths.iter().filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("css" | "scss")
            )
        }) {
            let source = std::fs::read_to_string(path)?;
            resources.add_stylesheet(&self.template_dir, path, &source)?;
        }

        write_if_changed(
            &out_dir.join("hemx.generated.rs"),
            resources.generated_rs(self.global_exports).as_bytes(),
        )?;
        write_if_changed(&out_dir.join("hemx.syms"), resources.syms().as_bytes())?;
        write_if_changed(
            &out_dir.join("hemx.client.js"),
            resources.client_bootstrap()?.as_bytes(),
        )?;
        Ok(())
    }
}

fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::write(path, contents)
}

pub fn app() -> AppBuilder {
    AppBuilder {
        out_dir: None,
        template_dir: PathBuf::from("templates"),
        global_exports: false,
        surfaces: Vec::new(),
    }
}

#[derive(Default)]
struct Resources {
    slots: BTreeMap<String, Resource>,
    handles: BTreeMap<String, Resource>,
    handle_forms: BTreeMap<String, String>,
    handle_params: BTreeMap<String, BTreeSet<String>>,
    forms: BTreeMap<String, FormResource>,
    atoms: BTreeMap<String, Resource>,
    classes: BTreeMap<String, ClassToken>,
    events: BTreeMap<String, EventToken>,
    client_handlers: BTreeSet<String>,
    client_modules: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct Resource {
    symbol: String,
    ident: String,
    component: String,
    keyed: bool,
    id: u32,
}

#[derive(Clone, Debug)]
struct FormResource {
    resource: Resource,
    controls: Vec<GeneratedControl>,
}

#[derive(Clone, Debug)]
struct GeneratedControl {
    name: String,
    kind: ControlKind,
    required: bool,
}

#[derive(Clone, Debug)]
struct ClassToken {
    symbol: String,
    ident: String,
    component: String,
    token: String,
}

#[derive(Clone, Debug)]
struct EventToken {
    symbol: String,
    ident: String,
    component: String,
    name: String,
}

impl Resources {
    fn add_surface(
        &mut self,
        root: &Path,
        path: &Path,
        surface: &SurfaceDocument,
    ) -> io::Result<()> {
        let component = component_ident(root, path)?;
        for node in &surface.nodes {
            let SurfaceNodeKind::Element { tag } = &node.kind else {
                continue;
            };

            reject_selector_target_attrs(path, &node.attrs)?;
            reject_unknown_hemx_attrs(path, &node.attrs)?;
            reject_invalid_hemx_attr_values(path, &node.attrs)?;
            reject_invalid_hemx_attr_placement(path, tag, &node.attrs)?;

            if let Some(class_attr) = static_attr(&node.attrs, "class") {
                for token in class_tokens(&class_attr) {
                    let canonical = canonical_symbol(root, path, token);
                    self.insert_class(canonical, token.to_owned(), component.clone())?;
                }
            }

            if let Some(on_attr) = static_attr(&node.attrs, "data-hemx-on") {
                for event in event_tokens(&on_attr) {
                    let canonical = canonical_symbol(root, path, event);
                    self.insert_event(canonical, event.to_owned(), component.clone());
                }
            }

            if let Some(handler) = static_attr(&node.attrs, "data-hemx-client") {
                let Some(handler) = rust_ident(&handler) else {
                    return Err(invalid_hemx_value(
                        path,
                        "data-hemx-client",
                        &handler,
                        "expected a Rust handler identifier",
                    ));
                };
                self.client_handlers.insert(handler);
            }
            if let Some(module) = static_attr(&node.attrs, "data-hemx-client-module") {
                self.client_modules.insert(module);
            }

            if let Some(name) = static_attr(&node.attrs, "data-hemx-slot") {
                reject_unkeyed_loop(surface, node.scope, path, "slot", &name)?;
                let keyed = is_inside_keyed_for(surface, node.scope)
                    || (can_host_keyed_collection(tag)
                        && has_descendant_keyed_for_scope(surface, node.scope));
                let canonical = canonical_symbol(root, path, &name);
                self.insert_slot(canonical, name, component.clone(), keyed)?;
            }

            if let Some(name) = static_attr(&node.attrs, "data-hemx-atom") {
                reject_unkeyed_loop(surface, node.scope, path, "atom", &name)?;
                let canonical = canonical_symbol(root, path, &name);
                self.insert_atom(canonical, name, component.clone())?;
            }

            if let Some(name) = static_attr(&node.attrs, "data-hemx-handle") {
                reject_unkeyed_loop(surface, node.scope, path, "handle", &name)?;
                let canonical = canonical_symbol(root, path, &name);
                self.insert_handle(canonical, name.clone(), component.clone())?;
                self.insert_handle_params(&name, &node.attrs)?;

                if tag == "form" {
                    let form_name =
                        static_attr(&node.attrs, "data-hemx-form").unwrap_or_else(|| name.clone());
                    let controls = surface
                        .forms
                        .iter()
                        .find(|form| form.node == node.id)
                        .map(|form| {
                            form.controls
                                .iter()
                                .map(|control| GeneratedControl {
                                    name: control.name.clone(),
                                    kind: control.kind.clone(),
                                    required: control.required,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.insert_form(
                        canonical_symbol(root, path, &form_name),
                        form_name.clone(),
                        component.clone(),
                        controls,
                    )?;
                    if let (Some(handle_ident), Some(form_ident)) =
                        (rust_ident(&name), rust_ident(&form_name))
                    {
                        self.handle_forms.insert(handle_ident, form_ident);
                    }
                }
            }
        }
        Ok(())
    }

    fn insert_slot(
        &mut self,
        symbol: String,
        name: String,
        component: String,
        keyed: bool,
    ) -> io::Result<()> {
        let slot = insert_resource(&mut self.slots, "slot", symbol, name, component)?;
        slot.keyed |= keyed;
        Ok(())
    }

    fn insert_handle(&mut self, symbol: String, name: String, component: String) -> io::Result<()> {
        insert_resource(&mut self.handles, "handle", symbol, name, component).map(|_| ())
    }

    fn insert_atom(&mut self, symbol: String, name: String, component: String) -> io::Result<()> {
        insert_resource(&mut self.atoms, "atom", symbol, name, component).map(|_| ())
    }

    fn insert_class(&mut self, symbol: String, token: String, component: String) -> io::Result<()> {
        let ident = class_ident(&token).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid CSS class `{token}`; expected an ASCII class token usable from Rust"
                ),
            )
        })?;
        let class = ClassToken {
            symbol,
            ident,
            component,
            token,
        };
        if let Some(existing) = self
            .classes
            .values()
            .find(|existing| existing.ident == class.ident && existing.token != class.token)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "duplicate generated class identifier `{}` for CSS classes `{}` and `{}`",
                    class.ident, existing.token, class.token
                ),
            ));
        }
        self.classes.entry(class.symbol.clone()).or_insert(class);
        Ok(())
    }

    fn insert_handle_params(&mut self, handle: &str, attrs: &[SurfaceAttribute]) -> io::Result<()> {
        let Some(handle_ident) = rust_ident(handle) else {
            return Ok(());
        };
        for attr in attrs {
            if !is_handle_param_attr(attr) {
                continue;
            }
            let Some(param) = data_param_ident(&attr.name) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid handler param attribute `{}`; expected data-* name usable from Rust", attr.name),
                ));
            };
            self.handle_params
                .entry(handle_ident.clone())
                .or_default()
                .insert(param);
        }
        Ok(())
    }

    fn insert_event(&mut self, symbol: String, name: String, component: String) {
        self.events.entry(symbol.clone()).or_insert(EventToken {
            symbol,
            ident: name.clone(),
            component,
            name,
        });
    }

    fn insert_form(
        &mut self,
        symbol: String,
        name: String,
        component: String,
        controls: Vec<GeneratedControl>,
    ) -> io::Result<()> {
        let controls = controls
            .into_iter()
            .filter(|control| control.name != "__h")
            .collect();
        let resource = make_resource("form", symbol, name, component)?;
        match self.forms.get(&resource.ident) {
            Some(existing) if existing.resource.symbol != resource.symbol => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "duplicate generated identifier `{}` for `{}` and `{}`",
                    resource.ident, existing.resource.symbol, resource.symbol
                ),
            )),
            Some(_) => Ok(()),
            None => {
                self.forms
                    .insert(resource.ident.clone(), FormResource { resource, controls });
                Ok(())
            }
        }
    }

    fn add_stylesheet(&mut self, root: &Path, path: &Path, source: &str) -> io::Result<()> {
        let component = component_ident(root, path)?;
        for token in stylesheet_class_tokens(source) {
            let canonical = canonical_symbol(root, path, token);
            self.insert_class(canonical, token.to_owned(), component.clone())?;
        }
        Ok(())
    }

    fn generated_rs(&self, global_exports: bool) -> String {
        let mut out = String::new();
        out.push_str("// @generated by hemx-build. Do not edit.\n");
        out.push_str(&format!(
            "pub const BUILD_FINGERPRINT: ::hemx::advanced::BuildFingerprint = ::hemx::advanced::BuildFingerprint::from_parts(&[{}]);\n\n",
            self.fingerprint_parts()
                .into_iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));

        let component_names: BTreeSet<String> = self.component_names().into_iter().collect();

        let root_indent = usize::default();
        if global_exports {
            self.push_component_refs(&mut out);
            self.push_resource_modules(&mut out, &component_names, None, root_indent);
        }
        self.push_lowering_api(&mut out, None, root_indent);

        for component in self.component_names() {
            out.push_str("\n#[allow(non_upper_case_globals)]\n");
            out.push_str(&format!("pub mod {component} {{\n"));
            out.push_str("    #[derive(Clone, Copy)]\n");
            out.push_str("    pub struct Component;\n\n");
            self.push_resource_modules(&mut out, &component_names, Some(&component), 1);
            self.push_lowering_api(&mut out, Some(&component), 1);
            out.push_str("}\n");
        }

        self.push_lowering_helpers(&mut out);
        out
    }

    fn push_component_refs(&self, out: &mut String) {
        out.push_str("#[allow(non_upper_case_globals)]\npub mod components {\n");
        for component in self.component_names() {
            out.push_str(&format!(
                "    pub const {component}: ::hemx::ComponentRef = ::hemx::ComponentRef::new({});\n",
                rust_str(&component)
            ));
        }
        out.push_str("}\n");
    }

    fn push_resource_modules(
        &self,
        out: &mut String,
        components: &BTreeSet<String>,
        component: Option<&str>,
        indent: usize,
    ) {
        let pad = "    ".repeat(indent);
        let inner = "    ".repeat(indent + 1);
        let mut handle_ids = Vec::new();
        let mut root_exports = BTreeSet::new();

        if let Some(component) = component {
            out.push_str(&format!(
                "{pad}pub const COMPONENT: ::hemx::ComponentRef = ::hemx::ComponentRef::new({});\n",
                rust_str(component)
            ));
        }

        out.push_str(&format!("{pad}#[doc(hidden)]\n{pad}pub mod advanced {{\n"));
        out.push_str(&format!(
            "{inner}#[allow(non_upper_case_globals)]\n{inner}pub mod slots {{\n"
        ));
        let slot_inner = "    ".repeat(indent + 2);
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
        {
            if res.keyed {
                out.push_str(&format!(
                    "{slot_inner}pub const {}: ::hemx::advanced::KeyedSlot<::std::string::String, ::std::string::String> = ::hemx::advanced::KeyedSlot::new({});\n",
                    res.ident, res.id
                ));
            } else {
                out.push_str(&format!(
                    "{slot_inner}pub const {}: ::hemx::advanced::Slot<::std::string::String> = ::hemx::advanced::Slot::new({});\n",
                    res.ident, res.id
                ));
            }
        }
        out.push_str(&format!("{inner}}}\n"));
        out.push_str(&format!("{pad}}}\n\n"));

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod targets {{\n"
        ));
        let child_prefix = if component.is_some() {
            "super::super::"
        } else {
            "super::"
        };
        out.push_str(&format!("{inner}#[derive(Clone, Copy)]\n"));
        out.push_str(&format!(
            "{inner}pub struct SlotTarget<T, C = ()> {{ slot: ::hemx::advanced::Slot<T>, _marker: ::std::marker::PhantomData<C> }}\n"
        ));
        out.push_str(&format!("{inner}#[allow(dead_code)]\n"));
        out.push_str(&format!("{inner}impl<T, C> SlotTarget<T, C> {{\n"));
        out.push_str(&format!("{inner}    const fn new(slot: ::hemx::advanced::Slot<T>) -> Self {{ Self {{ slot, _marker: ::std::marker::PhantomData }} }}\n"));
        out.push_str(&format!("{inner}    pub fn text(self, value: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.text(value) }}\n"));
        out.push_str(&format!("{inner}    pub fn set(self, value: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.text(value) }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        out.push_str(&format!("{inner}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{inner}impl<T> SlotTarget<T, ()> {{\n"));
        out.push_str(&format!("{inner}    pub fn put(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ super::put(self.slot, view) }}\n"));
        out.push_str(&format!("{inner}    pub fn replace(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ super::put(self.slot, view) }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
            .filter(|res| components.contains(&res.ident) && Some(res.ident.as_str()) != component)
        {
            let child = &res.ident;
            out.push_str(&format!("{inner}#[cfg(not(target_arch = \"wasm32\"))]\n"));
            out.push_str(&format!(
                "{inner}impl<T> SlotTarget<T, {child_prefix}{child}::Component> {{\n"
            ));
            out.push_str(&format!("{inner}    pub fn put(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ {child_prefix}{child}::put(self.slot, view) }}\n"));
            out.push_str(&format!("{inner}    pub fn replace(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ {child_prefix}{child}::put(self.slot, view) }}\n"));
            out.push_str(&format!("{inner}}}\n"));
        }
        out.push_str(&format!(
            "{inner}impl<T, C> ::hemx::GeneratedTarget for SlotTarget<T, C> {{\n"
        ));
        out.push_str(&format!("{inner}    fn __hemx_resource_id(self) -> ::hemx::advanced::ResourceId {{ self.slot.id() }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        out.push_str(&format!("{inner}#[derive(Clone, Copy)]\n"));
        out.push_str(&format!("{inner}pub struct KeyedSlotTarget<K, T, C = ()> {{ slot: ::hemx::advanced::KeyedSlot<K, T>, _marker: ::std::marker::PhantomData<C> }}\n"));
        out.push_str(&format!("{inner}#[allow(dead_code)]\n"));
        out.push_str(&format!("{inner}impl<K, T, C> KeyedSlotTarget<K, T, C>\n"));
        out.push_str(&format!("{inner}where\n"));
        out.push_str(&format!("{inner}    K: ::std::string::ToString,\n"));
        out.push_str(&format!("{inner}{{\n"));
        out.push_str(&format!("{inner}    const fn new(slot: ::hemx::advanced::KeyedSlot<K, T>) -> Self {{ Self {{ slot, _marker: ::std::marker::PhantomData }} }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        out.push_str(&format!(
            "{inner}impl<K: ::std::string::ToString, T, C> KeyedSlotTarget<K, T, C> {{\n"
        ));
        out.push_str(&format!("{inner}    pub fn move_before(self, key: K, before: K) -> ::hemx::advanced::Effect {{ self.slot.move_before(key, before) }}\n"));
        out.push_str(&format!("{inner}    pub fn move_to_end(self, key: K) -> ::hemx::advanced::Effect {{ self.slot.move_to_end(key) }}\n"));
        out.push_str(&format!("{inner}    pub fn remove_key(self, key: K) -> ::hemx::advanced::Effect {{ self.slot.remove(key) }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        out.push_str(&format!("{inner}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!(
            "{inner}impl<T> KeyedSlotTarget<::std::string::String, T, ()> {{\n"
        ));
        out.push_str(&format!("{inner}    pub fn append(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.append_html(view.hemx_key(), super::render(&view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn prepend(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.prepend_html(view.hemx_key(), super::render(&view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn replace(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.replace_html(view.hemx_key(), super::render(&view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn append_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.append_html(key.to_string(), super::render(view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn prepend_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.prepend_html(key.to_string(), super::render(view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn replace_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.replace_html(key.to_string(), super::render(view)) }}\n"));
        out.push_str(&format!("{inner}    pub fn remove(self, key: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.remove(key.to_string()) }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
            .filter(|res| res.keyed)
            .filter(|res| components.contains(&res.ident) && Some(res.ident.as_str()) != component)
        {
            let child = &res.ident;
            out.push_str(&format!("{inner}#[cfg(not(target_arch = \"wasm32\"))]\n"));
            out.push_str(&format!(
                "{inner}impl<T> KeyedSlotTarget<::std::string::String, T, {child_prefix}{child}::Component> {{\n"
            ));
            out.push_str(&format!("{inner}    pub fn append(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.append_html(view.hemx_key(), {child_prefix}{child}::render(&view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn prepend(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.prepend_html(view.hemx_key(), {child_prefix}{child}::render(&view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn replace(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.slot.replace_html(view.hemx_key(), {child_prefix}{child}::render(&view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn append_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.append_html(key.to_string(), {child_prefix}{child}::render(view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn prepend_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.prepend_html(key.to_string(), {child_prefix}{child}::render(view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn replace_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.slot.replace_html(key.to_string(), {child_prefix}{child}::render(view)) }}\n"));
            out.push_str(&format!("{inner}    pub fn remove(self, key: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.remove(key.to_string()) }}\n"));
            out.push_str(&format!("{inner}}}\n"));
        }
        out.push_str(&format!(
            "{inner}impl<K: ::std::string::ToString, T, C> ::hemx::GeneratedTarget for KeyedSlotTarget<K, T, C> {{\n"
        ));
        out.push_str(&format!("{inner}    fn __hemx_resource_id(self) -> ::hemx::advanced::ResourceId {{ self.slot.id() }}\n"));
        out.push_str(&format!("{inner}}}\n"));
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
        {
            let marker = if components.contains(&res.ident) && Some(res.ident.as_str()) != component
            {
                format!("{child_prefix}{}::Component", res.ident)
            } else {
                "()".to_string()
            };
            if res.keyed {
                out.push_str(&format!(
                    "{inner}pub const {}: KeyedSlotTarget<::std::string::String, ::std::string::String, {marker}> = KeyedSlotTarget::new(super::advanced::slots::{});\n",
                    res.ident, res.ident
                ));
            } else {
                out.push_str(&format!(
                    "{inner}pub const {}: SlotTarget<::std::string::String, {marker}> = SlotTarget::new(super::advanced::slots::{});\n",
                    res.ident, res.ident
                ));
            }
        }
        out.push_str(&format!("{pad}}}\n"));
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
        {
            root_exports.insert(res.ident.clone());
            out.push_str(&format!("{pad}pub use self::targets::{};\n", res.ident));
        }
        out.push('\n');

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod handles {{\n"
        ));
        for res in self
            .handles
            .values()
            .filter(|res| component_matches(res, component))
        {
            handle_ids.push(res.id);
            let input = if self.handle_forms.contains_key(&res.ident) {
                "::hemx::Form<::std::string::String>"
            } else {
                "()"
            };
            out.push_str(&format!(
                "{inner}pub const {}: ::hemx::Handle<{input}> = ::hemx::Handle::new({});\n",
                res.ident, res.id
            ));
        }
        out.push_str(&format!(
            "{inner}pub const ALL_IDS: &[u32] = &[{}];\n",
            handle_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!("{pad}}}\n"));
        for res in self
            .handles
            .values()
            .filter(|res| component_matches(res, component))
        {
            push_root_export(
                out,
                &pad,
                "handles",
                &mut root_exports,
                &res.ident,
                "handle",
            );
        }
        out.push('\n');

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod params {{\n"
        ));
        let mut emitted_params = BTreeSet::new();
        for res in self
            .handles
            .values()
            .filter(|res| component_matches(res, component))
        {
            if let Some(params) = self.handle_params.get(&res.ident) {
                for param in params {
                    if emitted_params.insert(param.as_str()) {
                        out.push_str(&format!(
                            "{inner}pub const {param}: ::hemx::ParamName = ::hemx::ParamName::new({});\n",
                            rust_str(param)
                        ));
                    }
                }
            }
        }
        out.push_str(&format!("{pad}}}\n\n"));

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod atoms {{\n"
        ));
        for res in self
            .atoms
            .values()
            .filter(|res| component_matches(res, component))
        {
            out.push_str(&format!(
                "{inner}pub const {}: ::hemx::Atom<::std::string::String> = ::hemx::Atom::new({});\n",
                res.ident, res.id
            ));
        }
        out.push_str(&format!("{pad}}}\n\n"));

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod classes {{\n"
        ));
        let mut emitted_classes = BTreeSet::new();
        for class in self
            .classes
            .values()
            .filter(|class| class_matches(class, component))
        {
            if !emitted_classes.insert(class.ident.as_str()) {
                continue;
            }
            out.push_str(&format!(
                "{inner}pub const {}: ::hemx::CssClass = ::hemx::CssClass::new({});\n",
                class.ident,
                rust_str(&class.token)
            ));
        }
        out.push_str(&format!("{pad}}}\n\n"));

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod events {{\n"
        ));
        let mut emitted_events = BTreeSet::new();
        for event in self
            .events
            .values()
            .filter(|event| event_matches(event, component))
        {
            if !emitted_events.insert(event.ident.as_str()) {
                continue;
            }
            out.push_str(&format!(
                "{inner}pub const {}: ::hemx::EventName = ::hemx::EventName::new({});\n",
                event.ident,
                rust_str(&event.name)
            ));
        }
        out.push_str(&format!("{pad}}}\n\n"));

        out.push_str(&format!(
            "{pad}#[allow(non_upper_case_globals)]\n{pad}pub mod forms {{\n"
        ));
        for form in self
            .forms
            .values()
            .filter(|form| component_matches(&form.resource, component))
        {
            let res = &form.resource;
            out.push_str(&format!(
                "{inner}pub const {}: ::hemx::Form<::std::string::String> = ::hemx::Form::new({});\n",
                res.ident, res.id
            ));
            out.push_str(&format!(
                "{inner}pub const {}_CONTRACT: ::hemx::FormContract = ::hemx::FormContract {{ fields: {}_FIELDS }};\n",
                res.ident.to_ascii_uppercase(), res.ident.to_ascii_uppercase()
            ));
            out.push_str(&format!(
                "{inner}pub const {}_FIELDS: &[::hemx::FormField] = &[\n",
                res.ident.to_ascii_uppercase()
            ));
            for control in &form.controls {
                out.push_str(&format!(
                    "{inner}    ::hemx::FormField {{ name: {}, kind: {}, required: {} }},\n",
                    rust_str(&control.name),
                    form_control_kind_expr(&control.kind),
                    control.required
                ));
            }
            out.push_str(&format!("{inner}];\n"));
        }
        out.push_str(&format!("{pad}}}\n"));
        for form in self
            .forms
            .values()
            .filter(|form| component_matches(&form.resource, component))
        {
            push_root_export(
                out,
                &pad,
                "forms",
                &mut root_exports,
                &form.resource.ident,
                "form",
            );
        }
    }

    fn push_lowering_api(&self, out: &mut String, component: Option<&str>, indent: usize) {
        let pad = "    ".repeat(indent);
        let table_name = match component {
            Some(component) => format!("__HEMX_LOWERING_TABLE_{}", component.to_ascii_uppercase()),
            None => "__HEMX_LOWERING_TABLE".to_string(),
        };
        out.push_str(&format!("\n{pad}pub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String {{\n"));
        out.push_str(&format!("{pad}    "));
        if component.is_some() {
            out.push_str("super::");
        }
        out.push_str(&format!("__hemx_lower_html(html.as_ref(), {table_name})\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn lower_html(html: impl ::std::convert::AsRef<str>) -> ::std::string::String {{ lower(html) }}\n\n"));
        out.push_str(&format!(
            "{pad}pub fn static_fragment(html: &'static str) -> ::hemx::Html {{\n"
        ));
        out.push_str(&format!(
            "{pad}    ::hemx::__private::html_trusted(lower(html))\n"
        ));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!(
            "{pad}pub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html {{\n"
        ));
        out.push_str(&format!(
            "{pad}    let mut html = ::std::string::String::new();\n"
        ));
        out.push_str(&format!("{pad}    ::hemplate::Hemplate::render_into(view, &mut html).expect(\"hemx hemplate view renders\");\n"));
        out.push_str(&format!(
            "{pad}    ::hemx::__private::html_trusted(lower(html))\n"
        ));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!(
            "{pad}#[cfg(not(target_arch = \"wasm32\"))]\n{pad}pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html {{ render(view) }}\n\n"
        ));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html {{ render(view) }}\n\n"));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{\n"));
        out.push_str(&format!("{pad}    slot.html(render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn append<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect\n"));
        out.push_str(&format!("{pad}where\n"));
        out.push_str(&format!("{pad}    K: ::std::string::ToString,\n"));
        out.push_str(&format!("{pad}{{\n"));
        out.push_str(&format!("{pad}    slot.append_html(key, render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn prepend<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect\n"));
        out.push_str(&format!("{pad}where\n"));
        out.push_str(&format!("{pad}    K: ::std::string::ToString,\n"));
        out.push_str(&format!("{pad}{{\n"));
        out.push_str(&format!("{pad}    slot.prepend_html(key, render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[cfg(not(target_arch = \"wasm32\"))]\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect\n"));
        out.push_str(&format!("{pad}where\n"));
        out.push_str(&format!("{pad}    K: ::std::string::ToString,\n"));
        out.push_str(&format!("{pad}{{\n"));
        out.push_str(&format!("{pad}    slot.replace_html(key, render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        self.push_lowering_table(out, component, indent, &table_name);
    }

    fn push_lowering_table(
        &self,
        out: &mut String,
        component: Option<&str>,
        indent: usize,
        name: &str,
    ) {
        let pad = "    ".repeat(indent);
        out.push_str(&format!("{pad}const {name}: &[(&str, &str, u32)] = &[\n"));
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
        {
            out.push_str(&format!(
                "{pad}    (\"data-hemx-slot\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        for res in self
            .handles
            .values()
            .filter(|res| component_matches(res, component))
        {
            out.push_str(&format!(
                "{pad}    (\"data-hemx-handle\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        for form in self
            .forms
            .values()
            .filter(|form| component_matches(&form.resource, component))
        {
            let res = &form.resource;
            out.push_str(&format!(
                "{pad}    (\"data-hemx-form\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        for res in self
            .atoms
            .values()
            .filter(|res| component_matches(res, component))
        {
            out.push_str(&format!(
                "{pad}    (\"data-hemx-atom\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        out.push_str(&format!("{pad}];\n"));
    }

    fn push_lowering_helpers(&self, out: &mut String) {
        out.push_str(r#"
fn __hemx_lower_html(html: &str, table: &[(&str, &str, u32)]) -> ::std::string::String {
    let mut out = html.to_owned();
    for (attr, name, id) in table {
        let runtime_attr = match *attr {
            "data-hemx-slot" => "data-sid",
            "data-hemx-handle" => "data-hid",
            "data-hemx-form" => "data-fid",
            "data-hemx-revealed" => "data-hemx-revealed",
            "data-hemx-atom" => "data-aid",
            _ => continue,
        };
        out = __hemx_replace_attr(out, attr, name, runtime_attr, *id);
    }
    out = __hemx_lower_static_attr(out, "data-hemx-key", "data-key");
    out = __hemx_lower_static_attr(out, "h-key", "data-key");
    __hemx_inject_handle_inputs(out)
}

fn __hemx_replace_attr(mut html: ::std::string::String, attr: &str, name: &str, runtime_attr: &str, id: u32) -> ::std::string::String {
    let replacement = if runtime_attr == "value" {
        ::std::format!("{attr}=\"{name}\" {runtime_attr}=\"{id}\"")
    } else {
        ::std::format!("{runtime_attr}=\"{id}\"")
    };
    let double = ::std::format!("{attr}=\"{name}\"");
    html = html.replace(&double, &replacement);
    let single_replacement = replacement.replace('"', "'");
    let single = ::std::format!("{attr}='{name}'");
    html.replace(&single, &single_replacement)
}

fn __hemx_lower_static_attr(mut html: ::std::string::String, attr: &str, runtime_attr: &str) -> ::std::string::String {
    html = html.replace(&::std::format!("{attr}=\""), &::std::format!("{runtime_attr}=\""));
    html.replace(&::std::format!("{attr}='"), &::std::format!("{runtime_attr}='"))
}

fn __hemx_inject_handle_inputs(html: ::std::string::String) -> ::std::string::String {
    let mut out = ::std::string::String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<form") {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(open_end) = rest.find('>') else {
            out.push_str(rest);
            return out;
        };
        let opening = &rest[..=open_end];
        out.push_str(opening);
        rest = &rest[open_end + 1..];

        let Some(handle_id) = __hemx_attr(opening, "data-hid") else {
            continue;
        };
        let form_body_end = rest.find("</form>").unwrap_or(rest.len());
        let form_body = &rest[..form_body_end];
        if !__hemx_has_handle_input(form_body) {
            out.push_str(&::std::format!("<input type=\"hidden\" name=\"__h\" value=\"{}\">", handle_id));
        }
    }
    out.push_str(rest);
    out
}

fn __hemx_has_handle_input(html: &str) -> bool {
    html.contains("name=\"__h\"") || html.contains("name='__h'")
}

fn __hemx_attr(tag: &str, attr: &str) -> Option<::std::string::String> {
    let attr_at = tag.find(attr)?;
    let after_attr = &tag[attr_at + attr.len()..];
    let after_equals = after_attr.trim_start().strip_prefix('=')?.trim_start();
    let quote = after_equals.chars().next()?;
    if quote != '\"' && quote != '\'' {
        return None;
    }
    let value = &after_equals[quote.len_utf8()..];
    let end = value.find(quote)?;
    Some(value[..end].to_owned())
}
"#);
    }

    fn component_names(&self) -> Vec<String> {
        let mut components = Vec::new();
        for component in self
            .slots
            .values()
            .map(|res| &res.component)
            .chain(self.handles.values().map(|res| &res.component))
            .chain(self.atoms.values().map(|res| &res.component))
            .chain(self.forms.values().map(|form| &form.resource.component))
            .chain(self.classes.values().map(|class| &class.component))
            .chain(self.events.values().map(|event| &event.component))
        {
            if !components.contains(component) {
                components.push(component.clone());
            }
        }
        components.sort();
        components
    }

    fn client_bootstrap(&self) -> io::Result<String> {
        if self.client_handlers.is_empty() {
            return Ok(String::new());
        }
        let module = match self.client_modules.len() {
            1 => self.client_modules.iter().next().unwrap(),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "client-local handlers require one data-hemx-client-module on a hemx root",
                ));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "client-local handlers must share one data-hemx-client-module per generated application",
                ));
            }
        };
        let exports = self
            .client_handlers
            .iter()
            .map(|handler| format!("__hemx_client_{handler}"))
            .collect::<Vec<_>>();
        let mut out = format!(
            "import init, {{ {} }} from {};\nawait init();\n",
            exports.join(", "),
            serde_json::to_string(module).expect("serialize client module")
        );
        for handler in &self.client_handlers {
            out.push_str(&format!(
                "window.hemx.registerClientHandler({}, __hemx_client_{});\n",
                serde_json::to_string(handler).expect("serialize client handler"),
                handler
            ));
        }
        let fingerprint = hemx_core::BuildFingerprint::from_parts(&self.fingerprint_parts()).0;
        out.push_str(&format!(
            "document.querySelectorAll('[data-hemx-root]').forEach((root) => {{ root.setAttribute('data-hemx-build', '{}'); root.setAttribute('data-hemx-client-ready', ''); }});\n",
            fingerprint
        ));
        Ok(out)
    }

    fn syms(&self) -> String {
        let mut out = String::from("hemx-syms-v1\n");
        for res in self.slots.values() {
            out.push_str(&format!(
                "slot\t{}\t{}\t{}\n",
                res.symbol, res.ident, res.id
            ));
        }
        for res in self.handles.values() {
            out.push_str(&format!(
                "handle\t{}\t{}\t{}\n",
                res.symbol, res.ident, res.id
            ));
        }
        for form in self.forms.values() {
            let res = &form.resource;
            out.push_str(&format!(
                "form\t{}\t{}\t{}\n",
                res.symbol, res.ident, res.id
            ));
        }
        for (handle_ident, form_ident) in &self.handle_forms {
            out.push_str(&format!("handle_form\t{handle_ident}\t{form_ident}\n"));
        }
        for form in self.forms.values() {
            let mut fields = BTreeMap::<&str, (bool, bool)>::new();
            for control in &form.controls {
                let field = fields.entry(&control.name).or_default();
                field.0 |= control.required;
                field.1 |= form_control_is_multiple(&control.kind);
            }
            for (name, (required, multiple)) in fields {
                out.push_str(&format!(
                    "form_field\t{}\t{name}\t{required}\t{multiple}\n",
                    form.resource.ident
                ));
            }
        }
        for (handle_ident, params) in &self.handle_params {
            for param in params {
                out.push_str(&format!("handle_param\t{handle_ident}\t{param}\n"));
            }
        }
        for res in self.atoms.values() {
            out.push_str(&format!(
                "atom\t{}\t{}\t{}\n",
                res.symbol, res.ident, res.id
            ));
        }
        for class in self.classes.values() {
            out.push_str(&format!(
                "class\t{}\t{}\t{}\n",
                class.symbol, class.ident, class.token
            ));
        }
        for event in self.events.values() {
            out.push_str(&format!(
                "event\t{}\t{}\t{}\n",
                event.symbol, event.ident, event.name
            ));
        }
        out
    }

    fn fingerprint_parts(&self) -> Vec<u32> {
        let mut parts = vec![
            SURFACE_SCHEMA_VERSION,
            EFFECT_BATCH_ABI_VERSION,
            RUNTIME_ABI_VERSION,
        ];

        for res in self.slots.values() {
            parts.push(0);
            parts.push(res.id);
        }
        for res in self.handles.values() {
            parts.push(1);
            parts.push(res.id);
        }
        for res in self.atoms.values() {
            parts.push(3);
            parts.push(res.id);
        }
        for form in self.forms.values() {
            parts.push(2);
            parts.push(form.resource.id);
            parts.push(form.controls.len() as u32);
            for control in &form.controls {
                parts.push(stable_id("form-field", &control.name));
                parts.push(control.required as u32);
            }
        }

        parts
    }
}

fn insert_resource<'a>(
    map: &'a mut BTreeMap<String, Resource>,
    kind: &str,
    symbol: String,
    name: String,
    component: String,
) -> io::Result<&'a mut Resource> {
    let resource = make_resource(kind, symbol, name, component)?;
    match map.entry(resource.ident.clone()) {
        std::collections::btree_map::Entry::Occupied(entry)
            if entry.get().symbol != resource.symbol =>
        {
            let existing = entry.get();
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "duplicate generated identifier `{}` for `{}` and `{}`",
                    resource.ident, existing.symbol, resource.symbol
                ),
            ))
        }
        std::collections::btree_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
        std::collections::btree_map::Entry::Vacant(entry) => Ok(entry.insert(resource)),
    }
}

fn make_resource(
    kind: &str,
    symbol: String,
    name: String,
    component: String,
) -> io::Result<Resource> {
    let ident = rust_ident(&name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid hemx {kind} name `{name}`; expected a Rust identifier"),
        )
    })?;
    let id = stable_id(kind, &symbol);
    Ok(Resource {
        symbol,
        ident,
        component,
        keyed: false,
        id,
    })
}

fn collect_input_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !root.exists() {
        return Ok(paths);
    }
    collect_input_files_into(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_input_files_into(dir: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_input_files_into(&path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

fn static_attr(attrs: &[SurfaceAttribute], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.origin == AttributeOrigin::Static && attr.name == name)
        .and_then(|attr| attr.value.clone())
}

fn push_root_export(
    out: &mut String,
    pad: &str,
    module: &str,
    used: &mut BTreeSet<String>,
    ident: &str,
    suffix: &str,
) {
    if used.insert(ident.to_owned()) {
        out.push_str(&format!("{pad}pub use self::{module}::{ident};\n"));
        return;
    }
    let alias = format!("{ident}_{suffix}");
    if used.insert(alias.clone()) {
        out.push_str(&format!(
            "{pad}pub use self::{module}::{ident} as {alias};\n"
        ));
    }
}

fn component_matches(res: &Resource, component: Option<&str>) -> bool {
    match component {
        Some(component) => res.component == component,
        None => true,
    }
}

fn class_matches(class: &ClassToken, component: Option<&str>) -> bool {
    match component {
        Some(component) => class.component == component,
        None => true,
    }
}

fn event_matches(event: &EventToken, component: Option<&str>) -> bool {
    match component {
        Some(component) => event.component == component,
        None => true,
    }
}

fn class_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
}

fn event_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
}

fn stylesheet_class_tokens(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'.' {
            i += 1;
            continue;
        }
        let prev = i.checked_sub(1).map(|idx| bytes[idx]);
        if prev.is_some_and(|ch| ch == b'-' || ch == b'_' || ch.is_ascii_alphanumeric())
            && !preceded_by_class_in_compound(bytes, i)
        {
            i += 1;
            continue;
        }
        let start = i + 1;
        if start >= bytes.len() || !is_class_start(bytes[start]) {
            i += 1;
            continue;
        }
        let mut end = start;
        while end < bytes.len() && is_class_continue(bytes[end]) {
            end += 1;
        }
        if let Ok(token) = std::str::from_utf8(&bytes[start..end]) {
            tokens.push(token);
        }
        i = end;
    }
    tokens.sort_unstable();
    tokens.dedup();
    tokens
}

fn preceded_by_class_in_compound(bytes: &[u8], dot: usize) -> bool {
    let mut i = dot;
    while let Some(prev) = i.checked_sub(1) {
        let byte = bytes[prev];
        if byte == b'.' {
            return true;
        }
        if matches!(
            byte,
            b' ' | b'\n' | b'\r' | b'\t' | b',' | b'{' | b'}' | b'>' | b'+' | b'~' | b'(' | b')'
        ) {
            return false;
        }
        i = prev;
    }
    false
}

fn is_class_start(byte: u8) -> bool {
    byte == b'_' || byte == b'-' || byte.is_ascii_alphabetic()
}

fn is_class_continue(byte: u8) -> bool {
    is_class_start(byte) || byte.is_ascii_digit()
}

fn class_ident(token: &str) -> Option<String> {
    rust_ident(&token.replace('-', "_"))
}

fn is_handle_param_attr(attr: &SurfaceAttribute) -> bool {
    matches!(
        attr.origin,
        AttributeOrigin::Static | AttributeOrigin::Dynamic
    ) && attr.name.starts_with("data-")
        && !attr.name.starts_with("data-hemx-")
}

fn data_param_ident(name: &str) -> Option<String> {
    let data_name = name.strip_prefix("data-")?;
    rust_ident(&data_name.replace('-', "_"))
}

fn can_host_keyed_collection(tag: &str) -> bool {
    matches!(
        tag,
        "ul" | "ol" | "tbody" | "thead" | "tfoot" | "table" | "select" | "datalist"
    )
}

fn has_descendant_keyed_for_scope(surface: &SurfaceDocument, scope: ScopeId) -> bool {
    surface.scopes.iter().enumerate().any(|(index, current)| {
        matches!(
            current.kind,
            ScopeKind::For {
                key_expr: Some(_),
                ..
            }
        ) && is_descendant_scope(surface, ScopeId(index as u32), scope)
    })
}

fn is_descendant_scope(surface: &SurfaceDocument, mut scope: ScopeId, ancestor: ScopeId) -> bool {
    loop {
        let Some(current) = surface.scopes.get(scope.0 as usize) else {
            return false;
        };
        let Some(parent) = current.parent else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        scope = parent;
    }
}

fn is_inside_keyed_for(surface: &SurfaceDocument, mut scope: ScopeId) -> bool {
    loop {
        let Some(current) = surface.scopes.get(scope.0 as usize) else {
            return false;
        };
        if matches!(
            current.kind,
            ScopeKind::For {
                key_expr: Some(_),
                ..
            }
        ) {
            return true;
        }
        let Some(parent) = current.parent else {
            return false;
        };
        scope = parent;
    }
}

fn reject_selector_target_attrs(path: &Path, attrs: &[SurfaceAttribute]) -> io::Result<()> {
    for attr in attrs {
        let name = attr.name.as_str();
        if matches!(name, "data-hemx-target" | "data-hemx-select") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: `{name}` is selector-style targeting; hemx uses generated resources instead. Add data-hemx-slot to the local element and return an effect for that generated slot.",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn reject_unknown_hemx_attrs(path: &Path, attrs: &[SurfaceAttribute]) -> io::Result<()> {
    for attr in attrs {
        let name = attr.name.as_str();
        if name.starts_with("data-hemx-") && !known_hemx_attr(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: unknown hemx attribute `{name}`; check the spelling or use a non-hemx data-* attribute for app-specific metadata",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn known_hemx_attr(name: &str) -> bool {
    matches!(
        name,
        "data-hemx-root"
            | "data-hemx-sse"
            | "data-hemx-st"
            | "data-hemx-handle"
            | "data-hemx-slot"
            | "data-hemx-form"
            | "data-hemx-atom"
            | "data-hemx-key"
            | "data-hemx-on"
            | "data-hemx-client"
            | "data-hemx-client-event"
            | "data-hemx-client-fallback"
            | "data-hemx-client-module"
            | "data-hemx-client-policy"
            | "data-hemx-client-state-version"
            | "data-hemx-pending-class"
            | "data-hemx-indicator"
            | "data-hemx-confirm"
            | "data-hemx-debounce"
            | "data-hemx-delay"
            | "data-hemx-throttle"
            | "data-hemx-every"
            | "data-hemx-interval"
            | "data-hemx-revealed"
            | "data-hemx-revealed-ahead"
            | "data-hemx-disable-while-pending"
            | "data-hemx-policy"
            | "data-hemx-nav"
            | "data-hemx-history"
            | "data-hemx-boost"
            | "data-hemx-error-for"
            | "data-hemx-error"
            | "data-hemx-island"
    )
}

fn reject_invalid_hemx_attr_values(path: &Path, attrs: &[SurfaceAttribute]) -> io::Result<()> {
    for attr in attrs
        .iter()
        .filter(|attr| attr.origin == AttributeOrigin::Static)
    {
        let value = attr.value.as_deref().unwrap_or("");
        match attr.name.as_str() {
            "data-hemx-policy" if !valid_policy(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected one of `latest`, `queue`, `drop`, or `parallel`",
                ));
            }
            "data-hemx-history" if !matches!(value.trim(), "" | "push" | "replace") => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected `push`, `replace`, or empty for the default push behavior",
                ));
            }
            "data-hemx-client" if value.trim().is_empty() => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-empty client handler name",
                ));
            }
            "data-hemx-client-policy" if !matches!(value.trim(), "latest" | "drop") => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected `latest` or `drop`",
                ));
            }
            "data-hemx-client-module" if !valid_client_module(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a same-origin module specifier beginning with `/`, `./`, or `../`",
                ));
            }
            "data-hemx-client-event" if !valid_event_list(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a runtime-supported event",
                ));
            }
            "data-hemx-client-state-version"
                if value
                    .parse::<u32>()
                    .ok()
                    .filter(|version| *version > 0)
                    .is_none() =>
            {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a positive client state ABI version",
                ));
            }
            "data-hemx-on" if !valid_event_list(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected runtime-supported events: `click`, `submit`, `input`, `change`, `keydown`, `dragstart`, `dragover`, or `drop`",
                ));
            }
            "data-hemx-confirm" if value.trim().is_empty() => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-empty confirmation message",
                ));
            }
            "data-hemx-sse" if value.trim().is_empty() => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-empty same-origin SSE URL",
                ));
            }
            "data-hemx-revealed-ahead"
                if !value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .is_some_and(|value| value.is_finite() && value >= 0.0) =>
            {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-negative number of viewports",
                ));
            }
            "data-hemx-debounce" | "data-hemx-delay" | "data-hemx-throttle" | "data-hemx-every"
            | "data-hemx-interval"
                if !valid_duration(value) =>
            {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected milliseconds like `250`/`250ms` or seconds like `1s`",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn reject_invalid_hemx_attr_placement(
    path: &Path,
    tag: &str,
    attrs: &[SurfaceAttribute],
) -> io::Result<()> {
    if has_attr(attrs, "data-hemx-nav")
        && (tag != "a"
            || !has_attr(attrs, "href")
            || static_attr(attrs, "href").is_some_and(|href| href.trim().is_empty()))
    {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-nav",
            "expected a real `<a href=...>` link so navigation works without JavaScript",
        ));
    }
    if has_attr(attrs, "data-hemx-boost") && matches!(tag, "a" | "form") {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-boost",
            "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
        ));
    }
    if has_attr(attrs, "data-hemx-client-module") && !has_attr(attrs, "data-hemx-root") {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-client-module",
            "expected placement on the same element as `data-hemx-root`",
        ));
    }
    if has_attr(attrs, "data-hemx-sse") && !has_attr(attrs, "data-hemx-root") {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-sse",
            "expected placement on the same element as `data-hemx-root`",
        ));
    }
    Ok(())
}

fn has_attr(attrs: &[SurfaceAttribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.name == name)
}

fn invalid_hemx_placement(path: &Path, attr: &str, expectation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{}: invalid {attr} placement; {expectation}",
            path.display()
        ),
    )
}

fn valid_policy(value: &str) -> bool {
    matches!(value.trim(), "latest" | "queue" | "drop" | "parallel")
}

fn valid_client_module(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.starts_with("//")
        && !value.contains(':')
        && (value.starts_with('/') || value.starts_with("./") || value.starts_with("../"))
}

fn valid_event_list(value: &str) -> bool {
    let mut events = event_tokens(value).peekable();
    events.peek().is_some() && events.all(valid_runtime_event)
}

fn valid_runtime_event(value: &str) -> bool {
    matches!(
        value,
        "click" | "submit" | "input" | "change" | "keydown" | "dragstart" | "dragover" | "drop"
    )
}

fn valid_duration(value: &str) -> bool {
    let value = value.trim();
    let digits = value
        .strip_suffix("ms")
        .or_else(|| value.strip_suffix('s'))
        .unwrap_or(value);
    !digits.is_empty() && digits.as_bytes().iter().all(u8::is_ascii_digit)
}

fn invalid_hemx_value(path: &Path, attr: &str, value: &str, expectation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{}: invalid {attr} value `{value}`; {expectation}",
            path.display()
        ),
    )
}

fn reject_unkeyed_loop(
    surface: &SurfaceDocument,
    scope: ScopeId,
    path: &Path,
    kind: &str,
    name: &str,
) -> io::Result<()> {
    if let Some(diagnostic) =
        unkeyed_generated_target_diagnostic_for_scope(surface, scope, path, kind, name)
    {
        Err(diagnostic.to_io_error())
    } else {
        Ok(())
    }
}

fn context_type_for_heml_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut out = String::new();
    for word in stem.split(['_', '-']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        out.extend(chars.next()?.to_uppercase());
        out.extend(chars);
    }
    (!out.is_empty()).then_some(out)
}

fn nearest_dir_with(path: &Path, file_name: &str) -> Option<PathBuf> {
    for dir in path.ancestors().filter(|path| path.is_dir()) {
        if dir.join(file_name).is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn rust_struct_facts_in(root: &Path) -> io::Result<HashMap<String, RustStructFact>> {
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

fn collect_rust_struct_facts_from_file(
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

fn compact_tokens(tokens: &impl ToTokens) -> String {
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

fn loop_locals_for_surface(
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

fn h_for_local_and_self_field(value: &str) -> Option<(String, String)> {
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

fn vec_element_type(type_name: &str) -> Option<String> {
    let inner = type_name
        .strip_prefix("Vec<")
        .or_else(|| type_name.strip_prefix("std::vec::Vec<"))?
        .strip_suffix('>')?;
    Some(inner.trim().to_owned())
}

fn unkeyed_generated_target_diagnostics(path: &Path, surface: &SurfaceDocument) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (node_scope, target) in generated_targets_with_scope(surface) {
        if let Some(diagnostic) = unkeyed_generated_target_diagnostic_for_scope(
            surface,
            node_scope,
            path,
            &target.kind,
            &target.name,
        ) {
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

fn generated_targets(surface: &SurfaceDocument) -> Vec<GeneratedTarget> {
    let mut targets = Vec::new();
    for (_, target) in generated_targets_with_scope(surface) {
        if !targets.iter().any(|existing: &GeneratedTarget| {
            existing.kind == target.kind && existing.name == target.name
        }) {
            targets.push(target);
        }
    }
    targets
}

fn generated_targets_with_scope(surface: &SurfaceDocument) -> Vec<(ScopeId, GeneratedTarget)> {
    let mut targets = Vec::new();
    for node in &surface.nodes {
        for attr in &node.attrs {
            let Some(kind) = attr.name.strip_prefix("data-hemx-") else {
                continue;
            };
            if !matches!(kind, "slot" | "form" | "handle") {
                continue;
            }
            let Some(name) = &attr.value else {
                continue;
            };
            targets.push((
                node.scope,
                GeneratedTarget {
                    kind: kind.to_owned(),
                    name: name.to_owned(),
                },
            ));
        }
    }
    targets
}

fn unkeyed_generated_target_diagnostic_for_scope(
    surface: &SurfaceDocument,
    mut scope: ScopeId,
    path: &Path,
    kind: &str,
    name: &str,
) -> Option<Diagnostic> {
    loop {
        let current = surface.scopes.get(scope.0 as usize)?;
        if let ScopeKind::For {
            pattern,
            expr,
            key_expr: None,
        } = &current.kind
        {
            return Some(unkeyed_generated_target_diagnostic(
                path, kind, name, pattern, expr,
            ));
        }
        let parent = current.parent?;
        scope = parent;
    }
}

fn unkeyed_generated_target_diagnostic(
    path: &Path,
    kind: &str,
    name: &str,
    pattern: &str,
    expr: &str,
) -> Diagnostic {
    Diagnostic {
        code: DiagnosticCode::UnkeyedGeneratedTarget,
        severity: DiagnosticSeverity::Error,
        file: path.to_path_buf(),
        directive: format!("data-hemx-{kind}"),
        target: name.to_owned(),
        message: format!(
            "data-hemx-{kind}=\"{name}\" is inside h-for=\"{pattern} in {expr}\" without h-key"
        ),
        expected: format!(
            "a stable template h-key on h-for=\"{pattern} in {expr}\" so generated keyed helpers such as ui::{name}.replace(row) can target this partial"
        ),
        repair: format!(
            "add h-key=\"{pattern}.id\" to that h-for; dynamic +data-key on the child is rendered HTML, not the template fact hemx uses for generated targets"
        ),
    }
}

fn canonical_symbol(root: &Path, path: &Path, name: &str) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    format!("{}::{name}", rel.to_string_lossy().replace('\\', "/"))
}

fn component_ident(root: &Path, path: &Path) -> io::Result<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let stem = rel
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid template path `{}`", path.display()),
            )
        })?;
    rust_ident(stem).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid template name `{stem}`; expected a Rust module identifier"),
        )
    })
}

fn rust_ident(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    if chars
        .clone()
        .any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
    {
        return None;
    }
    Some(name.to_string())
}

fn stable_id(kind: &str, symbol: &str) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in kind.bytes().chain(*b":").chain(symbol.bytes()) {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn form_control_is_multiple(kind: &ControlKind) -> bool {
    matches!(kind, ControlKind::Select { multiple: true })
}

fn form_control_kind_expr(kind: &ControlKind) -> String {
    match kind {
        ControlKind::Text => "::hemx::FormControlKind::Text".to_string(),
        ControlKind::Number { min, max, step } => format!(
            "::hemx::FormControlKind::Number {{ min: {}, max: {}, step: {} }}",
            rust_str_opt(min.as_deref()),
            rust_str_opt(max.as_deref()),
            rust_str_opt(step.as_deref())
        ),
        ControlKind::Checkbox => "::hemx::FormControlKind::Checkbox".to_string(),
        ControlKind::Radio => "::hemx::FormControlKind::Radio".to_string(),
        ControlKind::Select { multiple } => {
            format!("::hemx::FormControlKind::Select {{ multiple: {multiple} }}")
        }
        ControlKind::TextArea => "::hemx::FormControlKind::TextArea".to_string(),
        ControlKind::File => "::hemx::FormControlKind::File".to_string(),
        ControlKind::Hidden => "::hemx::FormControlKind::Hidden".to_string(),
        ControlKind::Submit => "::hemx::FormControlKind::Submit".to_string(),
        ControlKind::Other { tag, input_type } => format!(
            "::hemx::FormControlKind::Other {{ tag: {}, input_type: {} }}",
            rust_str(tag),
            rust_str_opt(input_type.as_deref())
        ),
    }
}

fn rust_str(value: &str) -> String {
    format!("{value:?}")
}

fn rust_str_opt(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("Some({})", rust_str(value)),
        None => "None".to_string(),
    }
}

fn parse_error(path: &Path, err: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("failed to parse {}: {err}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    static OUT_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn public_heml_inspection_entry_points_preserve_paths_and_io_diagnostics() {
        let root = std::env::temp_dir().join(format!(
            "hemx-build-inspection-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::create_dir_all(&root).unwrap();
        let template = root.join("card.heml");
        let source = r#"<template h-for="todo in &self.todos"><button data-hemx-handle="save">Save</button></template>"#;
        std::fs::write(&template, source).unwrap();

        let from_source = diagnostics_for_heml_source(&template, source.to_owned()).unwrap();
        let from_file = diagnostics_for_heml_file(&template).unwrap();
        assert_eq!(from_file, from_source);
        assert_eq!(from_source.len(), 1);
        assert_eq!(from_source[0].file, template);
        assert_eq!(from_source[0].directive, "data-hemx-handle");

        let targets = generated_targets_for_heml_source(
            &template,
            r#"<main data-hemx-root="app"><section h-slot="notice"></section><button data-hemx-handle="save">Save</button></main>"#,
        )
        .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "save");
        assert_eq!(
            template_context_facts_for_heml_source(
                &template,
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap(),
            None
        );
        let crate_root = root.join("crate");
        let templates = crate_root.join("templates");
        std::fs::create_dir_all(crate_root.join("src")).unwrap();
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            crate_root.join("Cargo.toml"),
            "[package]\nname='facts'\nversion='0.0.0'\n",
        )
        .unwrap();
        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[derive(Hemplate)] struct Profile { title: String }\n#[derive(Clone, hemplate::Hemplate)] struct ProfileCard { card_title: String }\n#[derive(NotHemplate)] struct ProfileDetails { hidden: String }\n#[derive(HemplateExtra)] struct Extra { hidden: String }\nmod inline { #[derive(Hemplate)] pub struct InlineProfile { pub inline_title: String } }\nmod external;\nstruct Plain { title: String }",
        )
        .unwrap();
        std::fs::create_dir_all(crate_root.join("src/nested")).unwrap();
        std::fs::write(
            crate_root.join("src/nested/profile.rs"),
            "#[derive(Hemplate)] struct NestedProfile { nested_title: String }",
        )
        .unwrap();
        std::fs::write(crate_root.join("src/external.rs"), "struct External;").unwrap();
        std::fs::write(crate_root.join("src/ignored.txt"), [0xff]).unwrap();
        std::fs::write(
            crate_root.join("outside.rs"),
            "#[derive(Hemplate)] struct OutsideProfile { outside: String }",
        )
        .unwrap();
        let profile = templates.join("profile.heml");
        let facts = template_context_facts_for_heml_source(
            &profile,
            "<main>{{ self.title }}</main>".to_owned(),
        )
        .unwrap()
        .expect("Hemlate-derived context facts");
        assert_eq!(facts.context_type, "Profile");
        assert_eq!(facts.self_fields[0].name, "title");

        for (template, context_type, field) in [
            ("inline_profile.heml", "InlineProfile", "inline_title"),
            ("nested_profile.heml", "NestedProfile", "nested_title"),
        ] {
            let nested_facts = template_context_facts_for_heml_source(
                templates.join(template),
                format!("<main>{{{{ self.{field} }}}}</main>"),
            )
            .unwrap()
            .expect("recursive Rust struct fact");
            assert_eq!(nested_facts.context_type, context_type);
            assert_eq!(nested_facts.self_fields[0].name, field);
        }
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("outside_profile.heml"),
                "<main>{{ self.outside }}</main>".to_owned(),
            )
            .unwrap(),
            None,
            "Rust facts outside src must not become template authority"
        );

        let card_facts = template_context_facts_for_heml_source(
            templates.join("profile-card.heml"),
            "<main>{{ self.card_title }}</main>".to_owned(),
        )
        .unwrap()
        .expect("qualified Hemlate derive");
        assert_eq!(card_facts.context_type, "ProfileCard");
        assert_eq!(card_facts.self_fields[0].name, "card_title");
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("profile_details.heml"),
                "<main>{{ self.hidden }}</main>".to_owned(),
            )
            .unwrap(),
            None,
            "derive names containing Hemlate must not grant context authority"
        );
        assert_eq!(
            context_type_for_heml_path(Path::new("--profile__card--.heml")),
            Some("ProfileCard".into())
        );
        assert_eq!(context_type_for_heml_path(Path::new("---.heml")), None);
        assert_eq!(context_type_for_heml_path(Path::new("")), None);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                context_type_for_heml_path(Path::new(std::ffi::OsStr::from_bytes(b"\xff.heml"))),
                None
            );
        }

        std::fs::write(crate_root.join("src/broken.rs"), [0xff]).unwrap();
        assert_eq!(
            template_context_facts_for_heml_source(
                &profile,
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_file(crate_root.join("src/broken.rs")).unwrap();
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("plain.heml"),
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap(),
            None
        );
        let missing = root.join("missing.heml");
        for error in [
            diagnostics_for_heml_file(&missing).unwrap_err(),
            template_context_facts_for_heml_file(&missing).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rust_type_facts_are_compact_and_vector_specific() {
        let qualified: syn::Type =
            syn::parse_str("std::collections::HashMap<String, &crate::Thing<Left, Right>>")
                .unwrap();
        assert_eq!(
            compact_tokens(&qualified),
            "std::collections::HashMap<String,&crate::Thing<Left,Right>>"
        );
        let mutable: syn::Type = syn::parse_str("&mut crate::Thing").unwrap();
        assert_eq!(compact_tokens(&mutable), "&mut crate::Thing");
        let tuple: syn::Type = syn::parse_str("(&crate::Thing, String)").unwrap();
        assert_eq!(compact_tokens(&tuple), "(&crate::Thing,String)");
        let function: syn::Type = syn::parse_str("fn(&crate::Thing) -> &crate::Thing").unwrap();
        assert_eq!(
            compact_tokens(&function),
            "fn (&crate::Thing) ->&crate::Thing"
        );

        assert_eq!(vec_element_type("Vec<String>"), Some("String".into()));
        assert_eq!(
            vec_element_type("std::vec::Vec<&crate::Thing<Left,Right>>"),
            Some("&crate::Thing<Left,Right>".into())
        );
        assert_eq!(vec_element_type("Vec< String >"), Some("String".into()));
        for invalid in [
            "",
            "String",
            "Vec<String",
            "VecString>",
            "alloc::vec::Vec<String>",
        ] {
            assert_eq!(
                vec_element_type(invalid),
                None,
                "accepted non-vector type {invalid:?}"
            );
        }
    }

    #[test]
    fn rust_fact_collection_propagates_parse_and_recursive_directory_errors() {
        let base = test_dir("hemx-build-rust-fact-errors");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        let mut facts = HashMap::new();
        std::fs::write(base.join("src/broken.rs"), "struct {").unwrap();
        assert_eq!(
            collect_rust_struct_facts_from_file(&base.join("src/broken.rs"), &mut facts)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(facts.is_empty());
        std::fs::remove_file(base.join("src/broken.rs")).unwrap();

        assert!(rust_struct_facts_in(&base.join("missing"))
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn identifier_and_literal_helpers_fail_closed_at_boundaries() {
        assert_eq!(rust_ident("alpha_9"), Some("alpha_9".into()));
        for invalid in ["", "9alpha", "alpha-beta", "alpha beta", "alphaé"] {
            assert_eq!(
                rust_ident(invalid),
                None,
                "accepted Rust identifier {invalid:?}"
            );
        }
        assert_eq!(class_ident("alpha-beta_9"), Some("alpha_beta_9".into()));
        for invalid in ["", "9alpha", "alphaé", "alpha.beta", "alpha beta"] {
            assert_eq!(class_ident(invalid), None, "accepted CSS class {invalid:?}");
        }

        assert_eq!(rust_str("a\n\"b\\c"), "\"a\\n\\\"b\\\\c\"");
        assert_eq!(rust_str_opt(None), "None");
        assert_eq!(rust_str_opt(Some("a\n\"b\\c")), "Some(\"a\\n\\\"b\\\\c\")");
        assert_eq!(
            canonical_symbol(
                Path::new("templates"),
                Path::new("templates/nested/panel.heml"),
                "save",
            ),
            "nested/panel.heml::save"
        );

        let root = Path::new("templates");
        assert_eq!(
            component_ident(root, Path::new("templates/nested/profile_card.heml")).unwrap(),
            "profile_card"
        );
        assert_eq!(
            component_ident(root, Path::new("templates/9bad.heml"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            component_ident(root, Path::new("outside.heml")).unwrap(),
            "outside"
        );
        assert_eq!(data_param_ident("data-item-id"), Some("item_id".into()));
        assert_eq!(data_param_ident("aria-label"), None);
        assert_eq!(app().template_dir, PathBuf::from("templates"));
        assert_eq!(nearest_dir_with(Path::new("/"), "hemx-absent-marker"), None);

        let mut exports = String::new();
        let mut used = BTreeSet::from(["shared".to_owned(), "shared_target".to_owned()]);
        push_root_export(&mut exports, "", "targets", &mut used, "shared", "target");
        assert!(
            exports.is_empty(),
            "colliding alias must not be emitted twice"
        );

        let mut resources = BTreeMap::new();
        let first = insert_resource(
            &mut resources,
            "slot",
            "panel.heml::summary".into(),
            "summary".into(),
            "panel".into(),
        )
        .unwrap();
        first.keyed = true;
        assert!(
            insert_resource(
                &mut resources,
                "slot",
                "panel.heml::summary".into(),
                "summary".into(),
                "panel".into(),
            )
            .unwrap()
            .keyed
        );
        assert_eq!(
            insert_resource(
                &mut resources,
                "slot",
                "other.heml::summary".into(),
                "summary".into(),
                "other".into(),
            )
            .unwrap_err()
            .to_string(),
            "duplicate generated identifier `summary` for `panel.heml::summary` and `other.heml::summary`"
        );

        let invalid_surface = surface_for_heml_source(
            Path::new("123.heml"),
            r#"<section data-hemx-slot="summary"></section>"#.to_owned(),
        )
        .unwrap();
        let mut extracted = Resources::default();
        assert_eq!(
            extracted
                .add_surface(
                    Path::new("templates"),
                    Path::new("templates/123.heml"),
                    &invalid_surface,
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        for (kind, source) in [
            ("atom", r#"<span data-hemx-atom="shared"></span>"#),
            ("handle", r#"<button data-hemx-handle="shared"></button>"#),
        ] {
            let first =
                surface_for_heml_source(Path::new("first.heml"), source.to_owned()).unwrap();
            let second =
                surface_for_heml_source(Path::new("second.heml"), source.to_owned()).unwrap();
            let mut duplicate = Resources::default();
            duplicate
                .add_surface(
                    Path::new("templates"),
                    Path::new("templates/first.heml"),
                    &first,
                )
                .unwrap();
            assert_eq!(
                duplicate
                    .add_surface(
                        Path::new("templates"),
                        Path::new("templates/second.heml"),
                        &second,
                    )
                    .unwrap_err()
                    .to_string(),
                format!(
                    "duplicate generated identifier `shared` for `first.heml::shared` and `second.heml::shared`"
                ),
                "{kind} collision must propagate"
            );
        }

        let mut form_without_controls = surface_for_heml_source(
            Path::new("form.heml"),
            r#"<form data-hemx-handle="save" data-hemx-form="empty"></form>"#.to_owned(),
        )
        .unwrap();
        form_without_controls.forms.clear();
        let mut form_resources = Resources::default();
        form_resources
            .add_surface(
                Path::new("templates"),
                Path::new("templates/form.heml"),
                &form_without_controls,
            )
            .unwrap();
        assert!(form_resources.forms["empty"].controls.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let invalid =
                PathBuf::from("templates").join(std::ffi::OsStr::from_bytes(b"\xff.heml"));
            assert_eq!(
                component_ident(root, &invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn generated_contract_fingerprint_and_client_bootstrap_are_deterministic() {
        let resource = |symbol: &str, component: &str, id| Resource {
            symbol: symbol.into(),
            ident: symbol.into(),
            component: component.into(),
            keyed: false,
            id,
        };
        let mut resources = Resources::default();
        resources
            .slots
            .insert("slot".into(), resource("slot", "page", 11));
        resources
            .handles
            .insert("save".into(), resource("save", "page", 12));
        resources
            .atoms
            .insert("count".into(), resource("count", "page", 13));
        resources.forms.insert(
            "profile".into(),
            FormResource {
                resource: resource("profile", "page", 14),
                controls: vec![
                    GeneratedControl {
                        name: "email".into(),
                        kind: ControlKind::Text,
                        required: true,
                    },
                    GeneratedControl {
                        name: "nickname".into(),
                        kind: ControlKind::Text,
                        required: false,
                    },
                ],
            },
        );

        let expected_parts = vec![
            SURFACE_SCHEMA_VERSION,
            EFFECT_BATCH_ABI_VERSION,
            RUNTIME_ABI_VERSION,
            0,
            11,
            1,
            12,
            3,
            13,
            2,
            14,
            2,
            stable_id("form-field", "email"),
            1,
            stable_id("form-field", "nickname"),
            0,
        ];
        assert_eq!(resources.fingerprint_parts(), expected_parts);

        let generated = resources.generated_rs(false);
        assert!(generated.starts_with("// @generated by hemx-build. Do not edit.\n"));
        assert!(generated.contains("pub const BUILD_FINGERPRINT"));
        assert!(generated.contains(
            "\n#[allow(non_upper_case_globals)]\npub mod page {\n    #[derive(Clone, Copy)]\n    pub struct Component;\n"
        ));
        assert!(generated.contains("pub const slot: SlotTarget"));
        assert!(generated.contains("pub const save: ::hemx::Handle"));
        assert!(generated.contains("pub const count: ::hemx::Atom"));
        assert!(generated.contains("pub const profile: ::hemx::Form"));
        assert!(generated.contains("\n    pub mod targets {\n"));

        let generated_with_globals = resources.generated_rs(true);
        assert!(generated_with_globals.contains("pub mod advanced {"));
        assert!(generated_with_globals.contains("pub mod slots {"));

        let mut row_slot = resource("row", "row", 21);
        row_slot.keyed = true;
        resources.slots.insert("row".into(), row_slot);
        resources
            .handles
            .insert("edit".into(), resource("edit", "row", 22));
        resources
            .atoms
            .insert("selection".into(), resource("selection", "row", 23));
        resources
            .slots
            .insert("child".into(), resource("child", "page", 24));
        resources
            .handles
            .insert("child_handle".into(), resource("child_handle", "child", 25));
        resources
            .slots
            .insert("shared".into(), resource("shared", "page", 26));
        resources
            .handles
            .insert("shared".into(), resource("shared", "page", 27));
        resources.forms.insert(
            "shared".into(),
            FormResource {
                resource: resource("shared", "page", 28),
                controls: Vec::new(),
            },
        );
        for (symbol, ident, component, token) in [
            ("a::card", "card", "page", "card"),
            ("b::card", "card", "row", "card"),
            ("z::last", "last", "page", "last"),
        ] {
            resources.classes.insert(
                symbol.into(),
                ClassToken {
                    symbol: symbol.into(),
                    ident: ident.into(),
                    component: component.into(),
                    token: token.into(),
                },
            );
        }
        for (component, name) in [("page", "click"), ("row", "click"), ("z", "submit")] {
            resources.events.insert(
                format!("{component}::{name}"),
                EventToken {
                    symbol: format!("{component}::{name}"),
                    ident: name.into(),
                    component: component.into(),
                    name: name.into(),
                },
            );
        }
        resources
            .handle_forms
            .insert("save".into(), "profile".into());
        resources
            .handle_params
            .entry("save".into())
            .or_default()
            .insert("item_id".into());
        let canonical_generated = resources.generated_rs(false);
        let canonical_globals = resources.generated_rs(true);
        let canonical_syms = resources.syms();
        assert_eq!(
            stable_id("generated-rs", &canonical_generated),
            2_620_423_950,
            "canonical generated Rust changed"
        );
        assert_eq!(
            stable_id("generated-rs-global", &canonical_globals),
            2_559_847_213,
            "canonical global-export Rust changed"
        );
        assert_eq!(
            stable_id("generated-syms", &canonical_syms),
            984_222_700,
            "canonical symbol manifest changed"
        );

        assert_eq!(Resources::default().client_bootstrap().unwrap(), "");
        resources
            .client_handlers
            .extend(["save".into(), "toggle".into()]);
        assert_eq!(
            resources.client_bootstrap().unwrap_err().to_string(),
            "client-local handlers require one data-hemx-client-module on a hemx root"
        );
        resources.client_modules.insert("/app.wasm".into());
        let bootstrap = resources.client_bootstrap().unwrap();
        assert!(bootstrap.starts_with(
            "import init, { __hemx_client_save, __hemx_client_toggle } from \"/app.wasm\";\nawait init();\n"
        ));
        assert!(
            bootstrap.contains("window.hemx.registerClientHandler(\"save\", __hemx_client_save);")
        );
        assert!(bootstrap.contains("data-hemx-client-ready"));
        resources.client_modules.insert("/other.wasm".into());
        assert_eq!(
            resources.client_bootstrap().unwrap_err().to_string(),
            "client-local handlers must share one data-hemx-client-module per generated application"
        );
    }

    #[test]
    fn app_builder_propagates_output_and_client_contract_errors_without_panicking() {
        let root = std::env::temp_dir().join(format!(
            "hemx-build-error-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let out = root.join("out");
        std::fs::create_dir_all(out.join("hemx.generated.rs")).unwrap();

        let result = std::panic::catch_unwind(|| app().out_dir(&out).run());
        assert!(result.is_ok(), "ordinary output errors must not panic");
        assert!(result.unwrap().is_err());

        std::fs::remove_dir_all(out.join("hemx.generated.rs")).unwrap();
        std::fs::create_dir_all(out.join("hemx.syms")).unwrap();
        let result = std::panic::catch_unwind(|| app().out_dir(&out).run());
        assert!(result.is_ok(), "symbol output errors must not panic");
        assert!(result.unwrap().is_err());

        std::fs::remove_dir_all(out.join("hemx.syms")).unwrap();
        std::fs::create_dir_all(out.join("hemx.client.js")).unwrap();
        let result = std::panic::catch_unwind(|| app().out_dir(&out).run());
        assert!(result.is_ok(), "client output errors must not panic");
        assert!(result.unwrap().is_err());

        let templates = root.join("templates");
        let invalid_out = root.join("invalid-out");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("client.heml"),
            "<button data-hemx-client=\"save\">Save</button>",
        )
        .unwrap();
        let result =
            std::panic::catch_unwind(|| app().template_dir(&templates).out_dir(&invalid_out).run());
        assert!(result.is_ok(), "invalid client contracts must not panic");
        assert_eq!(
            result.unwrap().unwrap_err().to_string(),
            "client-local handlers require one data-hemx-client-module on a hemx root"
        );

        let invalid_handler = templates.join("invalid_handler.heml");
        std::fs::write(
            &invalid_handler,
            r#"<main data-hemx-root="app" data-hemx-client-module="/app.js"><button data-hemx-client="save-item">Save</button></main>"#,
        )
        .unwrap();
        std::fs::remove_file(templates.join("client.heml")).unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(&invalid_out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            format!(
                "{}: invalid data-hemx-client value `save-item`; expected a Rust handler identifier",
                invalid_handler.display()
            )
        );

        std::fs::remove_file(invalid_handler).unwrap();
        std::fs::write(
            templates.join("invalid_param.heml"),
            r#"<button data-hemx-handle="save" data-123="value">Save</button>"#,
        )
        .unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(&invalid_out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "invalid handler param attribute `data-123`; expected data-* name usable from Rust"
        );

        std::fs::remove_file(templates.join("invalid_param.heml")).unwrap();
        std::fs::write(
            templates.join("invalid_slot.heml"),
            r#"<section data-hemx-slot="123">Invalid</section>"#,
        )
        .unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(&invalid_out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "invalid hemx slot name `123`; expected a Rust identifier"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn app_builder_discovers_nested_inputs_and_propagates_collection_errors() {
        let base = test_dir("hemx-build-collection");
        let templates = base.join("templates");
        let nested = templates.join("nested");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(templates.join("a_empty.heml"), "").unwrap();
        std::fs::write(
            nested.join("panel.heml"),
            r#"<section data-hemx-slot="nested_panel"></section>"#,
        )
        .unwrap();
        std::fs::write(nested.join("panel.css"), ".nested-css {}").unwrap();
        std::fs::write(nested.join("theme.scss"), ".nested-scss {}").unwrap();
        std::fs::write(nested.join("ignored.txt"), [0xff]).unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("slot\tnested/panel.heml::nested_panel\tnested_panel\t"));
        assert!(syms.contains("class\tnested/panel.css::nested-css\tnested_css\tnested-css\n"));
        assert!(syms.contains("class\tnested/theme.scss::nested-scss\tnested_scss\tnested-scss\n"));

        let missing = base.join("missing");
        let missing_out = base.join("missing-out");
        app()
            .template_dir(&missing)
            .out_dir(&missing_out)
            .run()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(missing_out.join("hemx.syms")).unwrap(),
            "hemx-syms-v1\n"
        );

        let _out_dir_lock = OUT_DIR_LOCK.lock().unwrap();
        let previous_out_dir = std::env::var_os("OUT_DIR");
        let env_out = base.join("env-out");
        std::env::set_var("OUT_DIR", &env_out);
        let env_result = app().template_dir(&missing).run();
        match previous_out_dir {
            Some(value) => std::env::set_var("OUT_DIR", value),
            None => std::env::remove_var("OUT_DIR"),
        }
        env_result.unwrap();
        assert!(env_out.join("hemx.generated.rs").is_file());

        let template_file = base.join("not-a-directory");
        std::fs::write(&template_file, "not a directory").unwrap();
        let error = app()
            .template_dir(&template_file)
            .out_dir(base.join("file-root-out"))
            .run()
            .unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::NotADirectory | io::ErrorKind::Other
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let restricted_root = base.join("restricted-root");
            let restricted_nested = restricted_root.join("nested");
            std::fs::create_dir_all(&restricted_nested).unwrap();
            std::fs::set_permissions(&restricted_nested, std::fs::Permissions::from_mode(0o000))
                .unwrap();
            let error = app()
                .template_dir(&restricted_root)
                .out_dir(base.join("restricted-out"))
                .run()
                .unwrap_err();
            std::fs::set_permissions(&restricted_nested, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }

        let invalid_heml = base.join("invalid-heml");
        std::fs::create_dir_all(&invalid_heml).unwrap();
        std::fs::write(invalid_heml.join("bad.heml"), [0xff]).unwrap();
        assert_eq!(
            app()
                .template_dir(&invalid_heml)
                .out_dir(base.join("invalid-heml-out"))
                .run()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let invalid_css = base.join("invalid-css");
        std::fs::create_dir_all(&invalid_css).unwrap();
        std::fs::write(invalid_css.join("bad.css"), [0xff]).unwrap();
        assert_eq!(
            app()
                .template_dir(&invalid_css)
                .out_dir(base.join("invalid-css-out"))
                .run()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let blocked_out = base.join("blocked-out");
        std::fs::write(&blocked_out, "not a directory").unwrap();
        assert!(app()
            .template_dir(&missing)
            .out_dir(&blocked_out)
            .run()
            .is_err());

        let explicit_surface = surface_for_heml_source(
            Path::new("explicit.heml"),
            r#"<section data-hemx-slot="123"></section>"#.to_owned(),
        )
        .unwrap();
        assert_eq!(
            app()
                .surface("explicit.heml", explicit_surface)
                .out_dir(base.join("explicit-out"))
                .run()
                .unwrap_err()
                .to_string(),
            "invalid hemx slot name `123`; expected a Rust identifier"
        );

        let ordered = base.join("ordered");
        std::fs::create_dir_all(&ordered).unwrap();
        std::fs::write(ordered.join("a.css"), ".foo-bar {}").unwrap();
        std::fs::write(ordered.join("b.css"), ".foo_bar {}").unwrap();
        assert_eq!(
            app()
                .template_dir(&ordered)
                .out_dir(base.join("ordered-out"))
                .run()
                .unwrap_err()
                .to_string(),
            "duplicate generated class identifier `foo_bar` for CSS classes `foo-bar` and `foo_bar`"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn no_op_build_preserves_generated_artifact_timestamps() {
        let base = test_dir("hemx-build-no-op");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("panel.heml"),
            r#"<button data-hemx-handle="save">Save</button>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let generated = out.join("hemx.generated.rs");
        let symbols = out.join("hemx.syms");
        let generated_modified = std::fs::metadata(&generated).unwrap().modified().unwrap();
        let symbols_modified = std::fs::metadata(&symbols).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        app().template_dir(&templates).out_dir(&out).run().unwrap();

        assert_eq!(
            std::fs::metadata(generated).unwrap().modified().unwrap(),
            generated_modified,
            "no-op codegen must not invalidate downstream Rust compilation"
        );
        assert_eq!(
            std::fs::metadata(symbols).unwrap().modified().unwrap(),
            symbols_modified,
            "no-op symbol generation must preserve its artifact timestamp"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn changed_template_refreshes_generated_artifacts() {
        let base = test_dir("hemx-build-refresh");
        let templates = base.join("templates");
        let out = base.join("out");
        let template = templates.join("panel.heml");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            &template,
            r#"<button data-hemx-handle="save">Save</button>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let initial_generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        let initial_symbols = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(initial_generated.contains("pub const save"));
        assert!(initial_symbols.contains("save"));

        std::fs::write(
            &template,
            r#"<button data-hemx-handle="cancel">Cancel</button>"#,
        )
        .unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let refreshed_generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        let refreshed_symbols = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert_ne!(refreshed_generated, initial_generated);
        assert_ne!(refreshed_symbols, initial_symbols);
        assert!(refreshed_generated.contains("pub const cancel"));
        assert!(!refreshed_generated.contains("pub const save"));
        assert!(refreshed_symbols.contains("cancel"));
        assert!(!refreshed_symbols.contains("save"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_generated_resources_from_heml() {
        let base = test_dir("hemx-build-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<form class="todo-card" data-hemx-handle="create" data-hemx-form="new_todo"><input name="title" required><select name="labels" multiple><option value="urgent">Urgent</option></select><input name="__h" value="reserved"></form><button data-hemx-handle="delete" data-todo-id="7" data-hemx-on="click change">Delete</button><ul data-hemx-slot="todos"></ul><section data-hemx-atom="filter"></section>"#,
        )
        .unwrap();
        std::fs::write(templates.join("todo.css"), ".todo-card { display: block; }").unwrap();

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .global_exports(true)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod components"));
        assert!(generated.contains(
            "pub const todo: ::hemx::ComponentRef = ::hemx::ComponentRef::new(\"todo\")"
        ));
        assert!(generated.contains("pub mod advanced"));
        assert!(generated.contains("pub mod slots"));
        assert!(generated.contains("pub const todos"));
        assert!(generated.contains("pub mod targets"));
        assert!(generated.contains("pub struct SlotTarget<T, C = ()>"));
        assert!(generated.contains("impl<T, C> ::hemx::GeneratedTarget for SlotTarget<T, C>"));
        assert!(generated.contains(
            "impl<K: ::std::string::ToString, T, C> ::hemx::GeneratedTarget for KeyedSlotTarget<K, T, C>"
        ));
        assert!(generated.contains(
            "pub fn append(self, view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn append_keyed(self, key: impl ::std::string::ToString, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains("pub const todos: SlotTarget<::std::string::String, ()> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("pub use self::targets::todos;"));
        assert!(generated.contains(
            "pub fn put(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn replace(self, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn set(self, value: impl ::std::string::ToString) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains("pub mod handles"));
        assert!(generated
            .contains("pub const create: ::hemx::Handle<::hemx::Form<::std::string::String>>"));
        assert!(generated.contains("pub mod params"));
        assert!(generated.contains(
            "pub const todo_id: ::hemx::ParamName = ::hemx::ParamName::new(\"todo_id\")"
        ));
        assert!(generated.contains("pub mod forms"));
        assert!(generated.contains("pub mod atoms"));
        assert!(generated.contains("pub const filter"));
        assert!(generated.contains("pub mod events"));
        assert!(generated
            .contains("pub const click: ::hemx::EventName = ::hemx::EventName::new(\"click\")"));
        assert!(generated
            .contains("pub const change: ::hemx::EventName = ::hemx::EventName::new(\"change\")"));
        assert!(generated.contains("pub const new_todo"));
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains("pub const ALL_IDS"));
        assert!(generated.contains(
            "pub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String"
        ));
        assert!(generated.contains("#[doc(hidden)]\npub fn lower_html"));
        assert!(generated.contains("pub fn static_fragment(html: &'static str) -> ::hemx::Html"));
        assert!(generated.contains(
            "#[doc(hidden)]\npub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(generated.contains("pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(generated.contains(
            "#[doc(hidden)]\npub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(generated.contains("pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("pub fn append<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("pub fn prepend<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("pub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("data-hemx-slot"));
        assert!(generated.contains("data-hemx-form"));
        assert!(generated.contains("data-sid"));
        assert!(generated.contains("data-fid"));
        assert!(generated.contains("__hemx_inject_handle_inputs"));

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        for line in [
            format!(
                "slot\ttodo.heml::todos\ttodos\t{}\n",
                stable_id("slot", "todo.heml::todos")
            ),
            format!(
                "handle\ttodo.heml::create\tcreate\t{}\n",
                stable_id("handle", "todo.heml::create")
            ),
            format!(
                "handle\ttodo.heml::delete\tdelete\t{}\n",
                stable_id("handle", "todo.heml::delete")
            ),
            format!(
                "form\ttodo.heml::new_todo\tnew_todo\t{}\n",
                stable_id("form", "todo.heml::new_todo")
            ),
            "handle_form\tcreate\tnew_todo\n".to_owned(),
            "form_field\tnew_todo\tlabels\tfalse\ttrue\n".to_owned(),
            "form_field\tnew_todo\ttitle\ttrue\tfalse\n".to_owned(),
            "handle_param\tdelete\ttodo_id\n".to_owned(),
            format!(
                "atom\ttodo.heml::filter\tfilter\t{}\n",
                stable_id("atom", "todo.heml::filter")
            ),
            "class\ttodo.css::todo-card\ttodo_card\ttodo-card\n".to_owned(),
            "class\ttodo.heml::todo-card\ttodo_card\ttodo-card\n".to_owned(),
            "event\ttodo.heml::change\tchange\tchange\n".to_owned(),
            "event\ttodo.heml::click\tclick\tclick\n".to_owned(),
        ] {
            assert!(
                syms.contains(&line),
                "missing symbol line {line:?} in {syms}"
            );
        }
        assert!(!syms.contains("form_field\tnew_todo\t__h\t"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn stylesheet_class_scanner_is_sorted_deduplicated_and_boundary_aware() {
        assert_eq!(stylesheet_class_tokens(""), Vec::<&str>::new());
        assert_eq!(stylesheet_class_tokens(".a"), vec!["a"]);
        assert_eq!(stylesheet_class_tokens(".ab"), vec!["ab"]);
        assert_eq!(stylesheet_class_tokens("x.skip .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens("x..kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens("..kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(". .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(".1 .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(".é .kept"), vec!["kept"]);
        assert!(!preceded_by_class_in_compound(b"tag.", 3));
        assert!(preceded_by_class_in_compound(b".first.", 6));
        assert_eq!(
            stylesheet_class_tokens(
                ".z9, .alpha.alpha, ._private, .-prefixed, .compound.second, tag.skipped, name-.skipped, name_.skipped"
            ),
            vec!["-prefixed", "_private", "alpha", "compound", "second", "z9"]
        );
        assert_eq!(
            stylesheet_class_tokens(
                " .space\n.newline\r.return\t.tab,.comma{.open}.close>.child+.adjacent~.sibling(.paren)"
            ),
            vec![
                "adjacent", "child", "close", "comma", "newline", "open", "paren", "return",
                "sibling", "space", "tab"
            ]
        );
    }

    #[test]
    fn class_and_form_resource_contracts_fail_closed_and_deduplicate() {
        let base = test_dir("hemx-build-resource-collisions");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let invalid_stylesheet = templates.join("123.css");
        std::fs::write(&invalid_stylesheet, ".valid {}").unwrap();
        let invalid_stylesheet_name = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_stylesheet_name.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_stylesheet_name.to_string(),
            "invalid template name `123`; expected a Rust module identifier"
        );
        std::fs::remove_file(invalid_stylesheet).unwrap();

        let invalid_class_template = templates.join("invalid_class.heml");
        std::fs::write(&invalid_class_template, r#"<div class="123"></div>"#).unwrap();
        let invalid_class = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_class.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_class.to_string(),
            "invalid CSS class `123`; expected an ASCII class token usable from Rust"
        );
        std::fs::remove_file(invalid_class_template).unwrap();

        let stylesheet = templates.join("app.css");
        std::fs::write(&stylesheet, ".foo-bar {} .foo_bar {}").unwrap();
        let class_error = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(class_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            class_error.to_string(),
            "duplicate generated class identifier `foo_bar` for CSS classes `foo-bar` and `foo_bar`"
        );

        std::fs::remove_file(stylesheet).unwrap();
        let first_template = templates.join("a.heml");
        std::fs::write(
            &first_template,
            r#"<form data-hemx-handle="save_a" data-hemx-form="profile-card"><input name="name"></form>"#,
        )
        .unwrap();
        let invalid_form = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_form.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_form.to_string(),
            "invalid hemx form name `profile-card`; expected a Rust identifier"
        );

        std::fs::write(
            &first_template,
            r#"<form data-hemx-handle="save_a" data-hemx-form="profile"><input name="name"></form>"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("b.heml"),
            r#"<form data-hemx-handle="save_b" data-hemx-form="profile"><input name="name"></form>"#,
        )
        .unwrap();
        let form_error = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(form_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            form_error.to_string(),
            "duplicate generated identifier `profile` for `a.heml::profile` and `b.heml::profile`"
        );

        std::fs::remove_file(templates.join("b.heml")).unwrap();
        std::fs::write(
            templates.join("a.heml"),
            r#"<form data-hemx-handle="save_name" data-hemx-form="profile"><input name="name" required></form><form data-hemx-handle="save_again" data-hemx-form="profile"><input name="name" required></form>"#,
        )
        .unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert_eq!(
            syms.matches("form_field\tprofile\tname\ttrue\tfalse\n")
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn duplicate_collection_slot_upgrades_to_keyed_target() {
        let base = test_dir("hemx-build-keyed-collection-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todos.heml"),
            r#"<ul data-hemx-slot="row"><template h-for="todo in &self.todos" h-key="todo.id"><li +data-key="todo.id">{+ todo +}</li></template></ul>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const row: ::hemx::advanced::KeyedSlot"));
        assert!(generated.contains("pub const row: KeyedSlotTarget"));
        assert!(generated.contains("pub use self::targets::row;"));
        assert!(!generated.contains("pub const row: SlotTarget"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn app_can_consume_precomputed_surface_facts() {
        let base = test_dir("hemx-build-precomputed-surface-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let path = templates.join("todo.heml");
        let source = Arc::new(
            r#"<button data-hemx-handle="create" data-hemx-on="click">Create</button><div data-hemx-slot="todos"></div>"#
                .to_string(),
        );
        let ast = build_ast(source).unwrap().unwrap();
        let surface = extract_surface(&ast);

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .surface(&path, surface)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains("pub const create: ::hemx::Handle<()> = ::hemx::Handle::new("));
        assert!(generated.contains("pub const todos: ::hemx::advanced::Slot<::std::string::String> = ::hemx::advanced::Slot::new("));
        assert!(generated.contains("pub const todos: SlotTarget<::std::string::String, ()> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("pub use self::targets::todos;"));
        assert!(generated
            .contains("pub const click: ::hemx::EventName = ::hemx::EventName::new(\"click\")"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn global_resource_exports_are_explicit_opt_in() {
        let base = test_dir("hemx-build-no-global-exports-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<button data-hemx-handle="create">Create</button><div data-hemx-slot="todos"></div>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(!generated.contains("\n#[allow(non_upper_case_globals)]\npub mod components"));
        assert!(!generated.contains("\n#[allow(non_upper_case_globals)]\npub mod slots"));
        assert!(generated.contains(
            "\npub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String"
        ));
        assert!(generated.contains("\n#[doc(hidden)]\npub fn lower_html"));
        assert!(generated.contains("\npub fn static_fragment(html: &'static str) -> ::hemx::Html"));
        assert!(generated.contains(
            "\n#[doc(hidden)]\npub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(
            generated.contains("\npub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html")
        );
        assert!(generated.contains("\n#[doc(hidden)]\npub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(generated.contains("\npub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("\npub fn append<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains(
            "    pub const COMPONENT: ::hemx::ComponentRef = ::hemx::ComponentRef::new(\"todo\")"
        ));
        assert!(generated.contains("    pub mod advanced"));
        assert!(generated.contains("    pub mod slots"));
        assert!(generated.contains("    pub mod targets"));
        assert!(generated.contains("    pub const todos: SlotTarget<::std::string::String, ()> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("    pub use self::targets::todos;"));
        assert!(generated.contains("    #[doc(hidden)]\n    pub fn lower_html"));
        assert!(generated.contains(
            "    #[doc(hidden)]\n    pub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(
            generated.contains("    pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html")
        );
        assert!(generated.contains("    #[doc(hidden)]\n    pub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(
            generated.contains("    pub fn static_fragment(html: &'static str) -> ::hemx::Html")
        );
        assert!(generated.contains("    pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("    pub fn append<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_form_contract_metadata_from_surface_controls() {
        let base = test_dir("hemx-build-form-contract-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("profile.heml"),
            r#"<form data-hemx-handle="save" data-hemx-form="profile">
                <input type="hidden" name="__h" value="123">
                <input name="title" required>
                <template h-if="alternate"><input name="title"></template>
                <input name="count" type="number" min="1" max="10" step="1">
                <select name="labels" multiple></select>
                <input name="avatar" type="file">
                <input name="token" type="hidden">
                <input name="enabled" type="checkbox">
                <input name="mode" type="radio">
                <textarea name="notes"></textarea>
                <input name="commit" type="submit">
                <input name="birthday" type="date">
                <button name="action" type="button">Preview</button>
            </form>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const PROFILE_CONTRACT: ::hemx::FormContract"));
        assert!(generated.contains("pub const PROFILE_FIELDS: &[::hemx::FormField]"));
        assert!(generated.contains(
            "::hemx::FormField { name: \"title\", kind: ::hemx::FormControlKind::Text, required: true }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"count\", kind: ::hemx::FormControlKind::Number { min: Some(\"1\"), max: Some(\"10\"), step: Some(\"1\") }, required: false }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"labels\", kind: ::hemx::FormControlKind::Select { multiple: true }, required: false }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"avatar\", kind: ::hemx::FormControlKind::File, required: false }"
        ));
        for field in [
            "::hemx::FormField { name: \"token\", kind: ::hemx::FormControlKind::Hidden, required: false }",
            "::hemx::FormField { name: \"enabled\", kind: ::hemx::FormControlKind::Checkbox, required: false }",
            "::hemx::FormField { name: \"mode\", kind: ::hemx::FormControlKind::Radio, required: false }",
            "::hemx::FormField { name: \"notes\", kind: ::hemx::FormControlKind::TextArea, required: false }",
            "::hemx::FormField { name: \"commit\", kind: ::hemx::FormControlKind::Submit, required: false }",
            "::hemx::FormField { name: \"birthday\", kind: ::hemx::FormControlKind::Other { tag: \"input\", input_type: Some(\"date\") }, required: false }",
            "::hemx::FormField { name: \"action\", kind: ::hemx::FormControlKind::Other { tag: \"button\", input_type: None }, required: false }",
        ] {
            assert!(generated.contains(field), "missing form field metadata {field}");
        }

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("handle_form\tsave\tprofile\n"));
        assert!(!syms.contains("form_field\tprofile\t__h\t"));
        assert_eq!(
            syms.matches("form_field\tprofile\ttitle\ttrue\tfalse\n")
                .count(),
            1,
            "conditional controls with one submitted name are one Rust form field"
        ); // test
        assert!(syms.contains("form_field\tprofile\tlabels\tfalse\ttrue\n"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_checked_css_class_tokens_from_templates_and_stylesheets() {
        let base = test_dir("hemx-build-classes-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("card.heml"),
            r#"<article class="card is-active" data-hemx-slot="card_body"></article>"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("card.css"),
            r#".card { padding: 1rem; }
.is-active:hover, .drag-handle { cursor: grab; }
.work-card.is-selected { outline: 1px solid currentColor; }
"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("panel.scss"),
            r#".panel-shell { &.is-open { display: block; } }"#,
        )
        .unwrap();

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .global_exports(true)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod classes"));
        assert!(generated
            .contains("pub const card: ::hemx::CssClass = ::hemx::CssClass::new(\"card\")"));
        assert!(generated.contains(
            "pub const is_active: ::hemx::CssClass = ::hemx::CssClass::new(\"is-active\")"
        ));
        assert!(generated.contains(
            "pub const drag_handle: ::hemx::CssClass = ::hemx::CssClass::new(\"drag-handle\")"
        ));
        assert!(generated.contains(
            "pub const work_card: ::hemx::CssClass = ::hemx::CssClass::new(\"work-card\")"
        ));
        assert!(generated.contains(
            "pub const is_selected: ::hemx::CssClass = ::hemx::CssClass::new(\"is-selected\")"
        ));
        assert!(generated.contains("pub mod card"));
        assert!(generated.contains("pub mod panel"));

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("class\t"));
        assert!(syms.contains("\tdrag_handle\tdrag-handle"));

        let source = format!(
            r###"
#![allow(dead_code)]
mod hemx {{
    #[derive(Clone, Copy)] pub struct BuildFingerprint;
    impl BuildFingerprint {{ pub const fn from_parts(_: &[u32]) -> Self {{ Self }} }}
    pub struct Effect;
    #[derive(Clone, Copy)] pub struct ResourceId;
    pub trait IntoEffect {{}}
    impl IntoEffect for Effect {{}}
    pub trait GeneratedTarget {{ fn __hemx_resource_id(self) -> ResourceId; }}
    pub trait KeyedPartial {{ fn hemx_key(&self) -> String; }}
    #[derive(Clone, Copy)] pub struct Slot<T>(::std::marker::PhantomData<T>);
    impl<T> Slot<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn html(self, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn text(self, _: impl ::std::string::ToString) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct KeyedSlot<K, T>(::std::marker::PhantomData<(K, T)>);
    impl<K, T> KeyedSlot<K, T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn append_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn prepend_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn replace_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn remove(self, _: K) -> Effect {{ Effect }} pub fn move_before(self, _: K, _: K) -> Effect {{ Effect }} pub fn move_to_end(self, _: K) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct Handle<T>(::std::marker::PhantomData<T>);
    impl<T> Handle<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Atom<T>(::std::marker::PhantomData<T>);
    impl<T> Atom<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Form<T>(::std::marker::PhantomData<T>);
    impl<T> Form<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct CssClass(&'static str);
    impl CssClass {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    #[derive(Clone, Copy)] pub struct ComponentRef(&'static str);
    impl ComponentRef {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    pub struct SafeHtml(String);
    impl SafeHtml {{ pub fn trusted(html: impl Into<String>) -> Self {{ Self(html.into()) }} }}
    pub struct Html(SafeHtml);
    impl ::std::convert::From<Html> for SafeHtml {{ fn from(value: Html) -> Self {{ value.0 }} }}
    impl ::std::convert::AsRef<str> for Html {{ fn as_ref(&self) -> &str {{ "" }} }}
    pub mod __private {{ pub fn html_trusted(html: impl Into<String>) -> super::Html {{ super::Html(super::SafeHtml::trusted(html)) }} }}
    pub mod advanced {{ pub use super::*; }}
    pub struct FormContract {{ pub fields: &'static [FormField] }}
    pub struct FormField {{ pub name: &'static str, pub kind: FormControlKind, pub required: bool }}
    pub enum FormControlKind {{ Text, Number {{ min: Option<&'static str>, max: Option<&'static str>, step: Option<&'static str> }}, Checkbox, Radio, Select {{ multiple: bool }}, TextArea, File, Hidden, Submit, Other {{ tag: &'static str, input_type: Option<&'static str> }} }}
}}
mod hemplate {{
    pub trait Hemplate {{ fn render_into(&self, out: &mut String) -> Result<(), ()>; }}
}}
{generated}
fn main() {{
    assert_eq!(classes::drag_handle.as_str(), "drag-handle");
    assert_eq!(card::classes::is_active.as_str(), "is-active");
    assert_eq!(panel::classes::panel_shell.as_str(), "panel-shell");
}}
"###,
        );
        let source_path = base.join("classes.rs");
        let bin_path = base.join("classes-bin");
        std::fs::write(&source_path, source).unwrap();
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&bin_path)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new(&bin_path).status().unwrap();
        assert!(status.success());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn diagnostics_for_heml_file_reports_unkeyed_generated_target() {
        let dir = test_dir("diagnostics_for_heml_file_reports_unkeyed_generated_target");
        std::fs::create_dir_all(&dir).expect("create test dir");
        let template = dir.join("todo.heml");
        std::fs::write(
            &template,
            r#"<main data-hemx-root="todos"><template h-for="todo in &self.todos"><li data-hemx-slot="todo_row">{+ todo.title +}</li></template></main>"#,
        )
        .expect("write template");

        let diagnostics = diagnostics_for_heml_file(&template).expect("diagnostics");
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, DiagnosticCode::UnkeyedGeneratedTarget);
        assert_eq!(diagnostic.directive, "data-hemx-slot");
        assert_eq!(diagnostic.target, "todo_row");
        assert!(diagnostic.repair.contains("h-key=\"todo.id\""));
    }

    #[test]
    fn generated_targets_for_heml_source_uses_surface_facts() {
        let targets = generated_targets_for_heml_source(
            "inline.heml",
            r#"
                <main data-hemx-root="todos">
                    <section class="card" data-hemx-slot="summary"></section>
                    <section data-hemx-root="nested" data-hemx-slot="after-root"></section>
                    <section data-hemx-slot="summary"></section>
                    <form data-hemx-form="summary"></form>
                    <span data-hemx-handle data-hemx-slot="after-missing"></span>
                    <button data-hemx-handle="submit">Save</button>
                    <aside data-hemx-slot="later"></aside>
                </main>
            "#,
        )
        .expect("generated targets");

        assert_eq!(
            targets,
            vec![
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "summary".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "after-root".into(),
                },
                GeneratedTarget {
                    kind: "form".into(),
                    name: "summary".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "after-missing".into(),
                },
                GeneratedTarget {
                    kind: "handle".into(),
                    name: "submit".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "later".into(),
                },
            ]
        );
    }

    #[test]
    fn template_context_facts_include_self_fields_and_h_for_local_fields() {
        let dir = test_dir("template_context_facts_include_self_fields_and_h_for_local_fields");
        std::fs::create_dir_all(dir.join("src")).expect("create src dir");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"facts\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");
        std::fs::write(
            dir.join("src/lib.rs"),
            r#"
                #[derive(Clone)]
                pub struct ExercisePlan { pub name: String, pub kg: f32 }

                #[derive(Hemplate)]
                pub struct Workout {
                    pub plan: Vec<ExercisePlan>,
                    pub alternate: std::vec::Vec<ExercisePlan>,
                    pub mysteries: Vec<MissingFact>,
                    pub progress: String,
                }
            "#,
        )
        .expect("write lib");
        let template = dir.join("workout.heml");
        let source = r#"
            <main title="ghost in self.plan">
                <p h-for="malformed">{+ self.progress +}</p>
                <p h-for="(tuple, item) in self.plan">ignored</p>
                <p h-for="unknown in self.unknown">ignored</p>
                <p h-for="scalar in self.progress">ignored</p>
                <li h-for="exercise in &self.plan">{+ exercise.name +}</li>
                <li h-for="exercise in self.alternate">{+ exercise.kg +}</li>
                <li h-for="mystery in self.mysteries">{+ mystery.anything +}</li>
                <li h-for="later in self.plan[0]">{+ later.name +}</li>
            </main>
        "#;
        std::fs::write(&template, source).expect("write heml");

        let facts = template_context_facts_for_heml_source(&template, source)
            .expect("facts")
            .expect("derive facts");

        assert_eq!(facts.context_type, "Workout");
        assert!(facts
            .self_fields
            .iter()
            .any(|field| field.name == "progress" && field.type_name == "String"));
        let local = facts
            .locals
            .iter()
            .find(|local| local.name == "exercise")
            .expect("exercise local");
        assert_eq!(local.type_name, "ExercisePlan");
        assert!(local
            .fields
            .iter()
            .any(|field| field.name == "name" && field.type_name == "String"));
        assert_eq!(
            facts
                .locals
                .iter()
                .filter(|local| local.name == "exercise")
                .count(),
            1
        );
        let mystery = facts
            .locals
            .iter()
            .find(|local| local.name == "mystery")
            .expect("unknown element type still exposes the local");
        assert_eq!(mystery.type_name, "MissingFact");
        assert!(mystery.fields.is_empty());
        assert!(facts.locals.iter().any(|local| {
            local.name == "later"
                && local.type_name == "ExercisePlan"
                && local.fields.iter().any(|field| field.name == "kg")
        }));
        for rejected in ["ghost", "tuple", "item", "unknown", "scalar"] {
            assert!(
                facts.locals.iter().all(|local| local.name != rejected),
                "unexpected loop local {rejected}"
            );
        }

        assert_eq!(
            h_for_local_and_self_field(" item in &self.plan "),
            Some(("item".into(), "plan".into()))
        );
        assert_eq!(
            h_for_local_and_self_field("item in self.plan.iter()"),
            Some(("item".into(), "plan".into()))
        );
        for invalid in [
            "item:self.plan",
            " in self.plan",
            "1item in self.plan",
            "item-name in self.plan",
            "(item, index) in self.plan",
            "item in plan",
            "item in &plan",
            "item in self.",
        ] {
            assert_eq!(
                h_for_local_and_self_field(invalid),
                None,
                "accepted invalid h-for {invalid:?}"
            );
        }
    }

    #[test]
    fn unkeyed_generated_target_diagnostic_is_structured() {
        let diagnostic = unkeyed_generated_target_diagnostic(
            Path::new("templates/todo.heml"),
            "slot",
            "todo_row",
            "todo",
            "&self.todos",
        );

        assert_eq!(diagnostic.code, DiagnosticCode::UnkeyedGeneratedTarget);
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.file, PathBuf::from("templates/todo.heml"));
        assert_eq!(diagnostic.directive, "data-hemx-slot");
        assert_eq!(diagnostic.target, "todo_row");
        assert!(diagnostic
            .message
            .contains("h-for=\"todo in &self.todos\" without h-key"));
        assert!(diagnostic.expected.contains("ui::todo_row.replace(row)"));
        assert!(diagnostic.repair.contains("h-key=\"todo.id\""));
        assert!(diagnostic.repair.contains("dynamic +data-key on the child"));
        assert!(diagnostic.to_string().contains("templates/todo.heml"));
    }

    #[test]
    fn rejects_hemx_resources_inside_unkeyed_for() {
        for (case, template, resource, helper) in [
            (
                "slot",
                r#"<template h-for="todo in &self.todos"><li data-hemx-slot="todo_row">{+ todo.title +}</li></template>"#,
                "data-hemx-slot=\"todo_row\"",
                "ui::todo_row",
            ),
            (
                "slot_data_key",
                r#"<template h-for="todo in &self.todos"><li data-hemx-slot="todo_row" +data-key="todo.id">{+ todo.title +}</li></template>"#,
                "data-hemx-slot=\"todo_row\"",
                "ui::todo_row",
            ),
            (
                "handle",
                r#"<template h-for="todo in &self.todos"><button data-hemx-handle="delete">Delete</button></template>"#,
                "data-hemx-handle=\"delete\"",
                "ui::delete",
            ),
            (
                "nested-handle",
                r#"<template h-for="todo in &self.todos"><template h-if="todo.visible"><button data-hemx-handle="delete">Delete</button></template></template>"#,
                "data-hemx-handle=\"delete\"",
                "ui::delete",
            ),
            (
                "atom",
                r#"<template h-for="todo in &self.todos"><span data-hemx-atom="selected">Selected</span></template>"#,
                "data-hemx-atom=\"selected\"",
                "ui::selected",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-unkeyed-for-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join("todo.heml"), template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            let err = err.to_string();
            assert!(err.contains("inside h-for=\"todo in &self.todos\" without h-key"));
            assert!(err.contains(resource));
            assert!(err.contains("h-key=\"todo.id\""));
            assert!(err.contains("generated keyed helpers"));
            assert!(err.contains(helper));
            assert!(err.contains("dynamic +data-key on the child"));

            let _ = std::fs::remove_dir_all(&base);
        }

        let base = test_dir("hemx-build-keyed-nested-resources-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let keyed_source = r#"
            <main>
                <ul data-hemx-slot="todos">
                    <template h-for="todo in &self.todos" h-key="todo.id">
                        <li data-hemx-slot="todo_row">
                            <template h-for="child in &todo.children" h-key="child.id">
                                <button data-hemx-handle="delete">Delete</button>
                                <span data-hemx-atom="selected">Selected</span>
                            </template>
                        </li>
                    </template>
                    <template h-for="other in &self.others"><li>Unaddressed sibling</li></template>
                </ul>
                <aside data-hemx-slot="summary"></aside>
            </main>
        "#;
        std::fs::write(templates.join("todo.heml"), keyed_source).unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("slot\ttodo.heml::todos\ttodos\t"));
        assert!(syms.contains("slot\ttodo.heml::todo_row\ttodo_row\t"));
        assert!(syms.contains("handle\ttodo.heml::delete\tdelete\t"));
        assert!(syms.contains("atom\ttodo.heml::selected\tselected\t"));
        assert!(syms.contains("slot\ttodo.heml::summary\tsummary\t"));
        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const todo_row: ::hemx::advanced::KeyedSlot"));
        assert!(generated.contains("pub const todos: ::hemx::advanced::KeyedSlot"));
        assert!(generated.contains("pub const summary: ::hemx::advanced::Slot"));
        assert!(!generated.contains("pub const summary: ::hemx::advanced::KeyedSlot"));

        let surface =
            surface_for_heml_source(Path::new("todo.heml"), keyed_source.to_owned()).unwrap();
        let keyed_scope = surface
            .scopes
            .iter()
            .position(|scope| {
                matches!(
                    scope.kind,
                    ScopeKind::For {
                        key_expr: Some(_),
                        ..
                    }
                )
            })
            .map(|index| ScopeId(index as u32))
            .unwrap();
        let nested_scope = surface
            .scopes
            .iter()
            .enumerate()
            .find(|(_, scope)| scope.parent == Some(keyed_scope))
            .map(|(index, _)| ScopeId(index as u32))
            .unwrap();
        let sibling_scope = surface
            .scopes
            .iter()
            .enumerate()
            .find(|(_, scope)| {
                matches!(scope.kind, ScopeKind::For { key_expr: None, .. })
                    && scope.parent != Some(keyed_scope)
            })
            .map(|(index, _)| ScopeId(index as u32))
            .unwrap();
        assert!(is_descendant_scope(&surface, nested_scope, keyed_scope));
        assert!(!is_descendant_scope(&surface, keyed_scope, keyed_scope));
        assert!(!is_descendant_scope(&surface, sibling_scope, keyed_scope));
        assert!(!is_descendant_scope(
            &surface,
            ScopeId(u32::MAX),
            keyed_scope
        ));
        assert!(is_inside_keyed_for(&surface, nested_scope));
        assert!(!is_inside_keyed_for(&surface, sibling_scope));
        assert!(!is_inside_keyed_for(&surface, ScopeId(u32::MAX)));
        assert!(has_descendant_keyed_for_scope(&surface, ScopeId(0)));
        assert!(!has_descendant_keyed_for_scope(&surface, sibling_scope));
        assert!(unkeyed_generated_target_diagnostic_for_scope(
            &surface,
            ScopeId(u32::MAX),
            Path::new("todo.heml"),
            "slot",
            "missing",
        )
        .is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn rejects_selector_style_targeting_attrs() {
        for (case, template, attr) in [
            (
                "hemx-target",
                r##"<button data-hemx-handle="save" data-hemx-target="#row">Save</button>"##,
                "data-hemx-target",
            ),
            (
                "hemx-select",
                r##"<button data-hemx-handle="save" data-hemx-select="closest tr">Save</button>"##,
                "data-hemx-select",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-selector-target-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            let template_path = templates.join("target.heml");
            std::fs::write(&template_path, template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                err.to_string(),
                format!(
                    "{}: `{attr}` is selector-style targeting; hemx uses generated resources instead. Add data-hemx-slot to the local element and return an effect for that generated slot.",
                    template_path.display()
                )
            );

            let _ = std::fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn rejects_unknown_hemx_authoring_attrs() {
        let base = test_dir("hemx-build-unknown-hemx-attr-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let template_path = templates.join("typo.heml");
        std::fs::write(
            &template_path,
            r#"<button data-hemx-handle="save" data-hemx-pendig-class="busy">Save</button>"#,
        )
        .unwrap();

        let err = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            err.to_string(),
            format!(
                "{}: unknown hemx attribute `data-hemx-pendig-class`; check the spelling or use a non-hemx data-* attribute for app-specific metadata",
                template_path.display()
            )
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_invalid_static_hemx_convention_values() {
        for (case, template, attr, guidance) in [
            (
                "policy",
                r#"<button data-hemx-handle="save" data-hemx-policy="newest">Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "policy-missing",
                r#"<button data-hemx-handle="save" data-hemx-policy>Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "empty-client",
                r#"<button data-hemx-client="">Save</button>"#,
                "data-hemx-client",
                "non-empty",
            ),
            (
                "client-module-scheme",
                r#"<main data-hemx-root="app" data-hemx-client-module="https://example.com/app.js"></main>"#,
                "data-hemx-client-module",
                "same-origin",
            ),
            (
                "client-event",
                r#"<button data-hemx-client="save" data-hemx-client-event="blur">Save</button>"#,
                "data-hemx-client-event",
                "runtime-supported",
            ),
            (
                "client-version-zero",
                r#"<button data-hemx-client="save" data-hemx-client-state-version="0">Save</button>"#,
                "data-hemx-client-state-version",
                "positive",
            ),
            (
                "client-version-invalid",
                r#"<button data-hemx-client="save" data-hemx-client-state-version="one">Save</button>"#,
                "data-hemx-client-state-version",
                "positive",
            ),
            (
                "client-policy-invalid",
                r#"<button data-hemx-client="save" data-hemx-client-policy="queue">Save</button>"#,
                "data-hemx-client-policy",
                "latest",
            ),
            (
                "events-mixed",
                r#"<button data-hemx-handle="save" data-hemx-on="click blur">Save</button>"#,
                "data-hemx-on",
                "runtime-supported",
            ),
            (
                "confirm-empty",
                r#"<button data-hemx-handle="save" data-hemx-confirm=" ">Save</button>"#,
                "data-hemx-confirm",
                "non-empty",
            ),
            (
                "sse-empty",
                r#"<main data-hemx-root="app" data-hemx-sse=" "></main>"#,
                "data-hemx-sse",
                "non-empty",
            ),
            (
                "delay-empty",
                r#"<button data-hemx-handle="save" data-hemx-delay="">Save</button>"#,
                "data-hemx-delay",
                "milliseconds",
            ),
            (
                "revealed-ahead-negative",
                r#"<form data-hemx-revealed data-hemx-revealed-ahead="-1"></form>"#,
                "data-hemx-revealed-ahead",
                "non-negative",
            ),
            (
                "throttle-invalid",
                r#"<button data-hemx-handle="save" data-hemx-throttle="ms">Save</button>"#,
                "data-hemx-throttle",
                "milliseconds",
            ),
            (
                "duration-suffix-only",
                r#"<button data-hemx-handle="save" data-hemx-delay="s">Save</button>"#,
                "data-hemx-delay",
                "milliseconds",
            ),
            (
                "policy-duplicate-anchor",
                r#"<button data-hemx-handle="save" data-hemx-policy="newest">Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "debounce",
                r#"<button data-hemx-handle="save" data-hemx-debounce="soon">Save</button>"#,
                "data-hemx-debounce",
                "250ms",
            ),
            (
                "delay",
                r#"<button data-hemx-handle="save" data-hemx-delay="soon">Save</button>"#,
                "data-hemx-delay",
                "250ms",
            ),
            (
                "every",
                r#"<button data-hemx-handle="save" data-hemx-every="1sec">Save</button>"#,
                "data-hemx-every",
                "1s",
            ),
            (
                "interval",
                r#"<button data-hemx-handle="save" data-hemx-interval="often">Save</button>"#,
                "data-hemx-interval",
                "1s",
            ),
            (
                "history",
                r#"<form method="get" action="/search" data-hemx-history="append"><input name="q"></form>"#,
                "data-hemx-history",
                "replace",
            ),
            (
                "event",
                r#"<button data-hemx-handle="save" data-hemx-on="blur">Save</button>"#,
                "data-hemx-on",
                "click",
            ),
            (
                "empty-event",
                r#"<button data-hemx-handle="save" data-hemx-on="">Save</button>"#,
                "data-hemx-on",
                "click",
            ),
            (
                "empty-confirm",
                r#"<button data-hemx-handle="delete" data-hemx-confirm="">Delete</button>"#,
                "data-hemx-confirm",
                "non-empty",
            ),
            (
                "empty-sse",
                r#"<section data-hemx-root="notifications" data-hemx-sse=""></section>"#,
                "data-hemx-sse",
                "same-origin SSE URL",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-invalid-convention-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            let template_path = templates.join("invalid.heml");
            std::fs::write(&template_path, template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            let surface = surface_for_heml_source(&template_path, template.to_owned()).unwrap();
            let value = surface
                .nodes
                .iter()
                .flat_map(|node| &node.attrs)
                .find(|candidate| candidate.name == attr)
                .and_then(|candidate| candidate.value.as_deref())
                .unwrap_or("");
            let expectation = match attr {
                "data-hemx-policy" => {
                    "expected one of `latest`, `queue`, `drop`, or `parallel`"
                }
                "data-hemx-history" => {
                    "expected `push`, `replace`, or empty for the default push behavior"
                }
                "data-hemx-client" => "expected a non-empty client handler name",
                "data-hemx-client-policy" => "expected `latest` or `drop`",
                "data-hemx-client-module" => {
                    "expected a same-origin module specifier beginning with `/`, `./`, or `../`"
                }
                "data-hemx-client-event" => "expected a runtime-supported event",
                "data-hemx-client-state-version" => {
                    "expected a positive client state ABI version"
                }
                "data-hemx-revealed-ahead" => "expected a non-negative number of viewports",
                "data-hemx-on" => "expected runtime-supported events: `click`, `submit`, `input`, `change`, `keydown`, `dragstart`, `dragover`, or `drop`",
                "data-hemx-confirm" => "expected a non-empty confirmation message",
                "data-hemx-sse" => "expected a non-empty same-origin SSE URL",
                "data-hemx-debounce" | "data-hemx-delay" | "data-hemx-throttle"
                | "data-hemx-every" | "data-hemx-interval" => {
                    "expected milliseconds like `250`/`250ms` or seconds like `1s`"
                }
                _ => panic!("missing expectation for {attr}"),
            };
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                err.to_string(),
                format!(
                    "{}: invalid {attr} value `{value}`; {expectation}",
                    template_path.display()
                ),
                "guidance anchor {guidance:?}"
            );

            let _ = std::fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn static_attribute_value_validators_cover_every_boundary() {
        for valid in ["latest", " queue ", "drop", "parallel"] {
            assert!(valid_policy(valid), "rejected policy {valid:?}");
        }
        for invalid in ["", "newest", "latest queue", "LATEST"] {
            assert!(!valid_policy(invalid), "accepted policy {invalid:?}");
        }

        for valid in ["/app.js", " ./app.js ", "../app.js"] {
            assert!(valid_client_module(valid), "rejected module {valid:?}");
        }
        for invalid in [
            "",
            " ",
            "//cdn/app.js",
            "https://example.com/app.js",
            "app.js",
        ] {
            assert!(!valid_client_module(invalid), "accepted module {invalid:?}");
        }

        for valid in [
            "click",
            "submit input change keydown dragstart dragover drop",
            " click change ",
        ] {
            assert!(valid_event_list(valid), "rejected event list {valid:?}");
        }
        for invalid in ["", " ", "blur", "click blur"] {
            assert!(
                !valid_event_list(invalid),
                "accepted event list {invalid:?}"
            );
        }
        for valid in [
            "click",
            "submit",
            "input",
            "change",
            "keydown",
            "dragstart",
            "dragover",
            "drop",
        ] {
            assert!(valid_runtime_event(valid), "rejected event {valid:?}");
        }
        for invalid in ["", "blur", "Click"] {
            assert!(!valid_runtime_event(invalid), "accepted event {invalid:?}");
        }

        for valid in ["0", "250", "250ms", "1s", " 5s "] {
            assert!(valid_duration(valid), "rejected duration {valid:?}");
        }
        for invalid in ["", " ", "ms", "s", "-1", "1sec", "1.5s", "1 ms"] {
            assert!(!valid_duration(invalid), "accepted duration {invalid:?}");
        }

        let valid_source = r#"
            <main data-hemx-root="app" data-hemx-client-module="/app.js" data-hemx-sse="/events">
                <button data-hemx-handle="save" data-hemx-policy="latest" data-hemx-on="click change" data-hemx-confirm="Save?" data-hemx-delay="250ms" data-hemx-throttle="1s">Save</button>
                <button data-hemx-client="save" data-hemx-client-event="click" data-hemx-client-policy="drop" data-hemx-client-state-version="1">Client</button>
                <form method="get" action="/search" data-hemx-history="replace"><input name="q"></form>
            </main>
        "#;
        let base = test_dir("hemx-build-valid-static-values");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(templates.join("valid.heml"), valid_source).unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn placement_contracts_fail_closed_with_exact_diagnostics() {
        for (case, template, attr, expectation) in [
            (
                "nav-button",
                r#"<button data-hemx-nav="" data-hemx-handle="go">Go</button>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "nav-missing-href",
                r#"<a data-hemx-nav="">Docs</a>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "nav-empty-href",
                r#"<a href="  " data-hemx-nav="">Docs</a>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "boost-anchor",
                r#"<a href="/docs" data-hemx-boost="">Docs</a>"#,
                "data-hemx-boost",
                "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
            ),
            (
                "boost-form",
                r#"<form data-hemx-boost=""><input name="q"></form>"#,
                "data-hemx-boost",
                "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
            ),
            (
                "client-module-child",
                r#"<section data-hemx-root="app"><div data-hemx-client-module="/app.js"></div></section>"#,
                "data-hemx-client-module",
                "expected placement on the same element as `data-hemx-root`",
            ),
            (
                "sse-child",
                r#"<section data-hemx-root="feed"><div data-hemx-sse="/events"></div></section>"#,
                "data-hemx-sse",
                "expected placement on the same element as `data-hemx-root`",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-invalid-placement-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let template_path = templates.join("placement.heml");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(&template_path, template).unwrap();

            let error = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                format!(
                    "{}: invalid {attr} placement; {expectation}",
                    template_path.display()
                )
            );

            let _ = std::fs::remove_dir_all(&base);
        }

        for (case, template) in [
            ("nav-link", r#"<a href="/docs" data-hemx-nav="">Docs</a>"#),
            (
                "boost-container",
                r#"<nav data-hemx-boost=""><a href="/docs">Docs</a></nav>"#,
            ),
            (
                "client-module-root",
                r#"<main data-hemx-root="app" data-hemx-client-module="/app.js"></main>"#,
            ),
            (
                "sse-root",
                r#"<main data-hemx-root="feed" data-hemx-sse="/events"></main>"#,
            ),
        ] {
            let base = test_dir(&format!("hemx-build-valid-placement-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join("placement.heml"), template).unwrap();
            app().template_dir(&templates).out_dir(&out).run().unwrap();
            let _ = std::fs::remove_dir_all(base);
        }
    }

    #[test]
    fn generated_lowering_injects_progressive_form_handle_and_key() {
        let base = test_dir("hemx-build-lowering-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<form data-hemx-handle="create"><input name="title"></form><li data-hemx-slot="row" data-hemx-key="7"></li>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("fn __hemx_lower_html"));
        let source = format!(
            r###"
#![allow(dead_code)]
mod hemx {{
    #[derive(Clone, Copy)] pub struct BuildFingerprint;
    impl BuildFingerprint {{ pub const fn from_parts(_: &[u32]) -> Self {{ Self }} }}
    pub struct Effect;
    #[derive(Clone, Copy)] pub struct ResourceId;
    pub trait IntoEffect {{}}
    impl IntoEffect for Effect {{}}
    pub trait GeneratedTarget {{ fn __hemx_resource_id(self) -> ResourceId; }}
    pub trait KeyedPartial {{ fn hemx_key(&self) -> String; }}
    #[derive(Clone, Copy)] pub struct Slot<T>(::std::marker::PhantomData<T>);
    impl<T> Slot<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn html(self, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn text(self, _: impl ::std::string::ToString) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct KeyedSlot<K, T>(::std::marker::PhantomData<(K, T)>);
    impl<K, T> KeyedSlot<K, T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn append_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn prepend_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn replace_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn remove(self, _: K) -> Effect {{ Effect }} pub fn move_before(self, _: K, _: K) -> Effect {{ Effect }} pub fn move_to_end(self, _: K) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct Handle<T>(::std::marker::PhantomData<T>);
    impl<T> Handle<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Atom<T>(::std::marker::PhantomData<T>);
    impl<T> Atom<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Form<T>(::std::marker::PhantomData<T>);
    impl<T> Form<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct ComponentRef(&'static str);
    impl ComponentRef {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    pub struct SafeHtml(String);
    impl SafeHtml {{ pub fn trusted(html: impl Into<String>) -> Self {{ Self(html.into()) }} }}
    pub struct Html(SafeHtml);
    impl ::std::convert::From<Html> for SafeHtml {{ fn from(value: Html) -> Self {{ value.0 }} }}
    impl ::std::convert::AsRef<str> for Html {{ fn as_ref(&self) -> &str {{ "" }} }}
    pub mod __private {{ pub fn html_trusted(html: impl Into<String>) -> super::Html {{ super::Html(super::SafeHtml::trusted(html)) }} }}
    pub mod advanced {{ pub use super::*; }}
    pub struct FormContract {{ pub fields: &'static [FormField] }}
    pub struct FormField {{ pub name: &'static str, pub kind: FormControlKind, pub required: bool }}
    pub enum FormControlKind {{ Text, Number {{ min: Option<&'static str>, max: Option<&'static str>, step: Option<&'static str> }}, Checkbox, Radio, Select {{ multiple: bool }}, TextArea, File, Hidden, Submit, Other {{ tag: &'static str, input_type: Option<&'static str> }} }}
}}
mod hemplate {{
    pub trait Hemplate {{ fn render_into(&self, out: &mut String) -> Result<(), ()>; }}
}}
{generated}
fn main() {{
    let html = todo::lower_html(r#"<form data-hemx-handle="create"><input name="title"></form><li data-hemx-slot="row" h-key="7"></li>"#);
    assert!(html.contains(r#"data-hid="#));
    assert!(html.contains(r#"name="__h""#));
    assert!(html.contains(r#"data-key="7""#)); // test
    assert!(!html.contains("h-key"));
}}
"###,
        );
        let source_path = base.join("lowering.rs");
        let bin_path = base.join("lowering-bin");
        std::fs::write(&source_path, source).unwrap();
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&bin_path)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new(&bin_path).status().unwrap();
        assert!(status.success());

        let _ = std::fs::remove_dir_all(&base);
    }

    fn test_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
    }
}
