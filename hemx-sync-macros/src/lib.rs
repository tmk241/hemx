use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Error, FnArg, ItemFn, Pat, ReturnType};

#[proc_macro_attribute]
pub fn presence(attributes: TokenStream, item: TokenStream) -> TokenStream {
    expand_presence(attributes.into(), item.into())
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand_presence(attributes: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    if !attributes.is_empty() {
        return Err(Error::new(
            proc_macro2::Span::call_site(),
            "#[hemx_sync::presence] does not accept arguments",
        ));
    }

    let mut function: ItemFn = syn::parse2(item)?;
    if let Some(asyncness) = &function.sig.asyncness {
        return Err(Error::new_spanned(
            asyncness,
            "presence projections must be synchronous",
        ));
    }
    if matches!(function.sig.output, ReturnType::Default) {
        return Err(Error::new_spanned(
            &function.sig,
            "presence projections must return impl IntoEffect",
        ));
    }
    let argument = match function.sig.inputs.first() {
        Some(FnArg::Typed(argument)) if function.sig.inputs.len() == 1 => argument,
        _ => {
            return Err(Error::new_spanned(
                &function.sig.inputs,
                "presence projections require exactly one typed presence argument",
            ));
        }
    };
    let argument_name = match argument.pat.as_ref() {
        Pat::Ident(argument) => argument.ident.clone(),
        pattern => {
            return Err(Error::new_spanned(
                pattern,
                "presence projection argument must be a simple identifier",
            ));
        }
    };

    let body = function.block;
    function.sig.output = syn::parse_quote!(-> impl ::hemx_sync::PresenceUpdate);
    function.block = Box::new(syn::parse_quote!({
        let __hemx_sync_channel =
            ::hemx_sync::PresenceScope::presence_channel(&#argument_name);
        let __hemx_sync_effect = (|| #body)();
        ::hemx_sync::PresenceProjection::new(__hemx_sync_channel, __hemx_sync_effect)
    }));
    Ok(quote!(#function))
}

#[cfg(test)]
mod tests {
    use super::expand_presence;
    use quote::quote;

    #[test]
    fn presence_expansion_enforces_the_typed_projection_contract() {
        assert!(expand_presence(quote!(), quote!(not a function)).is_err());

        for (attributes, item, expected) in [
            (
                quote!(unexpected),
                quote!(
                    fn project(scope: Scope) -> Effect {
                        effect()
                    }
                ),
                "#[hemx_sync::presence] does not accept arguments",
            ),
            (
                quote!(),
                quote!(
                    async fn project(scope: Scope) -> Effect {
                        effect()
                    }
                ),
                "presence projections must be synchronous",
            ),
            (
                quote!(),
                quote!(
                    fn project(scope: Scope) {}
                ),
                "presence projections must return impl IntoEffect",
            ),
            (
                quote!(),
                quote!(
                    fn project() -> Effect {
                        effect()
                    }
                ),
                "presence projections require exactly one typed presence argument",
            ),
            (
                quote!(),
                quote!(
                    fn project(a: Scope, b: Scope) -> Effect {
                        effect()
                    }
                ),
                "presence projections require exactly one typed presence argument",
            ),
            (
                quote!(),
                quote!(
                    fn project((scope,): (Scope,)) -> Effect {
                        effect()
                    }
                ),
                "presence projection argument must be a simple identifier",
            ),
        ] {
            assert_eq!(
                expand_presence(attributes, item).unwrap_err().to_string(),
                expected
            );
        }

        let expanded = expand_presence(
            quote!(),
            quote!(
                pub fn project(scope: Scope) -> Effect {
                    effect(scope)
                }
            ),
        )
        .unwrap()
        .to_string();
        assert!(expanded.contains("pub fn project"));
        assert!(expanded.contains("impl :: hemx_sync :: PresenceUpdate"));
        assert!(expanded.contains("PresenceScope :: presence_channel (& scope)"));
        assert!(expanded.contains("PresenceProjection :: new"));
        assert!(expanded.contains("effect (scope)"));
    }
}
