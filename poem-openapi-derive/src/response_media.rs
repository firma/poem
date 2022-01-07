use std::str::FromStr;

use darling::{
    ast::{Data, Fields},
    util::{Ignored, SpannedValue},
    FromDeriveInput, FromVariant,
};
use mime::Mime;
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{Attribute, DeriveInput, Error, Generics, Type};

use crate::{
    error::GeneratorResult,
    utils::{get_crate_name, get_description, optional_literal},
};

#[derive(FromVariant)]
#[darling(attributes(oai), forward_attrs(doc))]
struct MediaItem {
    ident: Ident,
    fields: Fields<Type>,

    #[darling(default)]
    content_type: Option<SpannedValue<String>>,
}

#[derive(FromDeriveInput)]
#[darling(attributes(oai), forward_attrs(doc))]
struct ResponseMediaArgs {
    ident: Ident,
    attrs: Vec<Attribute>,
    generics: Generics,
    data: Data<RequestItem, Ignored>,

    #[darling(default)]
    internal: bool,
}

pub(crate) fn generate(args: DeriveInput) -> GeneratorResult<TokenStream> {
    let args: ResponseMediaArgs = ResponseMediaArgs::from_derive_input(&args)?;
    let crate_name = get_crate_name(args.internal);
    let (impl_generics, ty_generics, where_clause) = args.generics.split_for_impl();
    let ident = &args.ident;
    let e = match &args.data {
        Data::Enum(e) => e,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "ResponseMediaArgs can only be applied to an enum.",
            )
            .into())
        }
    };
    let description = get_description(&args.attrs)?;
    let description = optional_literal(&description);

    let mut content_types = Vec::new();
    let mut into_responses = Vec::new();
    let mut content = Vec::new();
    let mut schemas = Vec::new();

    for (idx, variant) in e.iter().enumerate() {
        let item_ident = &variant.ident;

        match variant.fields.len() {
            1 => {
                // Item(payload)
                let payload_ty = &variant.fields.fields[0];
                let content_type = match &variant.content_type {
                    Some(content_type) => {
                        if !matches!(Mime::from_str(content_type), Ok(mime) if mime.params().count() == 0)
                        {
                            return Err(Error::new(content_type.span(), "Invalid mime type").into());
                        }
                        let content_type = &**content_type;
                        quote!(#content_type)
                    }
                    None => quote!(<#payload_ty as #crate_name::payload::Payload>::CONTENT_TYPE),
                };
                content_types.push(content_type.clone());
                into_responses.push(quote! {});
                content.push(quote! {
                    #crate_name::registry::MetaMediaType {
                        content_type: #content_type,
                        schema: <#payload_ty as #crate_name::payload::Payload>::schema_ref(),
                    }
                });
                schemas.push(payload_ty);
            }
            _ => {
                return Err(
                    Error::new_spanned(&variant.ident, "Incorrect request definition.").into(),
                )
            }
        }
    }

    let expanded = {
        quote! {
            #[#crate_name::__private::poem::async_trait]
            impl #impl_generics #crate_name::Payload for #ident #ty_generics #where_clause {
                const TYPE: #crate_name::ApiExtractorType = #crate_name::ApiExtractorType::RequestObject;

                type ParamType = ();
                type ParamRawType = ();

                fn register(registry: &mut #crate_name::registry::Registry) {
                    #(<#schemas as #crate_name::payload::Payload>::register(registry);)*
                }

                fn request_meta() -> ::std::option::Option<#crate_name::registry::MetaRequest> {
                    ::std::option::Option::Some(#crate_name::registry::MetaRequest {
                        description: #description,
                        content: ::std::vec![#(#content),*],
                        required: true,
                    })
                }

                async fn from_request(
                    request: &'__request #crate_name::__private::poem::Request,
                    body: &mut #crate_name::__private::poem::RequestBody,
                    _param_opts: #crate_name::ExtractParamOptions<Self::ParamType>,
                ) -> #crate_name::__private::poem::Result<Self> {
                    match request.content_type() {
                        ::std::option::Option::Some(content_type) => {
                            let table = #crate_name::__private::ContentTypeTable::new(&[#(#content_types),*]);
                            match table.matches(content_type) {
                                #(#from_requests)*
                                _ => {
                                    ::std::result::Result::Err(
                                        ::std::convert::Into::into(#crate_name::error::ContentTypeError::NotSupported {
                                            content_type: ::std::string::ToString::to_string(content_type),
                                    }))
                                }
                            }
                        }
                        ::std::option::Option::None => {
                            ::std::result::Result::Err(::std::convert::Into::into(#crate_name::error::ContentTypeError::ExpectContentType))
                        }
                    }
                }
            }
        }
    };

    Ok(expanded)
}