use anyhow::{Ok, Result};
use lucie_common::geometry::{Bounds, Pixels, Point, px};

use crate::{DisplayId, PlatformDisplay};

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
			uuid: {
				let mut u = [0; 16];
				fastrand::fill(&mut u);
				u
			},
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

	fn bounds(&self) -> Bounds<Pixels> {
		self.bounds
	}
}
