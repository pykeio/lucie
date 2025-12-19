use std::{
    fmt::Debug,
    hash::{Hash, Hasher},
};

use anyhow::Context as _;
use wayland_backend::client::ObjectId;

use crate::{Bounds, DisplayId, Pixels, PlatformDisplay};

#[derive(Debug, Clone)]
pub(crate) struct WaylandDisplay {
    /// The ID of the wl_output object
    pub id: ObjectId,
    pub name: Option<String>,
    pub bounds: Bounds<Pixels>,
}

impl Hash for WaylandDisplay {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PlatformDisplay for WaylandDisplay {
    fn id(&self) -> DisplayId {
        DisplayId(self.id.protocol_id())
    }

    fn uuid(&self) -> anyhow::Result<[u8; 16]> {
        let mut hasher = rapidhash::quality::RapidHasher::default();
        self.id.hash(&mut hasher);
        let id = hasher.finish().to_le_bytes();

        Ok([
            id[0], id[1], id[2], id[3], id[4], id[5], id[6], id[7], id[0], id[1], id[2], id[3],
            id[4], id[5], id[6], id[7],
        ])
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
}
