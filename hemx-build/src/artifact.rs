use super::*;

pub(super) fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::write(path, contents)
}

#[derive(Default)]
pub(super) struct Resources {
    pub(super) slots: BTreeMap<String, Resource>,
    pub(super) handles: BTreeMap<String, Resource>,
    pub(super) handle_forms: BTreeMap<String, String>,
    pub(super) handle_params: BTreeMap<String, BTreeSet<String>>,
    pub(super) forms: BTreeMap<String, FormResource>,
    pub(super) classes: BTreeMap<String, ClassToken>,
    pub(super) events: BTreeMap<String, EventToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SlotRole {
    Slot,
    Row,
    Collection,
}

impl SlotRole {
    fn fingerprint_part(self) -> u32 {
        match self {
            Self::Slot => 0,
            Self::Row => 1,
            Self::Collection => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Resource {
    pub(super) symbol: String,
    pub(super) ident: String,
    pub(super) component: String,
    pub(super) role: SlotRole,
    pub(super) id: u32,
}

#[derive(Clone, Debug)]
pub(super) struct FormResource {
    pub(super) resource: Resource,
    pub(super) controls: Vec<GeneratedControl>,
}

#[derive(Clone, Debug)]
pub(super) struct GeneratedControl {
    pub(super) name: String,
    pub(super) kind: ControlKind,
    pub(super) required: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ClassToken {
    pub(super) symbol: String,
    pub(super) ident: String,
    pub(super) component: String,
    pub(super) token: String,
}

#[derive(Clone, Debug)]
pub(super) struct EventToken {
    pub(super) symbol: String,
    pub(super) ident: String,
    pub(super) component: String,
    pub(super) name: String,
}

impl Resources {
    pub(super) fn add_surface(
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

            if let Some(name) = static_attr(&node.attrs, "data-hemx-slot") {
                reject_unkeyed_loop(surface, node.scope, path, "slot", &name)?;
                let role = if is_inside_keyed_for(surface, node.scope)
                    || node.attrs.iter().any(|attr| attr.name == "data-hemx-key")
                {
                    SlotRole::Row
                } else {
                    SlotRole::Slot
                };
                let canonical = canonical_symbol(root, path, &name);
                self.insert_slot(canonical, name, component.clone(), role)?;
            }
            if let Some(name) = static_attr(&node.attrs, "data-hemx-collection") {
                // A collection may repeat under separate roots, even in an unkeyed loop.
                // Runtime root ownership and destination uniqueness decide addressability.
                let canonical = canonical_symbol(root, path, &name);
                self.insert_slot(canonical, name, component.clone(), SlotRole::Collection)?;
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

    pub(super) fn insert_slot(
        &mut self,
        symbol: String,
        name: String,
        component: String,
        role: SlotRole,
    ) -> io::Result<()> {
        let ident = rust_ident(&name);
        if let Some(existing) = ident.as_ref().and_then(|ident| self.slots.get(ident)) {
            if existing.symbol == symbol && existing.role != role {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "conflicting slot roles for `{symbol}`: {:?} and {role:?}; use distinct names for ordinary slots, keyed rows, and collections",
                        existing.role
                    ),
                ));
            }
        }
        let slot = insert_resource(&mut self.slots, "slot", symbol, name, component)?;
        slot.role = role;
        Ok(())
    }

    pub(super) fn insert_handle(
        &mut self,
        symbol: String,
        name: String,
        component: String,
    ) -> io::Result<()> {
        insert_resource(&mut self.handles, "handle", symbol, name, component).map(|_| ())
    }

    pub(super) fn insert_class(
        &mut self,
        symbol: String,
        token: String,
        component: String,
    ) -> io::Result<()> {
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

    pub(super) fn insert_handle_params(
        &mut self,
        handle: &str,
        attrs: &[SurfaceAttribute],
    ) -> io::Result<()> {
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
                    format!("invalid handler param attribute `{}`; expected data-hemx-param-* name usable from Rust", attr.name),
                ));
            };
            self.handle_params
                .entry(handle_ident.clone())
                .or_default()
                .insert(param);
        }
        Ok(())
    }

    pub(super) fn insert_event(&mut self, symbol: String, name: String, component: String) {
        self.events.entry(symbol.clone()).or_insert(EventToken {
            symbol,
            ident: name.clone(),
            component,
            name,
        });
    }

    pub(super) fn insert_form(
        &mut self,
        symbol: String,
        name: String,
        component: String,
        controls: Vec<GeneratedControl>,
    ) -> io::Result<()> {
        let controls = controls
            .into_iter()
            .filter(|control| control.name != "__h" && control.name != "__b")
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

    pub(super) fn add_stylesheet(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
    ) -> io::Result<()> {
        let component = component_ident(root, path)?;
        for token in stylesheet_class_tokens(source) {
            let canonical = canonical_symbol(root, path, token);
            self.insert_class(canonical, token.to_owned(), component.clone())?;
        }
        Ok(())
    }

    pub(super) fn generated_rs(&self, global_exports: bool) -> String {
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

        let root_indent = usize::default();
        if global_exports {
            self.push_component_refs(&mut out);
            self.push_resource_modules(&mut out, None, root_indent);
        }
        self.push_lowering_api(&mut out, None, root_indent);

        for component in self.component_names() {
            out.push_str("\n#[allow(non_upper_case_globals)]\n");
            out.push_str(&format!("pub mod {component} {{\n"));
            self.push_resource_modules(&mut out, Some(&component), 1);
            self.push_lowering_api(&mut out, Some(&component), 1);
            out.push_str("}\n");
        }

        self.push_lowering_helpers(&mut out);
        out
    }

    pub(super) fn push_component_refs(&self, out: &mut String) {
        out.push_str("#[allow(non_upper_case_globals)]\npub mod components {\n");
        for component in self.component_names() {
            out.push_str(&format!(
                "    pub const {component}: ::hemx::ComponentRef = ::hemx::ComponentRef::new({});\n",
                rust_str(&component)
            ));
        }
        out.push_str("}\n");
    }

    pub(super) fn push_resource_modules(
        &self,
        out: &mut String,
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
            if res.role == SlotRole::Row {
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
        out.push_str(&format!(r#"{inner}#[derive(Clone, Copy)]
{inner}pub struct SlotTarget<T> {{ slot: ::hemx::advanced::Slot<T> }}
{inner}#[allow(dead_code)]
{inner}impl<T> SlotTarget<T> {{
{inner}    const fn new(slot: ::hemx::advanced::Slot<T>) -> Self {{ Self {{ slot }} }}
{inner}    pub fn text(self, __hemx_value: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.text(__hemx_value) }}
{inner}    pub fn set(self, __hemx_value: impl ::std::string::ToString) -> ::hemx::advanced::Effect {{ self.slot.text(__hemx_value) }}
{inner}    pub fn put(self, __hemx_view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ super::put(self.slot, __hemx_view) }}
{inner}    pub fn replace(self, __hemx_view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ super::put(self.slot, __hemx_view) }}
{inner}}}
{inner}impl<T> ::hemx::GeneratedTarget for SlotTarget<T> {{
{inner}    fn __hemx_resource_id(self) -> ::hemx::advanced::ResourceId {{ self.slot.id() }}
{inner}}}
{inner}#[derive(Clone, Copy)]
{inner}pub struct KeyedSlotTarget<K, T> {{ slot: ::hemx::advanced::KeyedSlot<K, T> }}
{inner}#[allow(dead_code)]
{inner}impl<K: ::std::string::ToString, T> KeyedSlotTarget<K, T> {{
{inner}    const fn new(slot: ::hemx::advanced::KeyedSlot<K, T>) -> Self {{ Self {{ slot }} }}
{inner}}}
{inner}impl<K: ::std::string::ToString, T> ::hemx::GeneratedTarget for KeyedSlotTarget<K, T> {{
{inner}    fn __hemx_resource_id(self) -> ::hemx::advanced::ResourceId {{ self.slot.id() }}
{inner}}}
{inner}#[derive(Clone, Copy)]
{inner}pub struct CollectionTarget<T> {{ slot: ::hemx::advanced::Slot<T>, rows: ::hemx::advanced::KeyedSlot<::std::string::String, T> }}
{inner}#[allow(dead_code)]
{inner}impl<T> CollectionTarget<T> {{
{inner}    const fn new(slot: ::hemx::advanced::Slot<T>, row_slot: ::hemx::advanced::KeyedSlot<::std::string::String, T>) -> Self {{ Self {{ slot, rows: row_slot }} }}
{inner}    /// Declare the complete collection: every row, in this order.
{inner}    pub fn set<'a, V>(self, __hemx_rows: impl ::std::iter::IntoIterator<Item = &'a V>) -> ::hemx::advanced::Effect where V: 'a + ::hemplate::Hemplate + ::hemx::KeyedPartial {{
{inner}        let __hemx_rows: ::std::vec::Vec<(::std::string::String, ::hemx::Html)> = __hemx_rows.into_iter().map(|__hemx_row| (__hemx_row.hemx_key(), super::render(__hemx_row))).collect();
{inner}        let __hemx_order: ::std::vec::Vec<::std::string::String> = __hemx_rows.iter().map(|(__hemx_key, _)| __hemx_key.clone()).collect();
{inner}        self.slot.rows_html(__hemx_order, __hemx_rows)
{inner}    }}
{inner}    /// Change the collection from the key order `before` to `after`, with the
{inner}    /// rows that are new or changed. Only the steps between the two orders and
{inner}    /// those rows travel; a client in any other state rejects it.
{inner}    pub fn transition<'a, B, A, V>(self, __hemx_before: impl ::std::iter::IntoIterator<Item = B>, __hemx_after: impl ::std::iter::IntoIterator<Item = A>, __hemx_changed: impl ::std::iter::IntoIterator<Item = &'a V>) -> ::hemx::advanced::Effect where B: ::std::string::ToString, A: ::std::string::ToString, V: 'a + ::hemplate::Hemplate + ::hemx::KeyedPartial {{
{inner}        self.slot.rows_delta_html(__hemx_before, __hemx_after, __hemx_changed.into_iter().map(|__hemx_row| (__hemx_row.hemx_key(), super::render(__hemx_row))))
{inner}    }}
{inner}}}
{inner}impl<T> ::hemx::GeneratedTarget for CollectionTarget<T> {{
{inner}    fn __hemx_resource_id(self) -> ::hemx::advanced::ResourceId {{ self.slot.id() }}
{inner}}}
"#));
        // Both capabilities patch one existing row; only collections declare rows.
        for (target, field) in [
            ("KeyedSlotTarget<::std::string::String, T>", "slot"),
            ("CollectionTarget<T>", "rows"),
        ] {
            out.push_str(&format!(r#"{inner}impl<T> {target} {{
{inner}    pub fn replace(self, __hemx_view: impl ::hemplate::Hemplate + ::hemx::KeyedPartial) -> ::hemx::advanced::Effect {{ self.{field}.replace_html(__hemx_view.hemx_key(), super::render(&__hemx_view)) }}
{inner}    pub fn replace_keyed(self, __hemx_key: impl ::std::string::ToString, __hemx_view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{ self.{field}.replace_html(__hemx_key.to_string(), super::render(__hemx_view)) }}
{inner}}}
"#));
        }
        for res in self
            .slots
            .values()
            .filter(|res| component_matches(res, component))
        {
            match res.role {
                SlotRole::Slot => out.push_str(&format!(
                    "{inner}pub const {}: SlotTarget<::std::string::String> = SlotTarget::new(super::advanced::slots::{});\n",
                    res.ident, res.ident
                )),
                SlotRole::Row => out.push_str(&format!(
                    "{inner}pub const {}: KeyedSlotTarget<::std::string::String, ::std::string::String> = KeyedSlotTarget::new(super::advanced::slots::{});\n",
                    res.ident, res.ident
                )),
                SlotRole::Collection => out.push_str(&format!(
                    "{inner}pub const {}: CollectionTarget<::std::string::String> = CollectionTarget::new(super::advanced::slots::{}, ::hemx::advanced::KeyedSlot::new({}));\n",
                    res.ident, res.ident, res.id
                )),
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

    pub(super) fn push_lowering_api(
        &self,
        out: &mut String,
        component: Option<&str>,
        indent: usize,
    ) {
        let pad = "    ".repeat(indent);
        // Authoring names are globally unique, so every render uses the complete table.
        let table_prefix = if component.is_some() { "super::" } else { "" };
        out.push_str(&format!("\n{pad}pub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String {{\n"));
        out.push_str(&format!("{pad}    "));
        out.push_str(&format!(
            "{table_prefix}__hemx_lower_html(html.as_ref(), {table_prefix}__HEMX_LOWERING_TABLE)\n"
        ));
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
            "{pad}pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html {{ render(view) }}\n\n"
        ));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html {{ render(view) }}\n\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect {{\n"));
        out.push_str(&format!("{pad}    slot.html(render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        out.push_str(&format!("{pad}#[doc(hidden)]\n"));
        out.push_str(&format!("{pad}pub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect\n"));
        out.push_str(&format!("{pad}where\n"));
        out.push_str(&format!("{pad}    K: ::std::string::ToString,\n"));
        out.push_str(&format!("{pad}{{\n"));
        out.push_str(&format!("{pad}    slot.replace_html(key, render(view))\n"));
        out.push_str(&format!("{pad}}}\n\n"));
        if component.is_none() {
            self.push_lowering_table(out);
        }
    }

    pub(super) fn push_lowering_table(&self, out: &mut String) {
        let pad = "";
        out.push_str("const __HEMX_LOWERING_TABLE: &[(&str, &str, u32)] = &[\n");
        for res in self.slots.values() {
            let attr = if res.role == SlotRole::Collection {
                "data-hemx-collection"
            } else {
                "data-hemx-slot"
            };
            out.push_str(&format!(
                "    ({}, {}, {}),\n",
                rust_str(attr),
                rust_str(&res.ident),
                res.id
            ));
        }
        for res in self.handles.values() {
            out.push_str(&format!(
                "{pad}    (\"data-hemx-handle\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        for form in self.forms.values() {
            let res = &form.resource;
            out.push_str(&format!(
                "{pad}    (\"data-hemx-form\", {}, {}),\n",
                rust_str(&res.ident),
                res.id
            ));
        }
        out.push_str(&format!("{pad}];\n"));
    }

    pub(super) fn push_lowering_helpers(&self, out: &mut String) {
        out.push_str(r#"
fn __hemx_lower_html(html: &str, table: &[(&str, &str, u32)]) -> ::std::string::String {
    let mut out = html.to_owned();
    for (attr, name, id) in table {
        let runtime_attr = match *attr {
            "data-hemx-slot" | "data-hemx-collection" => "data-hemx-resource",
            "data-hemx-handle" => "data-hemx-action",
            "data-hemx-form" => "data-hemx-resource",
            "data-hemx-revealed" => "data-hemx-revealed",
            _ => continue,
        };
        out = __hemx_replace_attr(out, attr, name, runtime_attr, *id);
    }
    out = __hemx_lower_static_attr(out, "data-hemx-key", "data-hemx-key");
    out = __hemx_lower_static_attr(out, "h-key", "data-hemx-key");
    __hemx_inject_handle_inputs(out)
}

fn __hemx_replace_attr(mut html: ::std::string::String, attr: &str, name: &str, runtime_attr: &str, id: u32) -> ::std::string::String {
    let replacement = if attr == "data-hemx-collection" {
        ::std::format!("{runtime_attr}=\"{id}\" data-hemx-collection=\"\"")
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

        let Some(handle_id) = __hemx_attr(opening, "data-hemx-action") else {
            continue;
        };
        let form_body_end = rest.find("</form>").unwrap_or(rest.len());
        let form_body = &rest[..form_body_end];
        if !__hemx_has_field(form_body, "__h") {
            out.push_str(&::std::format!("<input type=\"hidden\" name=\"__h\" value=\"{}\">", handle_id));
        }
        // The page's build travels with the form so native and enhanced submissions
        // are both checked against the server's build.
        if !__hemx_has_field(form_body, "__b") {
            out.push_str(&::std::format!("<input type=\"hidden\" name=\"__b\" value=\"{}\">", BUILD_FINGERPRINT.0));
        }
    }
    out.push_str(rest);
    out
}

fn __hemx_has_field(html: &str, name: &str) -> bool {
    html.contains(&::std::format!("name=\"{name}\"")) || html.contains(&::std::format!("name='{name}'"))
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

    pub(super) fn component_names(&self) -> Vec<String> {
        let mut components = Vec::new();
        for component in self
            .slots
            .values()
            .map(|res| &res.component)
            .chain(self.handles.values().map(|res| &res.component))
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

    pub(super) fn syms(&self) -> String {
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

    pub(super) fn fingerprint_parts(&self) -> Vec<u32> {
        let mut parts = vec![
            SURFACE_SCHEMA_VERSION,
            EFFECT_BATCH_ABI_VERSION,
            RUNTIME_ABI_VERSION,
        ];

        for res in self.slots.values() {
            parts.push(0);
            parts.push(res.id);
            parts.push(res.role.fingerprint_part());
        }
        for res in self.handles.values() {
            parts.push(1);
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
pub(super) fn insert_resource<'a>(
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

pub(super) fn make_resource(
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
        role: SlotRole::Slot,
        id,
    })
}

pub(super) fn collect_input_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !root.exists() {
        return Ok(paths);
    }
    collect_input_files_into(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

pub(super) fn collect_input_files_into(dir: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
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

pub(super) fn static_attr(attrs: &[SurfaceAttribute], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.origin == AttributeOrigin::Static && attr.name == name)
        .and_then(|attr| attr.value.clone())
}

pub(super) fn push_root_export(
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

pub(super) fn component_matches(res: &Resource, component: Option<&str>) -> bool {
    match component {
        Some(component) => res.component == component,
        None => true,
    }
}

pub(super) fn class_matches(class: &ClassToken, component: Option<&str>) -> bool {
    match component {
        Some(component) => class.component == component,
        None => true,
    }
}

pub(super) fn event_matches(event: &EventToken, component: Option<&str>) -> bool {
    match component {
        Some(component) => event.component == component,
        None => true,
    }
}

pub(super) fn class_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
}

pub(super) fn event_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split_ascii_whitespace()
        .filter(|token| !token.is_empty())
}

pub(super) fn stylesheet_class_tokens(source: &str) -> Vec<&str> {
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

pub(super) fn preceded_by_class_in_compound(bytes: &[u8], dot: usize) -> bool {
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

pub(super) fn is_class_start(byte: u8) -> bool {
    byte == b'_' || byte == b'-' || byte.is_ascii_alphabetic()
}

pub(super) fn is_class_continue(byte: u8) -> bool {
    is_class_start(byte) || byte.is_ascii_digit()
}

pub(super) fn class_ident(token: &str) -> Option<String> {
    rust_ident(&token.replace('-', "_"))
}

/// Handler parameters are explicit: only `data-hemx-param-*` becomes a request
/// field, so ordinary `data-*` metadata never overrides form values.
pub(super) const HANDLE_PARAM_PREFIX: &str = "data-hemx-param-";

pub(super) fn is_handle_param_attr(attr: &SurfaceAttribute) -> bool {
    matches!(
        attr.origin,
        AttributeOrigin::Static | AttributeOrigin::Dynamic
    ) && attr.name.starts_with(HANDLE_PARAM_PREFIX)
}

pub(super) fn data_param_ident(name: &str) -> Option<String> {
    let param = name.strip_prefix(HANDLE_PARAM_PREFIX)?;
    rust_ident(&param.replace('-', "_"))
}

pub(super) fn is_inside_keyed_for(surface: &SurfaceDocument, mut scope: ScopeId) -> bool {
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

pub(super) fn unkeyed_generated_target_diagnostics(
    path: &Path,
    surface: &SurfaceDocument,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (node_scope, target) in generated_targets_with_scope(surface) {
        if target.kind == "collection" {
            continue;
        }
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

pub(super) fn generated_targets(surface: &SurfaceDocument) -> Vec<GeneratedTarget> {
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

pub(super) fn generated_targets_with_scope(
    surface: &SurfaceDocument,
) -> Vec<(ScopeId, GeneratedTarget)> {
    let mut targets = Vec::new();
    for node in &surface.nodes {
        for attr in &node.attrs {
            let Some(kind) = attr.name.strip_prefix("data-hemx-") else {
                continue;
            };
            if !matches!(kind, "slot" | "collection" | "form" | "handle") {
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

pub(super) fn unkeyed_generated_target_diagnostic_for_scope(
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

pub(super) fn unkeyed_generated_target_diagnostic(
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
            "add h-key=\"{pattern}.id\" to that h-for; dynamic +data-hemx-key on the child is rendered HTML, not the template fact hemx uses for generated targets"
        ),
    }
}

pub(super) fn canonical_symbol(root: &Path, path: &Path, name: &str) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    format!("{}::{name}", rel.to_string_lossy().replace('\\', "/"))
}

pub(super) fn component_ident(root: &Path, path: &Path) -> io::Result<String> {
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

pub(super) fn rust_ident(name: &str) -> Option<String> {
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

pub(super) fn stable_id(kind: &str, symbol: &str) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in kind.bytes().chain(*b":").chain(symbol.bytes()) {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

pub(super) fn form_control_is_multiple(kind: &ControlKind) -> bool {
    matches!(kind, ControlKind::Select { multiple: true })
}

pub(super) fn form_control_kind_expr(kind: &ControlKind) -> String {
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

pub(super) fn rust_str(value: &str) -> String {
    format!("{value:?}")
}

pub(super) fn rust_str_opt(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("Some({})", rust_str(value)),
        None => "None".to_string(),
    }
}

pub(super) fn parse_error(path: &Path, err: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("failed to parse {}: {err}", path.display()),
    )
}
