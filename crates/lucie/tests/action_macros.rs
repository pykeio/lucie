use lucie::{Action, actions};
use lucie_macros::register_action;

#[test]
fn test_action_macros() {
	actions!(
		test_only,
		[
			SomeAction,
			/// Documented action
			SomeActionWithDocs,
		]
	);

	#[derive(PartialEq, Clone, Action)]
	#[action(namespace = test_only)]
	struct AnotherAction;

	#[derive(PartialEq, Clone)]
	struct RegisterableAction {}

	register_action!(RegisterableAction);

	impl lucie::Action for RegisterableAction {
		fn boxed_clone(&self) -> Box<dyn lucie::Action> {
			unimplemented!()
		}

		fn partial_eq(&self, _action: &dyn lucie::Action) -> bool {
			unimplemented!()
		}

		fn name(&self) -> &'static str {
			unimplemented!()
		}

		fn name_for_type() -> &'static str
		where
			Self: Sized
		{
			unimplemented!()
		}

		fn build(_data: Option<&mut [u8]>) -> anyhow::Result<Box<dyn lucie::Action>>
		where
			Self: Sized
		{
			unimplemented!()
		}
	}
}
