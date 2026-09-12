//! Shared syntax inspection for source-boundary contracts.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// proc-macro spans are thread-local, so parsed syntax cannot live in a global
/// OnceLock. Keep immutable trees on their parsing thread instead.
pub(super) fn parsed(source: &'static str) -> Rc<syn::File> {
    thread_local! {
        static FILES: RefCell<HashMap<&'static str, Rc<syn::File>>> = RefCell::default();
    }
    FILES.with(|files| {
        files
            .borrow_mut()
            .entry(source)
            .or_insert_with(|| Rc::new(syn::parse_file(source).expect("parse production source")))
            .clone()
    })
}

pub(super) fn item_fn<'a>(file: &'a syn::File, name: &str) -> Option<&'a syn::ItemFn> {
    file.items.iter().find_map(|item| match item {
        syn::Item::Fn(item) if item.sig.ident == name => Some(item),
        _ => None,
    })
}

pub(super) fn item_struct<'a>(file: &'a syn::File, name: &str) -> Option<&'a syn::ItemStruct> {
    file.items.iter().find_map(|item| match item {
        syn::Item::Struct(item) if item.ident == name => Some(item),
        _ => None,
    })
}

#[test]
fn cached_syntax_keeps_identity_and_exact_item_kind() {
    let first = parsed("fn entry() {} struct Owner;");
    let second = parsed("fn entry() {} struct Owner;");
    assert!(Rc::ptr_eq(&first, &second));
    assert!(item_fn(&first, "entry").is_some());
    assert!(item_fn(&first, "Owner").is_none());
    assert!(item_struct(&first, "Owner").is_some());
    assert!(item_struct(&first, "entry").is_none());
}
