use jq_rs::JqProgram;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, ItemFn, Path, Type};
use syn::{Ident, LitStr, Token};

// Goal: parse a macro invocation like
// json_enum(BaseItem, "base_items.json", ".[].name", {tags: Vec<String> = ".[].tags", item_class: String = ".[].item_class"})

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
    pub name: Ident,
    pub filename: String,
    pub names_expr: JqProgram,
    pub getters: Vec<Getter>,
}

impl JsonEnumInput {
    fn get_names(&mut self, data: String) -> anyhow::Result<Vec<Ident>> {
        let names: Vec<String> = serde_json::from_str(&*self.names_expr.run(&data).unwrap())?;
        let idents = names
            .into_iter()
            .map(|name| Ident::new(&*name, Span::call_site()))
            .collect::<Vec<Ident>>();
        Ok(idents)
    }

    fn get_paths(&mut self, data: String) -> anyhow::Result<Vec<Path>> {
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
}

impl Parse for JsonEnumInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let filename = input.parse::<LitStr>()?.value();
        input.parse::<Token![,]>()?;
        let names_expr = jq_rs::compile(&*input.parse::<LitStr>()?.value()).unwrap();
        input.parse::<Token![,]>()?;

        let content;
        let _brace = syn::braced!(content in input);
        let getters = content
            .parse_terminated::<Getter, Token![,]>(Getter::parse)?
            .into_iter()
            .collect();

        Ok(Self {
            name,
            filename,
            names_expr,
            getters,
        })
    }
}

#[proc_macro]
pub fn json_enum(input: TokenStream) -> TokenStream {
    let mut enum_spec: JsonEnumInput = parse_macro_input!(input as JsonEnumInput);

    let data = std::fs::read_to_string(&enum_spec.filename).unwrap();
    let variants = enum_spec.get_names(data.clone()).unwrap();
    let paths = enum_spec.get_paths(data.clone()).unwrap();
    // panic!("paths {:#?}", paths);

    let name = enum_spec.name;
    let getters = enum_spec
        .getters
        .iter_mut()
        .map(|getter| getter.make_getter_func(&paths, &data))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    // we also generate a test function for each attribute method that calls it on every variant
    // this is needed to ensure that all the embedded static data is deserializable
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
    quote::quote!(
        // hack because proc_macro::tracked* has no apparent path to stability
        // (see rust-lang/rust #73921)
        const _: &'static str = include_str!(#file_ref);

        // the generated enum
        pub enum #name {
            #(#variants),*
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
