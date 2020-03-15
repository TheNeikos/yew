// mod builder;
mod field;
mod generics;
mod wrapper;


use field::{PropField, PropAttr};
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens, format_ident};
use std::convert::TryInto;
use syn::parse::{Parse, ParseStream, Result};
use syn::{DeriveInput, Generics, Visibility, parse::ParseBuffer};
use wrapper::PropsWrapper;

pub struct DerivePropsInput {
    vis: Visibility,
    generics: Generics,
    props_name: Ident,
    model_name: Ident,
    prop_fields: Vec<PropField>,
}

impl Parse for DerivePropsInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let input: DeriveInput = input.parse()?;
        let named_fields = match input.data {
            syn::Data::Struct(data) => match data.fields {
                syn::Fields::Named(fields) => fields.named,
                _ => unimplemented!("only structs are supported"),
            },
            _ => unimplemented!("only structs are supported"),
        };

        let model_name: Ident = input.attrs.iter().find(|attr| {
            attr.path.is_ident("prop_for")
        }).map(|attr| {
            attr.parse_args_with(|parser: &ParseBuffer| {
                parser.parse()
            }).unwrap()
        }).expect("You need to have an attribute 'prop_for'");

        let mut prop_fields: Vec<PropField> = named_fields
            .into_iter()
            .map(|f| f.try_into())
            .collect::<Result<Vec<PropField>>>()?;

        // Alphabetize
        prop_fields.sort();

        Ok(Self {
            vis: input.vis,
            props_name: input.ident,
            generics: input.generics,
            model_name,
            prop_fields,
        })
    }
}

impl ToTokens for DerivePropsInput {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            generics,
            props_name,
            prop_fields,
            model_name,
            ..
        } = self;

        // The wrapper is a new struct which wraps required props in `Option`
        let wrapper_name = Ident::new(&format!("{}Wrapper", props_name), Span::call_site());
        let wrapper = PropsWrapper::new(&wrapper_name, &generics, &self.prop_fields);
        tokens.extend(wrapper.into_token_stream());

        // The builder will only build if all required props have been set
        // let builder_name = Ident::new(&format!("{}Builder", props_name), Span::call_site());
        // let builder_step = Ident::new(&format!("{}BuilderStep", props_name), Span::call_site());
        // let builder = PropsBuilder::new(&builder_name, &builder_step, &self, &wrapper_name);
        // let builder_generic_args = builder.first_step_generic_args();
        // tokens.extend(builder.into_token_stream());

        // The properties trait has a `builder` method which creates the props builder
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        let ty_macro = format_ident!("{}PropCreator", &model_name);

        let prop_field_gen = prop_fields.iter().map(|field| {
            let ident = &field.name;
            let match_ident = format_ident!("match_{}", ident);
            quote!(let #ident = #ty_macro! { @#match_ident $($fields)* };)
        });

        let prop_field_names = prop_fields.iter().map(|field| &field.name);

        let prop_field_macro = prop_fields.iter().map(|field| {
            let ident = &field.name;
            let match_ident = format_ident!("match_{}", ident);
            let field_value = match &field.attr {
                PropAttr::Required { .. } => { quote!( compile_error!("Field #ident is required") ) }
                PropAttr::PropOr(expr) | PropAttr::PropOrElse(expr) => quote!( #expr ),
                PropAttr::PropOrDefault => quote!(Default::default()),
            };

            quote!(
                (@#match_ident #ident : $expr:expr , $($rest:tt)*) => {{
                    $expr
                }};
                (@#match_ident $first:ident : $expr:expr , $($rest:tt)*) => {{
                    #ty_macro! { @#match_ident $($rest)* }
                }};
                (@#match_ident $first:ident : $expr:expr) => {{
                    #field_value
                }};
                (@#match_ident) => {{
                    #field_value
                }};
            )
        });

        let properties = quote! {
            macro_rules! #ty_macro {
                #(#prop_field_macro)*
                ($($fields:tt)*) => {{
                    
                    #(#prop_field_gen)*

                    #props_name#ty_generics {
                        #(#prop_field_names),*
                    }
                }};
            }

            impl#impl_generics ::yew::html::Properties for #props_name#ty_generics #where_clause {}
        };

        tokens.extend(properties);
    }
}
