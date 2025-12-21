#[test]
fn test_derive_render() {
	use lucie_macros::Render;

	#[derive(Render)]
	struct _Element;
}
