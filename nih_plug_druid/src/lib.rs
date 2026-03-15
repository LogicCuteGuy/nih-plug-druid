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

fn default_druid_scale_factor() -> AtomicCell<f64> {
    AtomicCell::new(1.0)
}

pub fn wrap_with_scale<T, W>(
    druid_state: Arc<DruidState>,
    context: Arc<dyn GuiContext>,
    resize_config: ResizableScaleConfig,
    child: W,
) -> impl druid::Widget<T>
where
    T: druid::Data,
    W: druid::Widget<T> + 'static,
{
    editor::ResizableScale::new(druid_state, context, resize_config, child)
}

#[derive(Debug, Clone, Copy)]
pub struct ResizableScaleConfig {
    pub handle_size: f64,
    pub min_scale_factor: f64,
    pub max_scale_factor: f64,
}

impl Default for ResizableScaleConfig {
    fn default() -> Self {
        Self {
            handle_size: 18.0,
            min_scale_factor: 0.5,
            max_scale_factor: 4.0,
        }
    }
}

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
    #[serde(rename = "size", with = "nih_plug::params::persist::serialize_atomic_cell")]
    logical_size: AtomicCell<(u32, u32)>,
    #[serde(
        default = "default_druid_scale_factor",
        with = "nih_plug::params::persist::serialize_atomic_cell"
    )]
    scale_factor: AtomicCell<f64>,
    #[serde(skip)]
    open: AtomicBool,
}

impl<'a> PersistentField<'a, DruidState> for Arc<DruidState> {
    fn set(&self, new_value: DruidState) {
        self.logical_size.store(new_value.logical_size.load());
        self.scale_factor.store(new_value.scale_factor.load());
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
        Self::new_with_default_scale_factor(width, height, 1.0)
    }

    pub fn new_with_default_scale_factor(
        width: u32,
        height: u32,
        default_scale_factor: f64,
    ) -> Arc<DruidState> {
        Arc::new(DruidState {
            logical_size: AtomicCell::new((width, height)),
            scale_factor: AtomicCell::new(default_scale_factor.clamp(0.5, 4.0)),
            open: AtomicBool::new(false),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        let (width, height) = self.logical_size();
        let scale_factor = self.user_scale_factor();

        (
            (width as f64 * scale_factor).round() as u32,
            (height as f64 * scale_factor).round() as u32,
        )
    }

    pub fn logical_size(&self) -> (u32, u32) {
        self.logical_size.load()
    }

    pub fn user_scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    pub fn set_user_scale_factor(&self, scale_factor: f64) {
        self.scale_factor.store(scale_factor.clamp(0.5, 4.0));
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}
