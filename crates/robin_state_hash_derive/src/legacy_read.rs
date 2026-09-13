//! `#[derive(LegacyRead)]` — positional field-by-field decoding of
//! original-game binary layouts through `robin_data_io::legacy_io`.
//!
//! The generated `LegacyRead::read` reads every named field in declaration
//! order (declaration order IS wire order) and builds `Self`. Each field is
//! bound to a local of the same name first, so attribute expressions can
//! refer to earlier fields, to `reader` and to the decode context `ctx`.
//!
//! Struct attributes:
//! - `ctx = Type` — implement `LegacyRead<Type>` only (default: every `C`).
//! - `fingerprint = EXPR, expected = DESC` — read and verify a 16-byte
//!   signature as field `fingerprint` before the first field (`DESC` is a
//!   `&'static str` expression, e.g. a literal or `ctx.description`).
//!
//! Field attributes (the wire name defaults to the field name):
//! - `name = "wire"` — error-context name.
//! - `fingerprint = EXPR, expected = DESC` — signature before this field;
//!   `fingerprint_name = "n"` overrides its field name.
//! - `offset` — `reader.offset()` here, consumes no bytes.
//! - `value = EXPR` — computed value, consumes no bytes.
//! - `read = EXPR` — custom read; `EXPR` yields `LegacyResult<FieldType>`.
//! - `with = path` — `path(reader, "wire")`; with `args(a, b)` it is
//!   `path(reader, "wire", a, b)`.
//! - `bytes` — raw byte array read as one field.
//! - `flatten` — `T::read` in the current scope (no field segment).
//! - `scoped` — `T::read` inside `reader.scope("wire", ..)` (for arrays this
//!   yields `wire.[i]` instead of the default `wire[i]`).
//! - `count_u32 = LIMIT` / `count_u16 = LIMIT` / `len = EXPR` on `Vec<T>` —
//!   count `wire.count`, items `wire[i]`; with `items` the list is instead
//!   scoped as `wire.count` / `wire.items[i]`. `count_name = "n"` overrides
//!   the count field's name.
//! - `when = EXPR` on `Option<T>` — read only when `EXPR` is true.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Expr, Fields, GenericArgument, LitStr, Path,
    PathArguments, Token, Type, parenthesized, parse_quote, punctuated::Punctuated,
    spanned::Spanned,
};

#[derive(Default)]
struct Attrs {
    ctx: Option<Type>,
    name: Option<LitStr>,
    fingerprint: Option<Expr>,
    fingerprint_name: Option<LitStr>,
    expected: Option<Expr>,
    args: Vec<Expr>,
    offset: bool,
    value: Option<Expr>,
    read: Option<Expr>,
    with: Option<Path>,
    bytes: bool,
    flatten: bool,
    scoped: bool,
    count_u32: Option<Expr>,
    count_u16: Option<Expr>,
    len: Option<Expr>,
    count_name: Option<LitStr>,
    items: bool,
    when: Option<Expr>,
}

fn parse_attrs(attrs: &[Attribute]) -> syn::Result<Attrs> {
    let mut out = Attrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("legacy")) {
        attr.parse_nested_meta(|meta| {
            let key = meta.path.get_ident().map(ToString::to_string);
            match key.as_deref() {
                Some("ctx") => out.ctx = Some(meta.value()?.parse()?),
                Some("name") => out.name = Some(meta.value()?.parse()?),
                Some("fingerprint") => out.fingerprint = Some(meta.value()?.parse()?),
                Some("fingerprint_name") => out.fingerprint_name = Some(meta.value()?.parse()?),
                Some("args") => {
                    let content;
                    parenthesized!(content in meta.input);
                    out.args = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                }
                Some("expected") => out.expected = Some(meta.value()?.parse()?),
                Some("value") => out.value = Some(meta.value()?.parse()?),
                Some("read") => out.read = Some(meta.value()?.parse()?),
                Some("with") => out.with = Some(meta.value()?.parse()?),
                Some("count_u32") => out.count_u32 = Some(meta.value()?.parse()?),
                Some("count_u16") => out.count_u16 = Some(meta.value()?.parse()?),
                Some("len") => out.len = Some(meta.value()?.parse()?),
                Some("count_name") => out.count_name = Some(meta.value()?.parse()?),
                Some("when") => out.when = Some(meta.value()?.parse()?),
                Some("offset") => out.offset = true,
                Some("bytes") => out.bytes = true,
                Some("flatten") => out.flatten = true,
                Some("scoped") => out.scoped = true,
                Some("items") => out.items = true,
                _ => return Err(meta.error("unsupported #[legacy(..)] attribute")),
            }
            Ok(())
        })?;
    }
    Ok(out)
}

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(ident, "LegacyRead requires a struct"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            ident,
            "LegacyRead requires named fields",
        ));
    };
    let top = parse_attrs(&input.attrs)?;
    let mut generics = input.generics.clone();
    let ctx: Type = match &top.ctx {
        Some(ctx) => ctx.clone(),
        None => {
            generics.params.push(parse_quote!(__C: ?Sized));
            parse_quote!(__C)
        }
    };
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let mut steps = vec![signature(&top, ident.span())?];
    let mut names = Vec::new();
    for field in &fields.named {
        let attrs = parse_attrs(&field.attrs)?;
        let id = field.ident.as_ref().expect("named field has an identifier");
        let name = attrs
            .name
            .clone()
            .unwrap_or_else(|| LitStr::new(&id.to_string(), id.span()));
        let ty = &field.ty;
        let read = field_read(&attrs, ty, &name, &ctx)?;
        steps.push(signature(&attrs, field.span())?);
        steps.push(quote! { let #id: #ty = #read; });
        names.push(id);
    }

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::robin_data_io::legacy_io::LegacyRead<#ctx>
            for #ident #ty_generics #where_clause
        {
            #[allow(unused_variables)]
            fn read(
                reader: &mut ::robin_data_io::legacy_io::LegacyReader<'_>,
                ctx: &#ctx,
            ) -> ::robin_data_io::legacy_io::LegacyResult<Self> {
                #(#steps)*
                Ok(Self { #(#names),* })
            }
        }
    })
}

fn signature(attrs: &Attrs, span: proc_macro2::Span) -> syn::Result<TokenStream> {
    match (&attrs.fingerprint, &attrs.expected) {
        (None, None) => Ok(TokenStream::new()),
        (Some(fingerprint), Some(expected)) => {
            let name = attrs
                .fingerprint_name
                .clone()
                .unwrap_or_else(|| LitStr::new("fingerprint", span));
            Ok(quote! {
                reader.read_signature(#name, #fingerprint, #expected)?;
            })
        }
        _ => Err(Error::new(
            span,
            "`fingerprint = ..` and `expected = \"..\"` must be given together",
        )),
    }
}

fn field_read(attrs: &Attrs, ty: &Type, name: &LitStr, ctx: &Type) -> syn::Result<TokenStream> {
    if attrs.ctx.is_some() {
        return Err(Error::new(name.span(), "`ctx` is a struct attribute"));
    }
    if attrs.offset {
        return Ok(quote!(reader.offset()));
    }
    if let Some(value) = &attrs.value {
        return Ok(quote!(#value));
    }
    if let Some(cond) = &attrs.when {
        let read = base_read(attrs, generic_arg(ty, "Option")?, name, ctx);
        return Ok(quote!(if #cond { Some(#read) } else { None }));
    }
    let count = match (&attrs.count_u32, &attrs.count_u16, &attrs.len) {
        (None, None, None) => return Ok(base_read(attrs, ty, name, ctx)),
        (Some(limit), None, None) => Some((quote!(read_count_u32), limit)),
        (None, Some(limit), None) => Some((quote!(read_count_u16), limit)),
        (None, None, Some(_)) => None,
        _ => return Err(Error::new(name.span(), "use one list count attribute")),
    };
    let element = generic_arg(ty, "Vec")?;
    let (count_name, list_name) = if attrs.items {
        (
            LitStr::new("count", name.span()),
            LitStr::new("items", name.span()),
        )
    } else {
        (
            LitStr::new(&format!("{}.count", name.value()), name.span()),
            name.clone(),
        )
    };
    let count_name = attrs.count_name.clone().unwrap_or(count_name);
    let count = match (count, &attrs.len) {
        (Some((method, limit)), _) => quote!(reader.#method(#count_name, #limit)?),
        (None, len) => quote!(#len),
    };
    let list = quote! {{
        let __count: usize = #count;
        reader.read_list(#list_name, __count, |reader, __item| {
            <#element as ::robin_data_io::legacy_io::LegacyRead<#ctx>>::read_field(
                reader, __item, ctx,
            )
        })?
    }};
    Ok(if attrs.items {
        quote!(reader.scope(#name, |reader| Ok(#list))?)
    } else {
        list
    })
}

fn base_read(attrs: &Attrs, ty: &Type, name: &LitStr, ctx: &Type) -> TokenStream {
    let trait_path = quote!(::robin_data_io::legacy_io::LegacyRead<#ctx>);
    if let Some(read) = &attrs.read {
        quote!((#read)?)
    } else if let Some(with) = &attrs.with {
        let args = &attrs.args;
        quote!(#with(reader, #name #(, #args)*)?)
    } else if attrs.bytes {
        quote!(reader.read_array(#name)?)
    } else if attrs.flatten {
        quote!(<#ty as #trait_path>::read(reader, ctx)?)
    } else if attrs.scoped {
        quote!(reader.scope(#name, |reader| <#ty as #trait_path>::read(reader, ctx))?)
    } else {
        quote!(<#ty as #trait_path>::read_field(reader, #name, ctx)?)
    }
}

/// The single generic argument of `Wrapper<T>` (`Option`/`Vec`).
fn generic_arg<'a>(ty: &'a Type, wrapper: &str) -> syn::Result<&'a Type> {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == wrapper
        && let PathArguments::AngleBracketed(arguments) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = arguments.args.first()
    {
        return Ok(inner);
    }
    Err(Error::new_spanned(ty, format!("expected `{wrapper}<T>`")))
}
