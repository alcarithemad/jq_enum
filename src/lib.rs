use jq_rs::JqProgram;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, ToTokens};
use std::collections::HashMap;
use std::iter;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, parse_quote, Attribute, ItemFn, Path, Type, Variant};
use syn::{Ident, LitStr, Token};

struct OptionalModifier {
    pub name: Ident,
    _col: Token![:],
    pub jq_expr: JqProgram,
}

impl Parse for OptionalModifier {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            name: input.parse()?,
            _col: input.parse()?,
            jq_expr: jq_rs::compile(input.parse::<LitStr>()?.value().as_ref()).unwrap(),
        })
    }
}
/// Parses something like `foo: T = ".[].foo"`
struct Getter {
    pub name: Ident,
    _col: Token![:],
    pub ty: Type,
    _eq: Token![=],
    pub jq_expr: JqProgram,
}

impl Parse for Getter {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            name: input.parse()?,
            _col: input.parse()?,
            ty: input.parse()?,
            _eq: input.parse()?,
            jq_expr: jq_rs::compile(
                {
                    let mut prog = input.parse::<LitStr>()?.value();
                    prog.push_str("| [ .[] | tojson ]");
                    prog
                }
                .as_ref(),
            )
            .unwrap(),
        })
    }
}

impl Getter {
    /// make a `syn::ImplItemMethod` that takes a variant of the enum and
    /// returns the nth value of `self.jq_expr` corresponding to the nth variant  
    fn make_getter_func(
        &mut self,
        variants: &[Path],
        data: &str,
    ) -> anyhow::Result<syn::ImplItemMethod> {
        let results: Vec<String> = serde_json::from_str(&*self.jq_expr.run(&*data).unwrap())?;
        let match_arms = variants
            .iter()
            .zip(results)
            .map(|(p, data)| {
                let ty = &self.ty;
                let p_s = p.to_token_stream();
                let lit_data = LitStr::new(&data, Span::call_site());
                let error = LitStr::new(&format!("failed to parse {p_s}"), Span::call_site());
                syn::parse_quote!(#p => serde_json::from_str::<#ty>(#lit_data).expect(#error))
            })
            .collect::<Vec<syn::Arm>>();
        let name = &self.name;
        let ty = &self.ty;
        Ok(syn::parse_quote!(
            pub fn #name(&self) -> #ty {
                match self {
                    #(#match_arms),*
                }
            }
        ))
    }
}

struct JsonEnumInput {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub filename: String,
    pub names_expr: JqProgram,
    pub getters: Vec<Getter>,
    pub options: HashMap<String, JqProgram>,
}

impl JsonEnumInput {
    fn get_names(&mut self, data: &str) -> anyhow::Result<Vec<Ident>> {
        let names: Vec<String> = serde_json::from_str(&*self.names_expr.run(&data).unwrap())?;
        let idents = names
            .into_iter()
            .map(|name| Ident::new(&*name, Span::call_site()))
            .collect::<Vec<Ident>>();
        Ok(idents)
    }

    fn get_paths(&mut self, data: &str) -> anyhow::Result<Vec<Path>> {
        Ok(self
            .get_names(data)?
            .into_iter()
            .map(|id| {
                let name = self.name.clone();
                let p: Path = syn::parse_quote!(#name::#id);
                p
            })
            .collect())
    }
    fn expanded_variants(&mut self, data: &str) -> anyhow::Result<Vec<Variant>> {
        let names = self.get_names(data)?;
        let renames: Vec<String> = match self.options.get_mut("serde_rename_variants") {
            Some(jq_expr) => serde_json::from_str(&jq_expr.run(data).unwrap())?,
            None => vec![],
        };
        let strum_strs: Vec<String> = match self.options.get_mut("strum_enum_string") {
            Some(jq_expr) => serde_json::from_str(&jq_expr.run(data).unwrap())?,
            None => vec![],
        };
        let x: Vec<_> = names
            .iter()
            .zip(renames.iter().map(Some).chain(iter::repeat(None)))
            .map(|(name, rename)| match rename {
                Some(_) => {
                    parse_quote! {
                        #[serde(rename = #rename)]
                        #name
                    }
                }
                None => {
                    parse_quote! {
                        #name
                    }
                }
            })
            .zip(strum_strs.iter().map(Some).chain(iter::repeat(None)))
            .map(|(mut name, st): (Variant, _)| match st {
                Some(s) => {
                    name.attrs.push(parse_quote! {#[strum(serialize = #s)]});
                    name
                }
                None => name,
            })
            .collect();
        Ok(x)
    }
}

impl Parse for JsonEnumInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let filename = input.parse::<LitStr>()?.value();
        input.parse::<Token![,]>()?;
        let names_expr = jq_rs::compile(&*input.parse::<LitStr>()?.value()).unwrap();
        let getters: Option<Vec<Getter>> = if input.parse::<Token![,]>().is_ok() {
            let content;
            let _brace = syn::braced!(content in input);
            Some(
                content
                    .parse_terminated::<_, Token![,]>(Getter::parse)?
                    .into_iter()
                    .collect(),
            )
        } else {
            None
        };

        let options: Option<HashMap<_, _>> =
            if getters.is_some() && input.parse::<Token![,]>().is_ok() {
                let content2;
                let _brace2 = syn::braced!(content2 in input);
                Some(
                    content2
                        .parse_terminated::<_, Token![,]>(OptionalModifier::parse)?
                        .into_iter()
                        .map(
                            |OptionalModifier {
                                 name,
                                 _col,
                                 jq_expr,
                             }| (name.to_string(), jq_expr),
                        )
                        .collect(),
                )
            } else {
                None
            };

        Ok(Self {
            attrs,
            name,
            filename,
            names_expr,
            getters: getters.unwrap_or_default(),
            options: options.unwrap_or_default(),
        })
    }
}

#[proc_macro]
pub fn json_enum(input: TokenStream) -> TokenStream {
    let mut enum_spec: JsonEnumInput = parse_macro_input!(input as JsonEnumInput);

    let data = std::fs::read_to_string(&enum_spec.filename).unwrap();
    let variants = enum_spec.get_names(&data).unwrap();
    let paths = enum_spec.get_paths(&data).unwrap();
    let expanded_variants = enum_spec.expanded_variants(&data).unwrap();

    let name = enum_spec.name;
    let getters = enum_spec
        .getters
        .iter_mut()
        .map(|getter| getter.make_getter_func(&paths, &data))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    // for each attribute method, we also generate a test function that calls it on every variant
    // this ensures that all the embedded static data is actually deserializable,
    // so schema violations are discovered at test time
    let test_name = format_ident!("test_{}", name);
    let tests = getters
        .iter()
        .map(|g| -> ItemFn {
            let name = &g.sig.ident;
            let test_name = format_ident!("test_{}", name);
            syn::parse_quote!(
                #[test]
                fn #test_name() {
                    #(#paths.#name();)*
                }
            )
        })
        .collect::<Vec<_>>();
    let file_path = std::path::PathBuf::from(enum_spec.filename)
        .canonicalize()
        .unwrap();
    let file_ref = file_path.to_str().unwrap();
    let attrs = &enum_spec.attrs;
    quote::quote!(
        // hack because proc_macro::tracked* has no apparent path to stability
        // (see rust-lang/rust #73921)
        const _: &'static str = include_str!(#file_ref);

        // the generated enum
        #(#attrs)*
        pub enum #name {
            #(#expanded_variants),*
        }

        // the generated attribute methods
        impl #name {
            #(#getters)*
        }

        // tests for the generated attribute methods
        // because the module is named for the type, which probably obeys type naming conventions
        // rather than module naming conventions
        #[allow(non_snake_case)]
        #[cfg(test)]
        mod #test_name {
            use super::#name;
            #(#tests)*
        }
    )
    .into()
}
