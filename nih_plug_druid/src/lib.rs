//! [Druid](https://github.com/linebender/druid) editor support for NIH-plug.

#![allow(clippy::type_complexity)]

use crossbeam::atomic::AtomicCell;
use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{Editor, GuiContext};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use druid;

mod editor;

pub fn create_druid_editor<T, F, G>(
    druid_state: Arc<DruidState>,
    make_data: G,
    make_window: F,
) -> Option<Box<dyn Editor>>
where
    T: druid::Data + Send + 'static,
    F: Fn(Arc<dyn GuiContext>) -> druid::WindowDesc<T> + 'static + Send + Sync,
    G: Fn() -> T + 'static + Send + Sync,
{
    Some(Box::new(editor::DruidEditor {
        druid_state,
        make_data: Arc::new(make_data),
        make_window: Arc::new(make_window),

        #[cfg(target_os = "macos")]
        scaling_factor: AtomicCell::new(None),
        #[cfg(not(target_os = "macos"))]
        scaling_factor: AtomicCell::new(Some(1.0)),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DruidState {
    #[serde(with = "nih_plug::params::persist::serialize_atomic_cell")]
    size: AtomicCell<(u32, u32)>,
    #[serde(skip)]
    open: AtomicBool,
}

impl<'a> PersistentField<'a, DruidState> for Arc<DruidState> {
    fn set(&self, new_value: DruidState) {
        self.size.store(new_value.size.load());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&DruidState) -> R,
    {
        f(self)
    }
}

impl DruidState {
    pub fn from_size(width: u32, height: u32) -> Arc<DruidState> {
        Arc::new(DruidState {
            size: AtomicCell::new((width, height)),
            open: AtomicBool::new(false),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}
