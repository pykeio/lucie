use std::borrow::Cow;

use lucie_common::{
	color::{Hsla, black},
	geometry::Pixels
};
use lucie_style::TextStyleRefinement;
use parley::{FontFamily, FontFamilyName, FontFeature, GenericFamily, StyleProperty, setting::Tag};

#[derive(Debug, Clone, PartialEq)]
pub struct Brush {
	/// The color of the text/strikethrough/underline.
	pub color: Hsla,
	pub background_color: Option<Hsla>,
	/// Whether the underline should be wavy, like in a spellchecker.
	pub wavy: bool
}

impl Default for Brush {
	fn default() -> Self {
		Brush {
			color: black(),
			background_color: None,
			wavy: false
		}
	}
}

fn resolve_font_name<'a>(name: &'a str) -> FontFamilyName<'a> {
	match name {
		"serif" => FontFamilyName::Generic(GenericFamily::Serif),
		"sans-serif" => FontFamilyName::Generic(GenericFamily::SansSerif),
		"monospace" => FontFamilyName::Generic(GenericFamily::Monospace),
		"cursive" => FontFamilyName::Generic(GenericFamily::Cursive),
		"fantasy" => FontFamilyName::Generic(GenericFamily::Fantasy),
		"system-ui" => FontFamilyName::Generic(GenericFamily::SystemUi),
		"ui-serif" => FontFamilyName::Generic(GenericFamily::UiSerif),
		"ui-sans-serif" => FontFamilyName::Generic(GenericFamily::UiSansSerif),
		"ui-monospace" => FontFamilyName::Generic(GenericFamily::UiMonospace),
		"ui-rounded" => FontFamilyName::Generic(GenericFamily::UiRounded),
		"math" => FontFamilyName::Generic(GenericFamily::Math),
		"fangsong" => FontFamilyName::Generic(GenericFamily::Math),
		_ => FontFamilyName::Named(Cow::Borrowed(name))
	}
}

pub(crate) fn style_to_parley(style: &TextStyleRefinement, rem_size: Pixels, mut apply_style: impl FnMut(StyleProperty<'_, Brush>)) {
	let mut brush: Option<Brush> = None;
	let mut strikethrough_brush: Option<Brush> = None;
	let mut underline_brush: Option<Brush> = None;

	if let Some(color) = style.color {
		brush.get_or_insert_default().color = color;
	}
	if let Some(background_color) = style.background_color {
		brush.get_or_insert_default().background_color = Some(background_color);
	}

	if let Some(font_fallbacks) = style.font_fallbacks.as_ref() {
		let mut families: Vec<_> = font_fallbacks
			.fallback_list()
			.iter()
			.map(|x| FontFamilyName::Named(Cow::Borrowed(x.as_str())))
			.collect();
		if let Some(font_family) = style.font_family.as_ref() {
			families.insert(0, resolve_font_name(&font_family));
		}
		apply_style(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(families))));
	} else if let Some(font_family) = style.font_family.as_ref() {
		apply_style(StyleProperty::FontFamily(FontFamily::Single(resolve_font_name(&font_family))));
	}
	if let Some(features) = style.font_features.as_ref() {
		let features = features.tag_value_list();
		let mut parley_features = Vec::new();
		for (tag, value) in features {
			if let Some(tag) = Tag::parse(tag) {
				parley_features.push(FontFeature::new(tag, *value as _));
			}
		}
		if !parley_features.is_empty() {
			apply_style(StyleProperty::FontFeatures(parley::FontFeatures::List(Cow::Owned(parley_features))));
		}
	}
	if let Some(font_weight) = style.font_weight.as_ref() {
		apply_style(StyleProperty::FontWeight(parley::FontWeight::new(font_weight.0)));
	}
	if let Some(font_style) = style.font_style {
		apply_style(StyleProperty::FontStyle(match font_style {
			lucie_style::FontStyle::Normal => parley::FontStyle::Normal,
			lucie_style::FontStyle::Italic => parley::FontStyle::Italic,
			lucie_style::FontStyle::Oblique => parley::FontStyle::Oblique(None)
		}));
	}

	if let Some(font_size) = style.font_size {
		apply_style(StyleProperty::FontSize(font_size.to_pixels(rem_size).0));
	}
	if let Some(line_height) = style.line_height {
		apply_style(StyleProperty::LineHeight(match line_height {
			lucie_common::geometry::DefiniteLength::Absolute(x) => parley::LineHeight::Absolute(x.to_pixels(rem_size).0),
			lucie_common::geometry::DefiniteLength::Fraction(x) => parley::LineHeight::FontSizeRelative(x)
		}));
	}
	if let Some(white_space) = style.white_space {
		apply_style(StyleProperty::TextWrapMode(match white_space {
			lucie_style::WhiteSpace::Normal => parley::TextWrapMode::Wrap,
			lucie_style::WhiteSpace::Nowrap => parley::TextWrapMode::NoWrap
		}));
	}

	if let Some(underline_style) = style.underline.as_ref() {
		underline_brush = Some(Brush {
			color: underline_style.color.unwrap_or_else(|| black()),
			wavy: underline_style.wavy,
			..Default::default()
		});
		apply_style(StyleProperty::UnderlineSize(Some(underline_style.thickness.0)));
		apply_style(StyleProperty::Underline(true));
	}
	if let Some(strikethrough_style) = style.strikethrough.as_ref() {
		strikethrough_brush = Some(Brush {
			color: strikethrough_style.color.unwrap_or_else(|| black()),
			..Default::default()
		});
		apply_style(StyleProperty::StrikethroughSize(Some(strikethrough_style.thickness.0)));
		apply_style(StyleProperty::Strikethrough(true));
	}

	if let Some(brush) = brush {
		apply_style(StyleProperty::Brush(brush));
	}
	if let Some(brush) = underline_brush {
		apply_style(StyleProperty::UnderlineBrush(Some(brush)));
	}
	if let Some(brush) = strikethrough_brush {
		apply_style(StyleProperty::StrikethroughBrush(Some(brush)));
	}
}
