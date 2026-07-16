//! `#[derive(StateHash)]` — proc-macro that emits a `StateHash` impl.
//!
//! Why a custom trait instead of `std::hash::Hash`? Floats. `f32` and
//! `f64` don't implement `Hash` because of NaN equality. We want a
//! deterministic byte-identical hash of the engine state that includes
//! float fields, so we define a separate `StateHash` trait whose float
//! impls go through `to_bits()`.
//!
//! The derived impl walks every field in declaration order, calling
//! each field's `StateHash::state_hash`. For enums it hashes the
//! discriminant first, then each variant's fields.
//!
//! Fields omitted from serialization via `#[serde(skip)]` or
//! `#[serde(skip_serializing)]` are represented by an explicit skipped-field
//! marker in `StateHash` too. A field can also opt out of hashing without
//! changing its Serde representation via `#[state_hash(skip)]`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DataEnum, DeriveInput, Field, Fields, Index, Meta, Token, parse_macro_input,
    punctuated::Punctuated, spanned::Spanned,
};

#[proc_macro_derive(StateHash, attributes(state_hash))]
pub fn derive_state_hash(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_state_hash(&input) {
        Ok(expanded) => TokenStream::from(expanded),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_state_hash(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => struct_body(&data.fields)?,
        Data::Enum(data) => enum_body(data)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                ident,
                "StateHash cannot be derived for unions",
            ));
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::robin_util::state_hash::StateHash for #ident #ty_generics #where_clause {
            fn state_hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) {
                #body
            }
        }
    })
}

fn enum_body(data: &DataEnum) -> syn::Result<TokenStream2> {
    let mut arms = Vec::with_capacity(data.variants.len());
    for (disc, variant) in data.variants.iter().enumerate() {
        let variant_ident = &variant.ident;
        let disc = disc as u64;
        let arm = match &variant.fields {
            Fields::Unit => quote! {
                Self::#variant_ident => {
                    ::core::hash::Hasher::write_u64(state, #disc);
                }
            },
            Fields::Named(fields) => {
                let mut patterns = Vec::with_capacity(fields.named.len());
                let mut calls = Vec::with_capacity(fields.named.len());
                for field in &fields.named {
                    let id = field
                        .ident
                        .as_ref()
                        .expect("named field must have an identifier");
                    if is_hash_skipped(field)? {
                        patterns.push(quote! { #id: _ });
                        calls.push(skipped_hash_call());
                    } else {
                        patterns.push(quote! { #id });
                        calls.push(hash_call(quote! { #id }));
                    }
                }
                quote! {
                    Self::#variant_ident { #(#patterns),* } => {
                        ::core::hash::Hasher::write_u64(state, #disc);
                        #(#calls)*
                    }
                }
            }
            Fields::Unnamed(fields) => {
                let mut patterns = Vec::with_capacity(fields.unnamed.len());
                let mut calls = Vec::with_capacity(fields.unnamed.len());
                for (index, field) in fields.unnamed.iter().enumerate() {
                    if is_hash_skipped(field)? {
                        patterns.push(quote! { _ });
                        calls.push(skipped_hash_call());
                    } else {
                        let binding = syn::Ident::new(&format!("__f{index}"), field.span());
                        patterns.push(quote! { #binding });
                        calls.push(hash_call(quote! { #binding }));
                    }
                }
                quote! {
                    Self::#variant_ident( #(#patterns),* ) => {
                        ::core::hash::Hasher::write_u64(state, #disc);
                        #(#calls)*
                    }
                }
            }
        };
        arms.push(arm);
    }

    Ok(quote! {
        match self {
            #(#arms)*
        }
    })
}

/// Whether this field is absent from the serialized snapshot or explicitly
/// excluded with `#[state_hash(skip)]`.
fn is_hash_skipped(field: &Field) -> syn::Result<bool> {
    let mut skipped = false;
    for attr in &field.attrs {
        if attr.path().is_ident("serde") {
            let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            skipped |= metas.iter().any(|meta| {
                meta.path().is_ident("skip") || meta.path().is_ident("skip_serializing")
            });
        } else if attr.path().is_ident("state_hash") {
            let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            for meta in metas {
                if let Meta::Path(path) = &meta
                    && path.is_ident("skip")
                {
                    skipped = true;
                } else {
                    return Err(syn::Error::new_spanned(
                        meta,
                        "unsupported StateHash field attribute; expected #[state_hash(skip)]",
                    ));
                }
            }
        }
    }
    Ok(skipped)
}

fn struct_body(fields: &Fields) -> syn::Result<TokenStream2> {
    let mut calls = Vec::with_capacity(fields.len());
    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                if is_hash_skipped(field)? {
                    calls.push(skipped_hash_call());
                } else {
                    let id = field
                        .ident
                        .as_ref()
                        .expect("named field must have an identifier");
                    calls.push(hash_call(quote! { self.#id }));
                }
            }
        }
        Fields::Unnamed(fields) => {
            for (index, field) in fields.unnamed.iter().enumerate() {
                if is_hash_skipped(field)? {
                    calls.push(skipped_hash_call());
                } else {
                    let index = Index {
                        index: index as u32,
                        span: field.span(),
                    };
                    calls.push(hash_call(quote! { self.#index }));
                }
            }
        }
        Fields::Unit => {}
    }
    Ok(quote! { #(#calls)* })
}

fn hash_call(accessor: TokenStream2) -> TokenStream2 {
    quote! {
        ::robin_util::state_hash::StateHash::state_hash(&#accessor, state);
    }
}

fn skipped_hash_call() -> TokenStream2 {
    quote! {
        ::robin_util::state_hash::hash_skipped_field(state);
    }
}
