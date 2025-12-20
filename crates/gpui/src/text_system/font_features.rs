use std::sync::Arc;

/// The OpenType features that can be configured for a given font.
#[derive(Default, Clone, Eq, PartialEq, Hash)]
pub struct FontFeatures(pub Arc<Vec<(String, u32)>>);

impl FontFeatures {
	/// Disables `calt`.
	pub fn disable_ligatures() -> Self {
		Self(Arc::new(vec![("calt".into(), 0)]))
	}

	/// Get the tag name list of the font OpenType features
	/// only enabled or disabled features are returned
	pub fn tag_value_list(&self) -> &[(String, u32)] {
		self.0.as_slice()
	}

	/// Returns whether the `calt` feature is enabled.
	///
	/// Returns `None` if the feature is not present.
	pub fn is_calt_enabled(&self) -> Option<bool> {
		self.0.iter().find(|(feature, _)| feature == "calt").map(|(_, value)| *value == 1)
	}
}

impl std::fmt::Debug for FontFeatures {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mut debug = f.debug_struct("FontFeatures");
		for (tag, value) in self.tag_value_list() {
			debug.field(tag, value);
		}

		debug.finish()
	}
}
