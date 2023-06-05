use inwelling::*;

use proc_macro2::Span;
use std::collections::HashSet;
use std::{env, fs, path::PathBuf};
use syn::visit::Visit;
use syn::{ItemEnum, ItemStruct};

fn main() {
    let mut optics_set = OpticsSet::new();
    let mut optics_collector = OpticsCollector(&mut optics_set);

    let mut manifests = vec![];

    for section in inwelling(Opts {
        watch_manifest: false,
        watch_rs_files: false,
        dump_rs_paths: true,
    })
    .sections
    {
        manifests.push(section.manifest);
        for rs_path in section.rs_paths.unwrap() {
            let contents = String::from_utf8(fs::read(rs_path).unwrap()).unwrap();
            if let Ok(syntax) = syn::parse_file(&contents) {
                optics_collector.visit_file(&syntax);
            }
        }
    }

    let mut optics_set: Vec<_> = optics_set
        .into_iter()
        .filter(|o| {
            !matches!(
                o.as_str(),
                "Some"
                    | "None"
                    | "Ok"
                    | "Err"
                    | "_0"
                    | "_1"
                    | "_2"
                    | "_3"
                    | "_4"
                    | "_5"
                    | "_6"
                    | "_7"
                    | "_8"
                    | "_9"
                    | "_10"
                    | "_11"
                    | "_12"
                    | "_13"
                    | "_14"
                    | "_15"
                    | "_16"
            )
        })
        .collect();
    optics_set.sort();

    let mut output = String::new();
    for optic_name in optics_set {
        output += &format!(
            r"

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[allow(non_camel_case_types)]
pub struct {}<Optics>(pub Optics);

",
            optic_name
        );
    }

    let out_path =
        PathBuf::from(env::var("OUT_DIR").expect("$OUT_DIR should exist.")).join("optics.rs");

    // Check if the derived optics are the same between this run and the previous run.
    // If so, don't rerun the build script. If not, run as normal.
    if let Ok(file) = std::fs::read_to_string(&out_path) {
        if file == output {
            // Output is the same as existing file, so don't rebuild
            // unless a manifest has changed
            for manifest in manifests {
                println!("cargo:rerun-if-changed={}", manifest.display());
            }
        }
    }

    std::fs::write(out_path, output).expect("optics.rs should be generated.");
}

type OpticsSet = HashSet<String>;

struct OpticsCollector<'a>(&'a mut OpticsSet);

impl<'a> OpticsCollector<'a> {
    #[cfg(feature = "structx")]
    fn parse_structx(&mut self, input: proc_macro2::TokenStream) {
        let input_pat = wrap_struct_name("structx_", input);

        if let Ok(pat) = syn::parse2::<syn::Pat>(input_pat) {
            if let syn::Pat::Struct(pat_struct) = pat {
                self.add_structx_field_names(join_fields(pat_struct.fields.iter().map(|field| {
                    if let syn::Member::Named(ident) = &field.member {
                        ident.to_string()
                    } else {
                        panic!("structx!()'s fields should have names.");
                    }
                })));
            } else {
                panic!("structx!()'s supported pattern matching is struct only.");
            }
        }
    }

    #[cfg(feature = "structx")]
    fn add_structx_field_names(&mut self, field_names: Vec<String>) {
        for field_name in field_names {
            self.0.insert(field_name);
        }
    }
}

impl<'a> Visit<'_> for OpticsCollector<'a> {
    fn visit_item_enum(&mut self, item_enum: &ItemEnum) {
        for variant in &item_enum.variants {
            if variant_with_optic_attr(variant) {
                self.0.insert(format!("{}", variant.ident));
            }
        }
    }

    fn visit_item_struct(&mut self, item_struct: &ItemStruct) {
        if let syn::Fields::Named(fields_named) = &item_struct.fields {
            for field in &fields_named.named {
                if field_with_optic_attr(field) {
                    self.0.insert(format!("{}", field.ident.clone().unwrap()));
                }
            }
        }
    }

    #[cfg(feature = "structx")]
    fn visit_macro(&mut self, mac: &syn::Macro) {
        syn::visit::visit_macro(self, mac);

        if mac.path.leading_colon.is_none() && mac.path.segments.len() == 1 {
            let seg = mac.path.segments.first().unwrap();
            if seg.arguments == syn::PathArguments::None
                && (seg.ident == "structx" || seg.ident == "Structx")
            {
                self.parse_structx(mac.tokens.clone().into());
            }
        }
    }

    #[cfg(feature = "structx")]
    fn visit_item_fn(&mut self, item_fn: &syn::ItemFn) {
        syn::visit::visit_item_fn(self, item_fn);

        for attr in &item_fn.attrs {
            if attr.path.leading_colon.is_none() && attr.path.segments.len() == 1 {
                if attr.path.segments.first().unwrap().ident == "named_args" {
                    let fn_args = item_fn.sig.inputs.iter();
                    let mut field_names = Vec::with_capacity(fn_args.len());
                    for fn_arg in fn_args {
                        match fn_arg {
                            syn::FnArg::Receiver(_) => (),
                            syn::FnArg::Typed(pat_type) => {
                                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                                    field_names.push(pat_ident.ident.to_string());
                                } else {
                                    panic!("#[named_args] function's arguments should be either receiver or `id: Type`.");
                                }
                            }
                        }
                    }
                    self.add_structx_field_names(field_names);
                    return;
                }
            }
        }
    }
}

fn variant_with_optic_attr(var: &syn::Variant) -> bool {
    var.attrs.iter().any(|attr| {
        attr.path
            .is_ident(&syn::Ident::new("optic", Span::call_site()))
    })
}

fn field_with_optic_attr(field: &syn::Field) -> bool {
    field.attrs.iter().any(|attr| {
        attr.path
            .is_ident(&syn::Ident::new("optic", Span::call_site()))
    })
}

#[cfg(feature = "structx")]
fn join_fields(fields: impl Iterator<Item = String>) -> Vec<String> {
    fields.into_iter().collect()
}

#[cfg(feature = "structx")]
fn wrap_struct_name(
    struct_name: &str,
    input: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    use quote::ToTokens;
    let mut ts = proc_macro2::TokenStream::from(
        syn::Ident::new(struct_name, Span::call_site()).into_token_stream(),
    );
    ts.extend(Some(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Brace, input),
    )));
    ts
}
