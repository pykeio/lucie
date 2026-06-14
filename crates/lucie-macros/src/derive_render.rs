use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

pub fn derive_render(input: TokenStream) -> TokenStream {
	let ast = parse_macro_input!(input as DeriveInput);
	let type_name = &ast.ident;
	let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();

	let r#gen = quote! {
		impl #impl_generics lucie::Render for #type_name #type_generics
		#where_clause
		{
			fn render(&mut self, _window: &mut lucie::Window, _cx: &mut lucie::Context<Self>) -> impl lucie::Element {
				lucie::Empty
			}
		}
	};

	r#gen.into()
}
