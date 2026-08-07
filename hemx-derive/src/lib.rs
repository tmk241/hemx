use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use std::path::PathBuf;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, parse_quote, Fields, FnArg, GenericArgument, Item, ItemFn, ItemMod,
    ItemStruct, LitStr, Pat, Path, PathArguments, ReturnType, Token, Type,
};

#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let placement = attr.to_string();
    let placement = match handler_placement(&placement) {
        Ok(placement) => placement,
        Err(error) => return error.into_compile_error().into(),
    };
    let function = parse_macro_input!(item as ItemFn);
    let name = function.sig.ident.to_string();

    let syms_path = match handler_syms_path(syms_path()) {
        Ok(path) => path,
        Err(message) => {
            return quote!(
                #function
                compile_error!(#message);
            )
            .into();
        }
    };
    if !syms_contains_handle(&syms_path, &name) {
        let message = format!(
            "unknown hemx handle `{name}`; add `data-hemx-handle=\"{name}\"` to a template or rename this handler"
        );
        return quote!(
            #function
            compile_error!(#message);
        )
        .into();
    }
    if !has_form_param(&function) && !has_non_unit_return(&function) {
        let message = format!(
            "hemx handler `{name}` must accept a form/context parameter or return a value implementing IntoEffect"
        );
        return quote!(
            #function
            compile_error!(#message);
        )
        .into();
    }
    if handle_requires_form(&syms_path, &name) && !has_form_param(&function) {
        let message = format!(
            "hemx handler `{name}` handles a generated form and must accept a typed form argument"
        );
        return quote!(
            #function
            compile_error!(#message);
        )
        .into();
    }
    let missing_params = missing_handle_params(&syms_path, &name, &function);
    if !missing_params.is_empty() {
        let message = format!(
            "hemx handler `{name}` is missing generated param argument(s): {}",
            missing_params.join(", ")
        );
        return quote!(
            #function
            compile_error!(#message);
        )
        .into();
    }

    expand_handler_function(function, placement).into()
}

fn expand_handler_function(
    function: ItemFn,
    placement: HandlerPlacement,
) -> proc_macro2::TokenStream {
    if placement == HandlerPlacement::Server {
        return quote!(#function);
    }
    let has_inputs = match client_handler_has_inputs(&function) {
        Ok(has_inputs) => has_inputs,
        Err(error) => {
            let message = error.to_string();
            return quote!(
                #function
                compile_error!(#message);
            );
        }
    };

    let function_name = &function.sig.ident;
    let export_name = format_ident!("__hemx_client_{function_name}");
    let export_module = format_ident!("__hemx_client_export_{function_name}");
    let invoke_handler = if has_inputs {
        quote!(super::#function_name(event, state))
    } else {
        quote!(super::#function_name())
    };
    quote!(
        #function

        #[cfg(target_arch = "wasm32")]
        mod #export_module {
            use ::hemx::wasm as wasm_bindgen;

            #[::hemx::wasm::wasm_bindgen(js_name = #export_name)]
            #[allow(clippy::too_many_arguments)]
            pub fn invoke(
                event_version: u32,
                event_kind: ::std::string::String,
                event_value: ::std::option::Option<::std::string::String>,
                event_checked: ::std::option::Option<bool>,
                event_key: ::std::option::Option<::std::string::String>,
                state_version: u32,
                encoded_state: ::std::string::String,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::hemx::wasm::JsValue> {
                let (event, state) = ::hemx::wasm::decode_client_inputs(
                    event_version,
                    event_kind,
                    event_value,
                    event_checked,
                    event_key,
                    state_version,
                    encoded_state,
                )
                .map_err(|error| ::hemx::wasm::JsValue::from_str(&error))?;
                Ok(::hemx::wasm::encode_handler_effect(
                    #invoke_handler,
                    crate::ui::BUILD_FINGERPRINT,
                ))
            }
        }
    )
}

#[proc_macro_attribute]
pub fn surface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    inject_surface_include(item)
}

#[proc_macro_attribute]
pub fn form(attr: TokenStream, item: TokenStream) -> TokenStream {
    let form_name = parse_macro_input!(attr as LitStr).value();
    let form_struct = parse_macro_input!(item as ItemStruct);
    let Some(syms_path) = syms_path() else {
        let message = missing_form_generated_files_message();
        return quote!(
            #form_struct
            compile_error!(#message);
        )
        .into();
    };
    let errors = form_contract_errors(&syms_path, &form_name, &form_struct);
    if errors.is_empty() {
        let ident = &form_struct.ident;
        let resource_id =
            form_resource_id(&syms_path, &form_name).expect("checked form exists in hemx.syms");
        let generics = form_impl_generics(&form_struct);
        let decode_fields = form_decode_fields(&syms_path, &form_name, &form_struct);
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
        quote!(
            #form_struct
            impl #impl_generics ::hemx::FormModel for #ident #ty_generics #where_clause {}
            impl #impl_generics ::hemx::FromForm for #ident #ty_generics #where_clause {
                fn from_form_fields(
                    __hemx_fields: &[(String, String)],
                ) -> Result<Self, ::hemx::FormError> {
                    Ok(Self {
                        #(#decode_fields),*
                    })
                }
            }
            impl #impl_generics #ident #ty_generics #where_clause {
                pub const FORM: ::hemx::Form<Self> = ::hemx::Form::new(#resource_id);
            }
        )
        .into()
    } else {
        let message = join_contract_errors(&errors);
        quote!(
            #form_struct
            compile_error!(#message);
        )
        .into()
    }
}

#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let component_name = if attr.is_empty() {
        None
    } else {
        Some(parse_macro_input!(attr as LitStr).value())
    };
    let module = parse_macro_input!(item as ItemMod);
    let Some((_, items)) = &module.content else {
        return quote!(
            #module
            compile_error!("#[hemx::component] must be used on an inline module");
        )
        .into();
    };
    let Some(syms_path) = syms_path() else {
        return quote!(#module).into();
    };
    let component_filter = component_name.as_deref();
    let errors = component_contract_errors(&syms_path, component_filter, items);
    if !errors.is_empty() {
        let message = errors.join("; ");
        return quote!(
            #module
            compile_error!(#message);
        )
        .into();
    }

    let module = match component_name.as_deref() {
        Some(component) => add_component_register_helper(module, component),
        None => module,
    };

    quote!(#module).into()
}

#[proc_macro_attribute]
pub fn app(attr: TokenStream, item: TokenStream) -> TokenStream {
    let components = match Punctuated::<Path, Token![,]>::parse_terminated.parse(attr) {
        Ok(components) => components.into_iter().collect::<Vec<_>>(),
        Err(error) => return error.to_compile_error().into(),
    };
    let function = parse_macro_input!(item as ItemFn);
    match add_app_registry_helper(function, components) {
        Ok(function) => quote!(#function).into(),
        Err(message) => quote!(compile_error!(#message);).into(),
    }
}

fn inject_surface_include(item: TokenStream) -> TokenStream {
    let item = item.to_string();
    let Some(insert_at) = item.rfind('}') else {
        return compile_error("#[hemx::surface] must be used on an inline module");
    };

    let include = surface_include();
    let expanded = format!("{}{}{}", &item[..insert_at], include, &item[insert_at..]);
    expanded
        .parse()
        .unwrap_or_else(|_| compile_error("#[hemx::surface] could not expand this module"))
}

fn surface_include() -> String {
    let Some(path) = generated_path("hemx.generated.rs") else {
        return format!(
            " compile_error!({:?}); ",
            "#[hemx::surface] requires generated hemx files; add hemx_build::app().run()? to build.rs or run inside a Cargo crate"
        );
    };

    if path.exists() {
        format!(" include!({:?}); ", path.display().to_string())
    } else {
        format!(
            " compile_error!({:?}); ",
            format!(
                "#[hemx::surface] could not find generated hemx module; add hemx_build::app().run()? to build.rs or check template generation"
            )
        )
    }
}

fn syms_path() -> Option<PathBuf> {
    generated_path("hemx.syms")
}

fn generated_path(file: &str) -> Option<PathBuf> {
    std::env::var_os("OUT_DIR").map(|out_dir| PathBuf::from(out_dir).join(file))
}

fn has_form_param(function: &ItemFn) -> bool {
    handler_form_model_type(function).is_some()
}

fn form_model_type(ty: &Type) -> Option<Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segments = path.path.segments.iter().collect::<Vec<_>>();
    let form = match segments.as_slice() {
        [form] if form.ident == "Form" => form,
        [hemx, .., form] if hemx.ident == "hemx" && form.ident == "Form" => form,
        _ => return None,
    };
    let PathArguments::AngleBracketed(args) = &form.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    })
}

fn missing_handler_generated_files_message() -> &'static str {
    "#[hemx::handler] requires generated hemx files; add hemx_build::app().run()? to build.rs or run inside a Cargo crate"
}

fn missing_form_generated_files_message() -> &'static str {
    "#[hemx::form] requires generated hemx files; add hemx_build::app().run()? to build.rs or run inside a Cargo crate"
}

fn handler_syms_path(syms_path: Option<PathBuf>) -> Result<PathBuf, &'static str> {
    match syms_path {
        None => Err(missing_handler_generated_files_message()),
        Some(path) if !path.exists() => Err(
            "#[hemx::handler] could not find generated hemx symbols; add hemx_build::app().run()? to build.rs or check template generation",
        ),
        Some(path) => Ok(path),
    }
}

fn join_contract_errors(errors: &[String]) -> String {
    errors.join("; ")
}

fn form_impl_generics(form_struct: &ItemStruct) -> syn::Generics {
    let mut generics = form_struct.generics.clone();
    for ty in form_parser_types(form_struct) {
        generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#ty: ::hemx::FormValue));
    }
    generics
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HandlerPlacement {
    Server,
    Client,
}

fn handler_placement(placement: &str) -> syn::Result<HandlerPlacement> {
    match placement {
        "" => Ok(HandlerPlacement::Server),
        "client" => Ok(HandlerPlacement::Client),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "unsupported hemx handler placement; expected #[hemx::handler] or #[hemx::handler(client)]",
        )),
    }
}

fn client_handler_has_inputs(function: &ItemFn) -> syn::Result<bool> {
    let input_count = function.sig.inputs.len();
    if matches!(input_count, 0 | 2)
        && function.sig.asyncness.is_none()
        && function.sig.unsafety.is_none()
        && function.sig.constness.is_none()
        && function.sig.generics.params.is_empty()
    {
        Ok(input_count == 2)
    } else {
        Err(syn::Error::new_spanned(
            &function.sig,
            format!(
                "client-local hemx handler `{}` must be safe, synchronous, non-generic, and accept either no parameters or `(hemx::wasm::ClientEvent, hemx::wasm::ClientState)`",
                function.sig.ident
            ),
        ))
    }
}

fn handler_form_model_type(function: &ItemFn) -> Option<Type> {
    function.sig.inputs.iter().rev().find_map(|arg| match arg {
        FnArg::Typed(arg) => form_model_type(&arg.ty),
        FnArg::Receiver(_) => None,
    })
}

fn has_non_unit_return(function: &ItemFn) -> bool {
    match &function.sig.output {
        ReturnType::Default => false,
        ReturnType::Type(_, ty) => {
            !matches!(ty.as_ref(), Type::Tuple(tuple) if tuple.elems.is_empty())
        }
    }
}

fn returns_result(output: &ReturnType) -> bool {
    match output {
        ReturnType::Type(_, ty) => is_type_named(ty, "Result"),
        ReturnType::Default => false,
    }
}

fn syms_contains_handle(path: &PathBuf, ident: &str) -> bool {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return true;
    };
    syms.lines().any(|line| {
        let mut fields = line.split('\t');
        matches!(fields.next(), Some("handle"))
            && fields
                .nth(1)
                .is_some_and(|handle_ident| handle_ident == ident)
    })
}

fn handle_requires_form(path: &PathBuf, ident: &str) -> bool {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return false;
    };
    syms.lines().any(|line| {
        let mut fields = line.split('\t');
        matches!(fields.next(), Some("handle_form"))
            && fields
                .next()
                .is_some_and(|handle_ident| handle_ident == ident)
    })
}

#[derive(Debug, Eq, PartialEq)]
struct GeneratedFormField {
    name: String,
    ident: String,
    required: bool,
    multiple: bool,
}

fn form_contract_errors(
    syms_path: &PathBuf,
    form_name: &str,
    form_struct: &ItemStruct,
) -> Vec<String> {
    if !syms_path.exists() {
        return vec![
            "#[hemx::form] could not find generated hemx symbols; add hemx_build::app().run()? to build.rs or check template generation"
                .to_owned(),
        ];
    }
    let expected = form_fields(syms_path, form_name);
    if expected.is_empty() {
        return vec![format!(
            "unknown hemx form `{form_name}`; add data-hemx-form=\"{form_name}\" to a template or rename this form binding"
        )];
    }
    let Fields::Named(fields) = &form_struct.fields else {
        return vec![format!(
            "hemx form `{form_name}` must be a struct with named fields"
        )];
    };
    let actual = fields
        .named
        .iter()
        .filter_map(|field| {
            field
                .ident
                .as_ref()
                .map(|ident| (form_field_name(ident), &field.ty))
        })
        .collect::<Vec<_>>();
    let mut errors = Vec::new();
    for field in expected {
        let Some((_, ty)) = actual.iter().find(|(ident, _)| ident == &field.ident) else {
            errors.push(format!(
                "hemx form `{form_name}` is missing field `{}` for form control `{}`",
                field.ident, field.name
            ));
            continue;
        };
        let optional = is_type_named(ty, "Option");
        let multiple = is_type_named(ty, "Vec");
        if field.required && optional {
            errors.push(format!(
                "hemx form `{form_name}` field `{}` is required in HTML and must not be Option<_>",
                field.ident
            ));
        }
        if field.multiple && !multiple {
            errors.push(format!(
                "hemx form `{form_name}` field `{}` accepts multiple values and must be Vec<_>",
                field.ident
            ));
        }
        if !field.multiple && multiple {
            errors.push(format!(
                "hemx form `{form_name}` field `{}` accepts one value and must not be Vec<_>",
                field.ident
            ));
        }
    }
    errors
}

fn form_parser_types(form_struct: &ItemStruct) -> Vec<Type> {
    let Fields::Named(fields) = &form_struct.fields else {
        return Vec::new();
    };
    fields
        .named
        .iter()
        .map(|field| parser_type(&field.ty).clone())
        .collect()
}

fn form_field_name(ident: &syn::Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

fn form_decode_fields(
    syms_path: &PathBuf,
    form_name: &str,
    form_struct: &ItemStruct,
) -> Vec<proc_macro2::TokenStream> {
    let Fields::Named(fields) = &form_struct.fields else {
        return Vec::new();
    };
    let actual = fields
        .named
        .iter()
        .filter_map(|field| field.ident.as_ref().map(|ident| (ident, &field.ty)))
        .collect::<Vec<_>>();

    form_fields(syms_path, form_name)
        .into_iter()
        .filter_map(|field| {
            let (ident, ty) = actual
                .iter()
                .find(|(ident, _)| form_field_name(ident) == field.ident)?;
            let control_name = field.name;
            let parser = parser_type(ty);
            Some(if field.multiple {
                quote! {
                    #ident: __hemx_fields
                        .iter()
                        .filter_map(|(__hemx_name, __hemx_value)|
                            (__hemx_name == #control_name).then_some(__hemx_value.as_str())
                        )
                        .map(|__hemx_value| {
                            <#parser as ::hemx::FormValue>::parse_form_value(__hemx_value)
                                .map_err(|_| ::hemx::FormError::new(format!("invalid form field `{}`", #control_name)))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
            } else if is_type_named(ty, "Option") {
                quote! {
                    #ident: match __hemx_fields
                        .iter()
                        .find_map(|(__hemx_name, __hemx_value)|
                            (__hemx_name == #control_name).then_some(__hemx_value.as_str())
                        )
                    {
                        Some(__hemx_value) => Some(
                            <#parser as ::hemx::FormValue>::parse_form_value(__hemx_value)
                                .map_err(|_| ::hemx::FormError::new(format!("invalid form field `{}`", #control_name)))?
                        ),
                        None => None,
                    }
                }
            } else {
                quote! {
                    #ident: {
                        let Some(__hemx_value) = __hemx_fields
                            .iter()
                            .find_map(|(__hemx_name, __hemx_value)|
                                (__hemx_name == #control_name).then_some(__hemx_value.as_str())
                            )
                        else {
                            return Err(::hemx::FormError::new(format!("missing form field `{}`", #control_name)));
                        };
                        <#parser as ::hemx::FormValue>::parse_form_value(__hemx_value)
                            .map_err(|_| ::hemx::FormError::new(format!("invalid form field `{}`", #control_name)))?
                    }
                }
            })
        })
        .collect()
}

fn parser_type(ty: &Type) -> &Type {
    generic_inner_type(ty, "Option")
        .or_else(|| generic_inner_type(ty, "Vec"))
        .unwrap_or(ty)
}

fn generic_inner_type<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn form_resource_id(path: &PathBuf, form_name: &str) -> Option<u32> {
    let syms = std::fs::read_to_string(path).ok()?;
    syms.lines().find_map(|line| {
        let mut fields = line.split('\t');
        if !matches!(fields.next(), Some("form")) {
            return None;
        }
        if fields.nth(1)? != form_name {
            return None;
        }
        fields.next()?.parse().ok()
    })
}

fn form_fields(path: &PathBuf, form_name: &str) -> Vec<GeneratedFormField> {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    syms.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if !matches!(fields.next(), Some("form_field")) {
                return None;
            }
            if fields.next()? != form_name {
                return None;
            }
            let name = fields.next()?.to_string();
            Some(GeneratedFormField {
                ident: rust_ident(&name)?,
                name,
                required: fields.next() == Some("true"),
                multiple: fields.next() == Some("true"),
            })
        })
        .collect()
}

fn is_type_named(ty: &Type, name: &str) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == name),
        _ => false,
    }
}

fn rust_ident(name: &str) -> Option<String> {
    let mut out = String::new();
    for ch in name.chars() {
        if ch == '-' || ch == '_' || ch.is_ascii_alphanumeric() {
            out.push(if ch == '-' { '_' } else { ch });
        } else {
            return None;
        }
    }
    let first = out.chars().next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    Some(out)
}

fn missing_handle_params(path: &PathBuf, ident: &str, function: &ItemFn) -> Vec<String> {
    let args = handler_arg_names(function);
    handle_params(path, ident)
        .into_iter()
        .filter(|param| !args.contains(param))
        .collect()
}

fn handle_params(path: &PathBuf, ident: &str) -> Vec<String> {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    syms.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if !matches!(fields.next(), Some("handle_param")) {
                return None;
            }
            if fields.next()? != ident {
                return None;
            }
            fields.next().map(ToOwned::to_owned)
        })
        .collect()
}

fn handler_arg_names(function: &ItemFn) -> Vec<String> {
    function
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(arg) => match arg.pat.as_ref() {
                Pat::Ident(ident) => Some(ident.ident.to_string()),
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .collect()
}

fn add_app_registry_helper(mut function: ItemFn, components: Vec<Path>) -> Result<ItemFn, String> {
    if components.is_empty() {
        return Err("#[hemx::app] requires component registry module(s), for example #[hemx::app(todo_handlers, auth_handlers)]".to_owned());
    }
    let Some(state) = function.sig.inputs.iter().find_map(|arg| match arg {
        FnArg::Typed(arg) => match arg.pat.as_ref() {
            Pat::Ident(ident) => Some(ident.ident.clone()),
            _ => None,
        },
        FnArg::Receiver(_) => None,
    }) else {
        return Err(
            "#[hemx::app] must be used on a registry function with a named app state argument"
                .to_owned(),
        );
    };
    let body = function.block;
    function.block = syn::parse2(quote!({
        let __hemx_registry = (|| #body)();
        #(
            let __hemx_registry = #components::register_with_state(
                __hemx_registry,
                ::hemx_axum::State(#state.clone()),
            );
        )*
        __hemx_registry
    }))
    .expect("generated app registry helper parses");
    Ok(function)
}

struct ComponentHandler {
    ident: syn::Ident,
    is_async: bool,
    returns_result: bool,
    typed_arg_count: usize,
}

fn add_component_register_helper(mut module: ItemMod, component: &str) -> ItemMod {
    let Some((_, items)) = &mut module.content else {
        return module;
    };
    if items.iter().any(|item| match item {
        Item::Fn(function) => {
            function.sig.ident == "register" || function.sig.ident == "register_with_state"
        }
        _ => false,
    }) {
        return module;
    }
    let component_ident = format_ident!("{}", component);
    let handlers = component_handler_idents(items);
    let Some(state_ty) = component_state_type(items) else {
        return module;
    };
    let calls = handlers
        .iter()
        .map(|handler| component_registration_call(handler, &component_ident));
    let register: Item = syn::parse2(quote! {
        pub fn register(
            registry: ::hemx_axum::StateHandlerRegistry<#state_ty>,
        ) -> ::hemx_axum::StateHandlerRegistry<#state_ty>
        where
            #state_ty: Clone + Send + Sync + 'static,
        {
            registry #(#calls)*
        }
    })
    .expect("generated component register helper parses");
    let calls = handlers
        .iter()
        .map(|handler| component_registration_call(handler, &component_ident));
    let register_with_state: Item = syn::parse2(quote! {
        pub fn register_with_state(
            registry: ::hemx_axum::HandlerRegistry,
            state: #state_ty,
        ) -> ::hemx_axum::HandlerRegistry
        where
            #state_ty: Clone + Send + Sync + 'static,
        {
            registry
                .with_state(state)
                #(#calls)*
                .into_registry()
        }
    })
    .expect("generated component state register helper parses");
    items.push(register);
    items.push(register_with_state);
    module
}

fn component_registration_call(
    handler: &ComponentHandler,
    component_ident: &syn::Ident,
) -> TokenStream2 {
    let ident = &handler.ident;
    if handler.typed_arg_count == 1 && handler.is_async && handler.returns_result {
        quote!(.on_state_async_result(super::#component_ident::#ident, #ident))
    } else if handler.typed_arg_count == 1 && handler.returns_result {
        quote!(.on_state_result(super::#component_ident::#ident, #ident))
    } else if handler.typed_arg_count == 1 && handler.is_async {
        quote!(.on_state_async(super::#component_ident::#ident, #ident))
    } else if handler.typed_arg_count == 1 {
        quote!(.on_state(super::#component_ident::#ident, #ident))
    } else if handler.is_async && handler.returns_result {
        quote!(.on_async_result(super::#component_ident::#ident, #ident))
    } else if handler.returns_result {
        quote!(.on_result(super::#component_ident::#ident, #ident))
    } else if handler.is_async {
        quote!(.on_async(super::#component_ident::#ident, #ident))
    } else {
        quote!(.on(super::#component_ident::#ident, #ident))
    }
}

fn component_contract_errors(
    path: &PathBuf,
    component: Option<&str>,
    items: &[Item],
) -> Vec<String> {
    let generated = syms_handles(path, component);
    let implemented = component_handler_names(items);
    let mut errors = Vec::new();

    if let Some(component) = component.filter(|_| !implemented.is_empty() && generated.is_empty()) {
        let available = syms_components(path);
        let repair = if available.is_empty() {
            "no generated components with handles are available; add a data-hemx-handle to the component template or check build.rs generation".to_owned()
        } else {
            format!("available generated components: {}", available.join(", "))
        };
        errors.push(format!(
            "#[hemx::component({component:?})] does not match any generated handles; {repair}"
        ));
    }

    let ambiguous = duplicate_names(&generated);
    if !ambiguous.is_empty() {
        errors.push(format!(
            "#[hemx::component] ambiguous generated handle name(s): {}; make handle names unique for this component before generated registration",
            ambiguous.join(", ")
        ));
    }

    let missing = generated
        .iter()
        .filter(|handle| !implemented.contains(handle))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        errors.push(format!(
            "#[hemx::component] missing handler implementation(s): {}",
            missing.join(", ")
        ));
    }

    let extras = implemented
        .iter()
        .filter(|handler| !generated.contains(handler))
        .cloned()
        .collect::<Vec<_>>();
    if !extras.is_empty() {
        errors.push(format!(
            "#[hemx::component] handler(s) not declared by this component's generated handles: {}; move them to the matching component module or add data-hemx-handle in .heml",
            extras.join(", ")
        ));
    }

    errors
}

#[cfg(test)]
fn missing_component_handlers(
    path: &PathBuf,
    component: Option<&str>,
    items: &[Item],
) -> Vec<String> {
    component_contract_errors(path, component, items)
        .into_iter()
        .find_map(|error| {
            error
                .strip_prefix("#[hemx::component] missing handler implementation(s): ")
                .map(|missing| missing.split(", ").map(ToOwned::to_owned).collect())
        })
        .unwrap_or_default()
}

fn duplicate_names(names: &[String]) -> Vec<String> {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for name in names {
        *counts.entry(name.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name.to_owned())
        .collect()
}

fn component_state_type(items: &[Item]) -> Option<Type> {
    items.iter().find_map(|item| match item {
        Item::Fn(function) if has_handler_attr(function) => {
            function.sig.inputs.iter().find_map(|arg| match arg {
                FnArg::Typed(arg) => Some((*arg.ty).clone()),
                FnArg::Receiver(_) => None,
            })
        }
        _ => None,
    })
}

fn component_handler_names(items: &[Item]) -> Vec<String> {
    component_handler_idents(items)
        .into_iter()
        .map(|handler| handler.ident.to_string())
        .collect()
}

fn component_handler_idents(items: &[Item]) -> Vec<ComponentHandler> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_handler_attr(function) => Some(ComponentHandler {
                ident: function.sig.ident.clone(),
                is_async: function.sig.asyncness.is_some(),
                returns_result: returns_result(&function.sig.output),
                typed_arg_count: function
                    .sig
                    .inputs
                    .iter()
                    .filter(|arg| matches!(arg, FnArg::Typed(_)))
                    .count(),
            }),
            _ => None,
        })
        .collect()
}

fn has_handler_attr(function: &ItemFn) -> bool {
    function.attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "handler")
    })
}

fn syms_handles(path: &PathBuf, component: Option<&str>) -> Vec<String> {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    syms.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if !matches!(fields.next(), Some("handle")) {
                return None;
            }
            let symbol = fields.next()?;
            if let Some(component) = component {
                if symbol_component(symbol) != Some(component) {
                    return None;
                }
            }
            fields.next().map(ToOwned::to_owned)
        })
        .collect()
}

fn syms_components(path: &PathBuf) -> Vec<String> {
    let Ok(syms) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut components = syms
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            matches!(fields.next(), Some("handle"))
                .then(|| fields.next())
                .flatten()
                .and_then(symbol_component)
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    components.sort_unstable();
    components.dedup();
    components
}

fn symbol_component(symbol: &str) -> Option<&str> {
    let path = symbol.split_once("::")?.0;
    path.rsplit_once('/')
        .map_or(path, |(_, stem)| stem)
        .strip_suffix(".heml")
}

fn compile_error(message: &str) -> TokenStream {
    quote!(compile_error!(#message);).into()
}

#[cfg(test)]
mod tests {
    use super::{
        add_app_registry_helper, add_component_register_helper, client_handler_has_inputs,
        component_contract_errors, component_handler_names, component_registration_call,
        expand_handler_function, form_contract_errors, form_decode_fields, form_fields,
        form_impl_generics, form_model_type, form_resource_id, generic_inner_type, handle_params,
        handle_requires_form, handler_arg_names, handler_form_model_type, handler_placement,
        handler_syms_path, has_form_param, has_non_unit_return, is_type_named,
        join_contract_errors, missing_component_handlers, missing_form_generated_files_message,
        missing_handle_params, missing_handler_generated_files_message, parser_type,
        returns_result, rust_ident, symbol_component, syms_components, syms_contains_handle,
        syms_handles, ComponentHandler, HandlerPlacement,
    };
    use quote::{quote, ToTokens};
    use syn::{parse_quote, ItemFn, Type};

    #[test]
    fn handler_attribute_parses_server_and_client_modes_exactly() {
        assert_eq!(handler_placement("").unwrap(), HandlerPlacement::Server);
        assert_eq!(
            handler_placement("client").unwrap(),
            HandlerPlacement::Client
        );
        for invalid in ["server", " client", "client ", "CLIENT"] {
            assert_eq!(
                handler_placement(invalid).unwrap_err().to_string(),
                "unsupported hemx handler placement; expected #[hemx::handler] or #[hemx::handler(client)]"
            );
        }

        let no_inputs: ItemFn = parse_quote!(
            fn save() {}
        );
        let two_inputs: ItemFn = parse_quote!(
            fn save(event: Event, state: State) {}
        );
        assert!(!client_handler_has_inputs(&no_inputs).unwrap());
        assert!(client_handler_has_inputs(&two_inputs).unwrap());

        let server =
            expand_handler_function(no_inputs.clone(), HandlerPlacement::Server).to_string();
        assert_eq!(
            server,
            quote!(
                fn save() {}
            )
            .to_string()
        );
        let no_input_client =
            expand_handler_function(no_inputs.clone(), HandlerPlacement::Client).to_string();
        assert!(no_input_client.contains("super :: save ()"));
        assert!(!no_input_client.contains("super :: save (event , state)"));
        let input_client =
            expand_handler_function(two_inputs.clone(), HandlerPlacement::Client).to_string();
        assert!(input_client.contains("super :: save (event , state)"));
        assert!(!input_client.contains("super :: save ()"));

        for invalid in [
            parse_quote!(
                fn save(event: Event) {}
            ),
            parse_quote!(
                fn save(a: A, b: B, c: C) {}
            ),
            parse_quote!(
                async fn save() {}
            ),
            parse_quote!(
                unsafe fn save() {}
            ),
            parse_quote!(
                const fn save() {}
            ),
            parse_quote!(
                fn save<T>() {}
            ),
        ] {
            assert_eq!(
                client_handler_has_inputs(&invalid).unwrap_err().to_string(),
                "client-local hemx handler `save` must be safe, synchronous, non-generic, and accept either no parameters or `(hemx::wasm::ClientEvent, hemx::wasm::ClientState)`"
            );
            let expanded = expand_handler_function(invalid, HandlerPlacement::Client).to_string();
            assert!(expanded.contains("compile_error !"));
            assert!(expanded.contains("client-local hemx handler"));
        }
    }

    #[test]
    fn generated_file_and_form_helpers_preserve_exact_contracts() {
        assert_eq!(
            missing_handler_generated_files_message(),
            "#[hemx::handler] requires generated hemx files; add hemx_build::app().run()? to build.rs or run inside a Cargo crate"
        );
        assert_eq!(
            missing_form_generated_files_message(),
            "#[hemx::form] requires generated hemx files; add hemx_build::app().run()? to build.rs or run inside a Cargo crate"
        );
        assert_eq!(
            handler_syms_path(None).unwrap_err(),
            missing_handler_generated_files_message()
        );
        let missing = std::env::temp_dir().join("hemx-derive-missing-symbols");
        assert_eq!(
            handler_syms_path(Some(missing)).unwrap_err(),
            "#[hemx::handler] could not find generated hemx symbols; add hemx_build::app().run()? to build.rs or check template generation"
        );
        let existing = std::env::current_exe().unwrap();
        assert_eq!(handler_syms_path(Some(existing.clone())).unwrap(), existing);
        assert_eq!(
            join_contract_errors(&["first".into(), "second".into()]),
            "first; second"
        );

        let form: syn::ItemStruct = parse_quote!(
            struct Profile<T> {
                name: String,
                tags: Vec<T>,
            }
        );
        let generics = form_impl_generics(&form);
        let where_clause = generics
            .where_clause
            .as_ref()
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(where_clause.contains("String : :: hemx :: FormValue"));
        assert!(where_clause.contains("T : :: hemx :: FormValue"));
    }

    #[test]
    fn generated_form_symbol_lookup_is_exact_and_fail_closed() {
        let path =
            std::env::temp_dir().join(format!("hemx-derive-form-symbols-{}", std::process::id()));
        std::fs::write(
            &path,
            "hemx-syms-v1\nform\tprofile.heml::profile\tprofile\t42\nform\nform\tbroken\nform\tmissing-id\tmissing-id\nform\tother.heml::other\tother\tbad\nform_field\tprofile\tname\ttrue\tfalse\nform_field\nform_field\tprofile\nform_field\tprofile\tbad name\tfalse\tfalse\nform_field\tprofile\ttags\tfalse\ttrue\nform_field\tprofile\tbad-name\tfalse\tfalse\nform_field\tother\tignored\tfalse\tfalse\n",
        )
        .unwrap();
        assert_eq!(form_resource_id(&path, "profile"), Some(42));
        assert_eq!(form_resource_id(&path, "missing"), None);
        assert_eq!(form_resource_id(&path, "broken"), None);
        assert_eq!(form_resource_id(&path, "missing-id"), None);
        assert_eq!(form_resource_id(&path, "other"), None);

        let fields = form_fields(&path, "profile");
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].ident, "name");
        assert!(fields[0].required);
        assert!(!fields[0].multiple);
        assert_eq!(fields[1].ident, "tags");
        assert!(!fields[1].required);
        assert!(fields[1].multiple);
        assert_eq!(fields[2].ident, "bad_name");
        assert!(form_fields(&path, "missing").is_empty());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(form_resource_id(&path, "profile"), None);
        assert!(form_fields(&path, "profile").is_empty());

        let string: Type = parse_quote!(String);
        let qualified: Type = parse_quote!(std::string::String);
        let reference: Type = parse_quote!(&String);
        assert!(is_type_named(&string, "String"));
        assert!(is_type_named(&qualified, "String"));
        assert!(!is_type_named(&string, "Vec"));
        assert!(!is_type_named(&reference, "String"));
    }

    #[test]
    fn handler_type_helpers_recognize_only_the_public_form_and_result_shapes() {
        let bare: Type = parse_quote!(Form<String>);
        let qualified: Type = parse_quote!(hemx::Form<crate::Input>);
        let wrong_module: Type = parse_quote!(other::Form<String>);
        let missing_model: Type = parse_quote!(hemx::Form);
        let unrelated: Type = parse_quote!(String);
        assert_eq!(quote!(#bare).to_string(), "Form < String >");
        assert_eq!(
            quote!(#qualified).to_string(),
            "hemx :: Form < crate :: Input >"
        );
        assert!(form_model_type(&bare).is_some());
        assert!(form_model_type(&qualified).is_some());
        assert!(form_model_type(&wrong_module).is_none());
        assert!(form_model_type(&missing_model).is_none());
        assert!(form_model_type(&unrelated).is_none());

        let function: ItemFn = parse_quote!(
            fn save(
                first: hemx::Form<crate::First>,
                value: String,
                last: Form<crate::Last>,
            ) -> Result<(), Error> {
                unimplemented!()
            }
        );
        assert!(quote!(#function)
            .to_string()
            .contains("last : Form < crate :: Last >"));
        assert!(quote!(#function)
            .to_string()
            .contains("first : hemx :: Form < crate :: First >"));
        assert_eq!(
            handler_form_model_type(&function)
                .map(|ty| quote!(#ty).to_string())
                .as_deref(),
            Some("crate :: Last")
        );
        assert!(returns_result(&function.sig.output));

        let no_result: ItemFn = parse_quote!(
            fn save() -> String {
                String::new()
            }
        );
        let no_return: ItemFn = parse_quote!(
            fn save() {}
        );
        assert!(!returns_result(&no_result.sig.output));
        assert!(!returns_result(&no_return.sig.output));
    }

    #[test]
    fn form_contract_diagnostics_accumulate_all_mismatches() {
        let path = std::env::temp_dir().join(format!(
            "hemx-derive-form-contract-errors-{}",
            std::process::id()
        ));
        let missing: syn::ItemStruct = parse_quote!(
            struct Profile {
                first: String,
            }
        );
        assert_eq!(
            form_contract_errors(&path, "profile", &missing),
            vec!["#[hemx::form] could not find generated hemx symbols; add hemx_build::app().run()? to build.rs or check template generation"]
        );

        std::fs::write(
            &path,
            "hemx-syms-v1\nform\tprofile.heml::profile\tprofile\t42\nform_field\tprofile\tfirst\tfalse\tfalse\nform_field\tprofile\tsecond\ttrue\tfalse\nform_field\tprofile\ttags\tfalse\ttrue\nform_field\tprofile\trequired_name\ttrue\tfalse\n",
        )
        .unwrap();
        assert_eq!(
            form_contract_errors(&path, "unknown", &missing),
            vec!["unknown hemx form `unknown`; add data-hemx-form=\"unknown\" to a template or rename this form binding"]
        );
        let tuple: syn::ItemStruct = parse_quote!(
            struct Profile(String);
        );
        assert_eq!(
            form_contract_errors(&path, "profile", &tuple),
            vec!["hemx form `profile` must be a struct with named fields"]
        );
        let mismatched: syn::ItemStruct = parse_quote!(
            struct Profile {
                first: Vec<String>,
                tags: String,
                required_name: Option<String>,
            }
        );
        assert_eq!(
            form_contract_errors(&path, "profile", &mismatched),
            vec![
                "hemx form `profile` field `first` accepts one value and must not be Vec<_>",
                "hemx form `profile` is missing field `second` for form control `second`",
                "hemx form `profile` field `tags` accepts multiple values and must be Vec<_>",
                "hemx form `profile` field `required_name` is required in HTML and must not be Option<_>",
            ]
        );

        let decode_struct: syn::ItemStruct = parse_quote!(
            struct Profile {
                first: Option<String>,
                tags: Vec<String>,
                required_name: String,
            }
        );
        let decoded = form_decode_fields(&path, "profile", &decode_struct)
            .into_iter()
            .map(|tokens| tokens.to_string())
            .collect::<Vec<_>>();
        assert_eq!(decoded.len(), 3);
        assert!(decoded[0].contains("Some (__hemx_value) => Some"));
        assert!(decoded[0].contains("None => None"));
        assert!(decoded[1].contains("collect :: < Result < Vec < _ > , _ >> () ?"));
        assert!(decoded[2].contains("missing form field"));

        let option: Type = parse_quote!(Option<String>);
        let qualified_vec: Type = parse_quote!(std::vec::Vec<u64>);
        let plain: Type = parse_quote!(String);
        assert_eq!(
            generic_inner_type(&option, "Option")
                .unwrap()
                .to_token_stream()
                .to_string(),
            "String"
        );
        assert_eq!(
            generic_inner_type(&qualified_vec, "Vec")
                .unwrap()
                .to_token_stream()
                .to_string(),
            "u64"
        );
        assert!(generic_inner_type(&option, "Vec").is_none());
        assert!(generic_inner_type(&plain, "String").is_none());
        let empty_path = Type::Path(syn::TypePath {
            qself: None,
            path: syn::Path {
                leading_colon: None,
                segments: Default::default(),
            },
        });
        assert!(generic_inner_type(&empty_path, "Vec").is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn syms_lookup_matches_handle_ident() {
        let path = std::env::temp_dir().join("hemx-derive-syms-test.syms");
        std::fs::write(
            &path,
            "hemx-syms-v1\nslot\ttemplates/a.heml::count\tcount\t1\nhandle\ttemplates/z.heml::archive\tarchive\t3\nhandle\ttemplates/a.heml::create\tcreate\t2\nhandle\ttemplates/a.heml::delete\tdelete\t4\nhandle\ttemplates/a.heml::create\tcreate-again\t5\nhandle\thas-no-component\tignored\t6\nhandle\nhandle_form\tcreate\tnew_todo\nhandle_param\tcreate\ttodo_id\n",
        )
        .unwrap();

        assert!(syms_contains_handle(&path, "create"));
        assert!(!syms_contains_handle(&path, "missing"));
        assert!(handle_requires_form(&path, "create"));
        assert!(!handle_requires_form(&path, "missing"));
        assert_eq!(handle_params(&path, "create"), vec!["todo_id"]);
        assert!(handle_params(&path, "missing").is_empty());
        assert_eq!(
            syms_handles(&path, Some("a")),
            vec!["create", "delete", "create-again"]
        );
        assert_eq!(
            syms_handles(&path, None),
            vec!["archive", "create", "delete", "create-again", "ignored"]
        );
        assert_eq!(syms_components(&path), vec!["a", "z"]);
        assert_eq!(symbol_component("templates/a.heml::create"), Some("a"));
        assert_eq!(symbol_component("a.heml::create"), Some("a"));
        assert_eq!(symbol_component("a.html::create"), None);
        assert_eq!(symbol_component("a.heml"), None);

        let _ = std::fs::remove_file(&path);
        assert!(
            syms_contains_handle(&path, "create"),
            "missing generated symbols defer to the dedicated generated-file diagnostic"
        );
        assert!(!handle_requires_form(&path, "create"));
    }

    #[test]
    fn handler_shape_accepts_form_or_effect_return() {
        let with_form = parse_quote!(
            fn save(form: hemx::Form<String>) {}
        );
        let with_return = parse_quote!(
            fn ping() -> impl hemx::IntoEffect {
                hemx::advanced::EffectBatch::default()
            }
        );
        let empty = parse_quote!(
            fn noop() {}
        );

        assert!(has_form_param(&with_form));
        assert!(has_non_unit_return(&with_return));
        assert!(!has_form_param(&empty));
        assert!(!has_non_unit_return(&empty));
    }

    #[test]
    fn form_parser_type_uses_option_and_vec_inner_types() {
        let required: Type = parse_quote!(Email);
        let optional: Type = parse_quote!(Option<Email>);
        let multiple: Type = parse_quote!(Vec<Email>);

        let required_parser = parser_type(&required);
        let optional_parser = parser_type(&optional);
        let multiple_parser = parser_type(&multiple);

        assert_eq!(
            quote!(#required_parser).to_string(),
            quote!(#required).to_string()
        );
        assert_eq!(
            quote!(#optional_parser).to_string(),
            quote!(#required).to_string()
        );
        assert_eq!(
            quote!(#multiple_parser).to_string(),
            quote!(#required).to_string()
        );
    }

    #[test]
    fn form_param_matches_form_type_not_name_suffix() {
        let qualified_form: Type = parse_quote!(hemx::Form<CreateTodo>);
        let imported_form: Type = parse_quote!(Form<CreateTodo>);
        let name_suffix_impostor: Type = parse_quote!(CreateTodoForm);
        let nongeneric_impostor: Type = parse_quote!(Form);
        let foreign_form: Type = parse_quote!(other::Form<CreateTodo>);

        assert!(form_model_type(&qualified_form).is_some());
        assert!(form_model_type(&imported_form).is_some());
        assert!(form_model_type(&name_suffix_impostor).is_none());
        assert!(form_model_type(&nongeneric_impostor).is_none());
        assert!(form_model_type(&foreign_form).is_none());
    }

    #[test]
    fn generated_form_field_names_map_only_to_valid_rust_identifiers() {
        assert_eq!(rust_ident("first-name").as_deref(), Some("first_name"));
        assert_eq!(rust_ident("_private2").as_deref(), Some("_private2"));
        for invalid in ["", "2fast", "with space", "naïve"] {
            assert_eq!(rust_ident(invalid), None, "{invalid:?} must fail closed");
        }
    }

    #[test]
    fn handler_params_match_generated_param_names() {
        let path = std::env::temp_dir().join("hemx-derive-param-test.syms");
        std::fs::write(
            &path,
            "hemx-syms-v1\nhandle_param\tshow\ttodo_id\nhandle_param\tshow\tmode\n",
        )
        .unwrap();
        let complete: ItemFn = parse_quote!(
            fn show(todo_id: String, mode: String) -> impl hemx::IntoEffect {
                hemx::advanced::EffectBatch::default()
            }
        );
        let missing: ItemFn = parse_quote!(
            fn show(todo_id: String) -> impl hemx::IntoEffect {
                hemx::advanced::EffectBatch::default()
            }
        );

        assert!(missing_handle_params(&path, "show", &complete).is_empty());
        assert_eq!(missing_handle_params(&path, "show", &missing), vec!["mode"]);
        assert!(missing_handle_params(&path, "unknown", &missing).is_empty());

        std::fs::write(&path, "handle_param\nhandle_param\tshow\tmode\n").unwrap();
        assert_eq!(handle_params(&path, "show"), vec!["mode"]);

        let patterns: ItemFn = parse_quote! {
            fn patterns(self, named: String, (left, right): (u8, u8), _: bool) {}
        };
        assert_eq!(handler_arg_names(&patterns), vec!["named"]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn component_registration_selects_each_state_async_result_shape() {
        let component_ident: syn::Ident = parse_quote!(todos);
        let expected = [
            (0, false, false, quote!(.on(super::todos::save, save))),
            (0, false, true, quote!(.on_result(super::todos::save, save))),
            (0, true, false, quote!(.on_async(super::todos::save, save))),
            (
                0,
                true,
                true,
                quote!(.on_async_result(super::todos::save, save)),
            ),
            (1, false, false, quote!(.on_state(super::todos::save, save))),
            (
                1,
                false,
                true,
                quote!(.on_state_result(super::todos::save, save)),
            ),
            (
                1,
                true,
                false,
                quote!(.on_state_async(super::todos::save, save)),
            ),
            (
                1,
                true,
                true,
                quote!(.on_state_async_result(super::todos::save, save)),
            ),
        ];
        for (typed_arg_count, is_async, returns_result, expected) in expected {
            let handler = ComponentHandler {
                ident: parse_quote!(save),
                is_async,
                returns_result,
                typed_arg_count,
            };
            assert_eq!(
                component_registration_call(&handler, &component_ident).to_string(),
                expected.to_string(),
                "args={typed_arg_count} async={is_async} result={returns_result}"
            );
        }
    }

    #[test]
    fn component_handlers_match_generated_handles() {
        let path = std::env::temp_dir().join("hemx-derive-component-test.syms");
        std::fs::write(
            &path,
            "hemx-syms-v1\nhandle\ttemplates/a.heml::create\tcreate\t1\nhandle\ttemplates/a.heml::delete\tdelete\t2\nhandle\ttemplates/other.heml::archive\tarchive\t3\n",
        )
        .unwrap();
        let module: syn::ItemMod = parse_quote! {
            mod component {
                #[hemx::handler]
                fn create() -> impl hemx::IntoEffect { hemx::advanced::EffectBatch::default() }

                fn helper() {}
            }
        };
        let (_, items) = module.content.expect("inline module");

        assert_eq!(component_handler_names(&items), vec!["create"]);
        assert_eq!(
            missing_component_handlers(&path, None, &items),
            vec!["delete", "archive"]
        );
        assert_eq!(
            missing_component_handlers(&path, Some("a"), &items),
            vec!["delete"]
        );

        let _ = std::fs::remove_file(&path);
        assert_eq!(
            component_contract_errors(&path, Some("missing"), &items),
            vec![
                "#[hemx::component(\"missing\")] does not match any generated handles; no generated components with handles are available; add a data-hemx-handle to the component template or check build.rs generation",
                "#[hemx::component] handler(s) not declared by this component's generated handles: create; move them to the matching component module or add data-hemx-handle in .heml",
            ]
        );
    }

    #[test]
    fn app_macro_generates_single_registry_entry_point() {
        let function = parse_quote! {
            fn registry(state: std::sync::Arc<App>) -> hemx_axum::HandlerRegistry {
                hemx_axum::interactions(ui::BUILD_FINGERPRINT)
            }
        };
        let function = add_app_registry_helper(
            function,
            vec![parse_quote!(counter_handlers), parse_quote!(todo_handlers)],
        )
        .unwrap();
        let generated = quote!(#function).to_string();

        assert!(
            generated.contains("counter_handlers :: register_with_state"),
            "{generated}"
        );
        assert!(
            generated.contains("todo_handlers :: register_with_state"),
            "{generated}"
        );
        assert!(generated.contains("hemx_axum :: State"), "{generated}");
        assert!(generated.contains("state . clone"), "{generated}");

        let no_components: ItemFn = parse_quote!(
            fn registry(state: App) {}
        );
        assert_eq!(
            add_app_registry_helper(no_components, vec![]).err().unwrap(),
            "#[hemx::app] requires component registry module(s), for example #[hemx::app(todo_handlers, auth_handlers)]"
        );
        let unnamed_state: ItemFn = parse_quote!(
            fn registry((state,): (App,)) {}
        );
        assert_eq!(
            add_app_registry_helper(unnamed_state, vec![parse_quote!(handlers)])
                .err()
                .unwrap(),
            "#[hemx::app] must be used on a registry function with a named app state argument"
        );
    }

    #[test]
    fn component_registration_generation_respects_existing_and_incomplete_modules() {
        let external: syn::ItemMod = parse_quote!(
            mod handlers;
        );
        let unchanged = add_component_register_helper(external.clone(), "todos");
        assert_eq!(
            quote!(#external).to_string(),
            quote!(#unchanged).to_string()
        );

        for existing in ["register", "register_with_state"] {
            let existing_ident = syn::Ident::new(existing, proc_macro2::Span::call_site());
            let module: syn::ItemMod = parse_quote! {
                mod handlers {
                    struct App;
                    #[hemx::handler]
                    fn create(app: App) {}
                    fn #existing_ident() {}
                }
            };
            let generated = add_component_register_helper(module, "todos");
            let items = generated.content.unwrap().1;
            assert_eq!(items.len(), 3, "must preserve existing {existing}");
        }

        let no_state: syn::ItemMod = parse_quote! {
            mod handlers {
                #[hemx::handler]
                fn create() {}
            }
        };
        assert_eq!(
            add_component_register_helper(no_state, "todos")
                .content
                .unwrap()
                .1
                .len(),
            1
        );
        let no_handlers: syn::ItemMod = parse_quote! {
            mod handlers {
                struct App;
                fn helper(app: App) {}
            }
        };
        assert_eq!(
            add_component_register_helper(no_handlers, "todos")
                .content
                .unwrap()
                .1
                .len(),
            2
        );
    }

    #[test]
    fn component_macro_generates_registration_helpers() {
        let module = parse_quote! {
            mod handlers {
                const COMPONENT_KIND: &str = "todos";

                #[hemx::handler]
                fn sync_form(app: super::App, form: super::NewTodo) -> impl hemx::IntoEffect {
                    hemx::EventName::new("sync-form").emit("")
                }

                #[hemx::handler]
                async fn async_state(app: super::App) -> impl hemx::IntoEffect {
                    hemx::EventName::new("async-state").emit("")
                }

                #[hemx::handler]
                fn sync_result(app: super::App) -> Result<impl hemx::IntoEffect, super::Error> {
                    Ok(hemx::EventName::new("sync-result").emit(""))
                }

                #[hemx::handler]
                async fn async_result(app: super::App) -> Result<impl hemx::IntoEffect, super::Error> {
                    Ok(hemx::EventName::new("async-result").emit(""))
                }

                #[hemx::handler]
                async fn async_form_result(app: super::App, form: super::NewTodo) -> Result<impl hemx::IntoEffect, super::Error> {
                    Ok(hemx::EventName::new("async-form-result").emit(""))
                }
            }
        };

        let module = add_component_register_helper(module, "todos");
        let generated = quote!(#module).to_string();

        assert!(generated.contains("register_with_state"), "{generated}");
        assert!(generated.contains("StateHandlerRegistry"), "{generated}");
        assert!(
            generated.contains("super :: todos :: sync_form"),
            "{generated}"
        );
        assert!(generated.contains(". on"), "{generated}");
        assert!(generated.contains(". on_state_async"), "{generated}");
        assert!(generated.contains(". on_state_result"), "{generated}");
        assert!(generated.contains(". on_state_async_result"), "{generated}");
        assert!(generated.contains(". on_async_result"), "{generated}");
    }
}
