use crate::{font::FontHandle, run::RunData};

const MAX_CACHED_HINT_INSTANCES: usize = 16;
const HINTING_OPTIONS: skrifa::outline::HintingOptions = skrifa::outline::HintingOptions {
	engine: skrifa::outline::Engine::AutoFallback,
	target: skrifa::outline::Target::Smooth {
		mode: skrifa::outline::SmoothMode::Lcd,
		symmetric_rendering: false,
		preserve_linear_metrics: true
	}
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HintKey<'a> {
	font: &'a FontHandle,
	size: skrifa::instance::Size,
	coords: &'a [skrifa::instance::NormalizedCoord]
}

impl<'a> HintKey<'a> {
	pub(crate) fn new(font: &'a FontHandle, run_data: &'a RunData<'a>) -> Self {
		Self {
			font,
			size: run_data.size(),
			coords: run_data.normalized_coords()
		}
	}

	fn make_instance(&self, outlines: &skrifa::OutlineGlyphCollection<'_>) -> Option<skrifa::outline::HintingInstance> {
		skrifa::outline::HintingInstance::new(outlines, self.size, self.coords, HINTING_OPTIONS).ok()
	}
}

pub struct HintCache {
	// Split caches for glyf/cff because the instance type can reuse internal memory when reconfigured for the same format.
	glyf_entries: Vec<HintEntry>,
	cff_entries: Vec<HintEntry>,
	serial: u64
}

impl HintCache {
	#[inline]
	pub fn new() -> Self {
		HintCache {
			glyf_entries: Vec::new(),
			cff_entries: Vec::new(),
			serial: 0
		}
	}

	pub(crate) fn get(&mut self, outlines: &skrifa::OutlineGlyphCollection<'_>, key: HintKey<'_>) -> Option<&skrifa::outline::HintingInstance> {
		let entries = match outlines.format()? {
			skrifa::outline::OutlineGlyphFormat::Glyf => &mut self.glyf_entries,
			_ => &mut self.cff_entries
		};
		let (entry_ix, is_current) = Self::find(entries, &key, outlines)?;
		let entry = entries.get_mut(entry_ix)?;
		self.serial += 1;
		entry.serial = self.serial;

		if !is_current {
			entry.font_id = key.font.id();
			entry.font_index = key.font.index();
			entry.instance.reconfigure(outlines, key.size, key.coords, HINTING_OPTIONS).ok()?;
		}

		Some(&entry.instance)
	}

	pub fn clear(&mut self) {
		self.glyf_entries.clear();
		self.cff_entries.clear();
		self.serial = 0;
	}

	fn find(entries: &mut Vec<HintEntry>, key: &HintKey<'_>, outlines: &skrifa::OutlineGlyphCollection<'_>) -> Option<(usize, bool)> {
		let mut found_serial = u64::MAX;
		let mut found_index = 0;
		for (ix, entry) in entries.iter().enumerate() {
			if entry.font_id == key.font.id()
				&& entry.font_index == key.font.index()
				&& entry.instance.size() == key.size
				&& entry.instance.location().coords() == key.coords
			{
				return Some((ix, true));
			}

			if entry.serial < found_serial {
				found_serial = entry.serial;
				found_index = ix;
			}
		}

		if entries.len() < MAX_CACHED_HINT_INSTANCES {
			let instance = key.make_instance(outlines)?;
			let ix = entries.len();
			entries.push(HintEntry {
				font_id: key.font.id(),
				font_index: key.font.index(),
				instance,
				serial: 0
			});
			Some((ix, true))
		} else {
			Some((found_index, false))
		}
	}
}

struct HintEntry {
	font_id: u64,
	font_index: u32,
	instance: skrifa::outline::HintingInstance,
	serial: u64
}
