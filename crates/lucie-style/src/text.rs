use std::{
	fmt::{self, Display},
	hash::{Hash, Hasher},
	sync::Arc
};

use derive_more::{Add, FromStr, Sub};
use lucie_common::{SharedString, color::Hsla};

use crate::{StrikethroughStyle, UnderlineStyle};

/// The degree of blackness or stroke thickness of a font. This value ranges from 100.0 to 900.0,
/// with 400.0 as normal.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Add, Sub, FromStr)]
pub struct FontWeight(pub f32);

impl Display for FontWeight {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl From<f32> for FontWeight {
	fn from(weight: f32) -> Self {
		FontWeight(weight)
	}
}

impl Default for FontWeight {
	#[inline]
	fn default() -> FontWeight {
		FontWeight::NORMAL
	}
}

impl Hash for FontWeight {
	fn hash<H: Hasher>(&self, state: &mut H) {
		state.write_u32(u32::from_be_bytes(self.0.to_be_bytes()));
	}
}

impl Eq for FontWeight {}

impl FontWeight {
	/// Thin weight (100), the thinnest value.
	pub const THIN: FontWeight = FontWeight(100.0);
	/// Extra light weight (200).
	pub const EXTRA_LIGHT: FontWeight = FontWeight(200.0);
	/// Light weight (300).
	pub const LIGHT: FontWeight = FontWeight(300.0);
	/// Normal (400).
	pub const NORMAL: FontWeight = FontWeight(400.0);
	/// Medium weight (500, higher than normal).
	pub const MEDIUM: FontWeight = FontWeight(500.0);
	/// Semibold weight (600).
	pub const SEMIBOLD: FontWeight = FontWeight(600.0);
	/// Bold weight (700).
	pub const BOLD: FontWeight = FontWeight(700.0);
	/// Extra-bold weight (800).
	pub const EXTRA_BOLD: FontWeight = FontWeight(800.0);
	/// Black weight (900), the thickest value.
	pub const BLACK: FontWeight = FontWeight(900.0);

	/// All of the font weights, in order from thinnest to thickest.
	pub const ALL: [FontWeight; 9] = [
		Self::THIN,
		Self::EXTRA_LIGHT,
		Self::LIGHT,
		Self::NORMAL,
		Self::MEDIUM,
		Self::SEMIBOLD,
		Self::BOLD,
		Self::EXTRA_BOLD,
		Self::BLACK
	];
}

/// Allows italic or oblique faces to be selected.
#[derive(Clone, Copy, Eq, PartialEq, Debug, Hash, Default)]
pub enum FontStyle {
	/// A face that is neither italic not obliqued.
	#[default]
	Normal,
	/// A form that is generally cursive in nature.
	Italic,
	/// A typically-sloped version of the regular face.
	Oblique
}

impl fmt::Display for FontStyle {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		fmt::Debug::fmt(self, f)
	}
}

/// A styled run of text, for use in [`crate::TextLayout`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextRun {
	/// A number of utf8 bytes
	pub len: usize,
	/// The font to use for this run.
	pub font: Font,
	/// The color
	pub color: Hsla,
	/// The background color (if any)
	pub background_color: Option<Hsla>,
	/// The underline style (if any)
	pub underline: Option<UnderlineStyle>,
	/// The strikethrough style (if any)
	pub strikethrough: Option<StrikethroughStyle>
}

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

/// The fallback fonts that can be configured for a given font.
/// Fallback fonts family names are stored here.
#[derive(Default, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FontFallbacks(pub Arc<Vec<String>>);

impl FontFallbacks {
	/// Get the fallback fonts family names
	pub fn fallback_list(&self) -> &[String] {
		self.0.as_slice()
	}

	/// Create a font fallback from a list of strings
	pub fn from_fonts(fonts: Vec<String>) -> Self {
		FontFallbacks(Arc::new(fonts))
	}
}

/// The configuration details for identifying a specific font.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Font {
	/// The font family name.
	///
	/// The special name "system-ui" is used to identify the system UI font, which varies based on platform.
	pub family: SharedString,

	/// The font features to use.
	pub features: FontFeatures,

	/// The fallbacks fonts to use.
	pub fallbacks: Option<FontFallbacks>,

	/// The font weight.
	pub weight: FontWeight,

	/// The font style.
	pub style: FontStyle
}

impl Default for Font {
	fn default() -> Self {
		font("system-ui")
	}
}

/// Get a [`Font`] for a given name.
pub fn font(family: impl Into<SharedString>) -> Font {
	Font {
		family: family.into(),
		features: FontFeatures::default(),
		weight: FontWeight::default(),
		style: FontStyle::default(),
		fallbacks: None
	}
}

impl Font {
	/// Set this Font to be bold
	pub fn bold(mut self) -> Self {
		self.weight = FontWeight::BOLD;
		self
	}

	/// Set this Font to be italic
	pub fn italic(mut self) -> Self {
		self.style = FontStyle::Italic;
		self
	}
}
