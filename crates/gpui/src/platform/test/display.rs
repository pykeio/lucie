use anyhow::{Ok, Result};

use crate::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, px};

#[derive(Debug)]
pub(crate) struct TestDisplay {
	id: DisplayId,
	uuid: [u8; 16],
	bounds: Bounds<Pixels>
}

impl TestDisplay {
	pub fn new() -> Self {
		TestDisplay {
			id: DisplayId(1),
			uuid: rand::random(),
			bounds: Bounds::from_corners(Point::default(), Point::new(px(1920.), px(1080.)))
		}
	}
}

impl PlatformDisplay for TestDisplay {
	fn id(&self) -> crate::DisplayId {
		self.id
	}

	fn uuid(&self) -> Result<[u8; 16]> {
		Ok(self.uuid)
	}

	fn bounds(&self) -> crate::Bounds<crate::Pixels> {
		self.bounds
	}
}
