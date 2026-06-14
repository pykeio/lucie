use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

use super::get_simple_attribute_field;

pub fn derive_visual_context(input: TokenStream) -> TokenStream {
	let ast = parse_macro_input!(input as DeriveInput);

	let Some(window_variable) = get_simple_attribute_field(&ast, "window") else {
		return quote! {
			compile_error!("Derive must have a #[window] attribute to detect the &mut Window field");
		}
		.into();
	};

	let Some(app_variable) = get_simple_attribute_field(&ast, "app") else {
		return quote! {
			compile_error!("Derive must have a #[app] attribute to detect the &mut App field");
		}
		.into();
	};

	let type_name = &ast.ident;
	let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();

	let r#gen = quote! {
		impl #impl_generics lucie::VisualContext for #type_name #type_generics
		#where_clause
		{
			type Result<T> = T;

			fn window_handle(&self) -> lucie::AnyWindowHandle {
				self.#window_variable.window_handle()
			}

			fn update_window_entity<T: 'static, R>(
				&mut self,
				entity: &lucie::Entity<T>,
				update: impl FnOnce(&mut T, &mut lucie::Window, &mut lucie::Context<T>) -> R,
			) -> Self::Result<R> {
				lucie::AppContext::update_entity(self.#app_variable, entity, |entity, cx| update(entity, self.#window_variable, cx))
			}

			fn new_window_entity<T: 'static>(
				&mut self,
				build_entity: impl FnOnce(&mut lucie::Window, &mut lucie::Context<'_, T>) -> T,
			) -> Self::Result<lucie::Entity<T>> {
				lucie::AppContext::new(self.#app_variable, |cx| build_entity(self.#window_variable, cx))
			}

			fn replace_root_view<V>(
				&mut self,
				build_view: impl FnOnce(&mut lucie::Window, &mut lucie::Context<V>) -> V,
			) -> Self::Result<lucie::Entity<V>>
			where
				V: 'static + lucie::Render,
			{
				self.#window_variable.replace_root(self.#app_variable, build_view)
			}

			fn focus<V>(&mut self, entity: &lucie::Entity<V>) -> Self::Result<()>
			where
				V: lucie::Focusable,
			{
				let focus_handle = lucie::Focusable::focus_handle(entity, self.#app_variable);
				self.#window_variable.focus(&focus_handle, self.#app_variable)
			}
		}
	};

	r#gen.into()
}
