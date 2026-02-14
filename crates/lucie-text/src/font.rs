use std::{
	hash::{Hash, Hasher},
	mem,
	ops::Deref,
	ptr,
	sync::atomic::{AtomicBool, Ordering}
};

use either::Either;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use rapidhash::RapidHashMap;
use skrifa::MetadataProvider as _;

use crate::{
	SubpixelVariant,
	glyph::GlyphId,
	hinting::HintCache,
	rasterize::{RasterizedGlyph, rasterize_outline_glyph},
	run::RunData
};

#[derive(Debug, Clone, PartialEq)]
#[repr(transparent)]
pub struct FontHandle(parley::FontData);

impl FontHandle {
	#[inline]
	pub(crate) fn new(font: parley::FontData) -> Self {
		Self(font)
	}

	#[inline]
	pub(crate) fn data(&self) -> &[u8] {
		&self.0.data.data()
	}

	#[inline]
	pub(crate) fn id(&self) -> u64 {
		self.0.data.id()
	}

	#[inline]
	pub(crate) fn index(&self) -> u32 {
		self.0.index
	}
}

impl Eq for FontHandle {}

impl Hash for FontHandle {
	fn hash<H: Hasher>(&self, state: &mut H) {
		state.write_u64(self.0.data.id());
		state.write_u32(self.0.index);
	}
}

pub struct FontCache {
	fonts: RwLock<RapidHashMap<FontHandle, LoadedFont>>
}

impl FontCache {
	pub fn new() -> Self {
		Self {
			fonts: RwLock::new(RapidHashMap::default())
		}
	}

	pub fn get(&self, font: &FontHandle) -> impl Deref<Target = LoadedFont> {
		let fonts = self.fonts.read();
		match RwLockReadGuard::try_map(fonts, |x| x.get(font)) {
			Ok(x) => {
				x.used.store(true, Ordering::Release);
				Either::Left(x)
			}
			Err(guard) => {
				drop(guard);

				let mut fonts = self.fonts.write();
				let new_font = fonts.entry(font.clone()).insert_entry(LoadedFont::new(font.clone()).unwrap()).into_mut();
				new_font.used.store(true, Ordering::Release);
				// The reference stays valid even after we downgrade the lock because `downgrade` doesn't allow other writers in the
				// meantime.
				let ptr = ptr::from_ref(new_font);
				let fonts = RwLockWriteGuard::downgrade(fonts);
				Either::Right(RwLockReadGuard::map(fonts, |_| unsafe { &*ptr }))
			}
		}
	}

	pub fn retain(&self) {
		let mut fonts = self.fonts.write();
		fonts.retain(|_, font| !font.used.swap(false, Ordering::AcqRel));
	}
}

pub struct LoadedFont {
	pub(crate) used: AtomicBool,

	#[allow(unused)] // TODO: do we need to hold onto this?
	font: skrifa::FontRef<'static>,
	outline_glyphs: skrifa::OutlineGlyphCollection<'static>,
	color_glyphs: skrifa::color::ColorGlyphCollection<'static>,
	bitmap_glyphs: skrifa::bitmap::BitmapStrikes<'static>,

	// define last so dropped last
	handle: FontHandle
}

impl LoadedFont {
	pub(crate) fn new(handle: FontHandle) -> Result<Self, skrifa::raw::ReadError> {
		let font = skrifa::FontRef::from_index(handle.data(), handle.index())?;

		let outline_glyphs = font.outline_glyphs();
		let color_glyphs = font.color_glyphs();
		let bitmap_glyphs = font.bitmap_strikes();

		Ok(Self {
			used: AtomicBool::new(false),
			// SAFETY: self-referential type hack. these all depend on the lifetime of `font_data`; transmute to 'static to shut the compiler up
			// this is valid because we hold onto the data `Blob` in the `handle` field, so the underlying data is indeed valid for as long
			// as this struct (and its members) are held. defining `handle` last also means it is dropped *after* the rest of the structs.
			font: unsafe { mem::transmute(font) },
			outline_glyphs: unsafe { mem::transmute(outline_glyphs) },
			color_glyphs: unsafe { mem::transmute(color_glyphs) },
			bitmap_glyphs: unsafe { mem::transmute(bitmap_glyphs) },
			handle
		})
	}

	pub fn rasterize_glyph(
		&self,
		id: GlyphId,
		subpixel_variant: SubpixelVariant,
		run_data: &RunData<'_>,
		hint_cache: Option<&mut HintCache>
	) -> Option<RasterizedGlyph> {
		let id = id.0;

		if let Some(outline) = self.outline_glyphs.get(id) {
			return rasterize_outline_glyph(outline, subpixel_variant, run_data, &self.handle, &self.outline_glyphs, hint_cache);
		}

		if let Some(_color) = self.color_glyphs.get(id) {
			unimplemented!("COLRv0/COLRv1 glyph rasterization not implemented");
		}

		if let Some(_bitmap) = self.bitmap_glyphs.glyph_for_size(run_data.size(), id) {
			unimplemented!("Bitmap glyph rasterization not implemented");
		}

		None
	}
}
