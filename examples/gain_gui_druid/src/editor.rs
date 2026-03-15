use atomic_float::AtomicF32;
use druid::widget::Controller;
use druid::widget::prelude::*;
use druid::widget::{Button, Flex, Label, Slider};
use druid::{Color, Cursor, Data, Event, EventCtx, Lens, Point, TimerToken, Widget, WidgetExt, WindowDesc};
use nih_plug::prelude::{util, Editor, Param, ParamSetter};
use nih_plug_druid::{
    create_druid_editor, wrap_with_scale, DruidState, ResizableScaleConfig,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::GainParams;

const POLL_INTERVAL: Duration = Duration::from_millis(33);
const RESIZE_HANDLE_SIZE: f64 = 18.0;
const MIN_USER_SCALE_FACTOR: f64 = 0.9;
const MAX_USER_SCALE_FACTOR: f64 = 3.5;

#[derive(Clone, Data, Lens)]
struct UiData {
    gain_db: f64,
    peak_meter_db: f64,
}

struct PollController {
    params: Arc<GainParams>,
    peak_meter: Arc<AtomicF32>,
    context: Arc<dyn nih_plug::prelude::GuiContext>,
    timer_id: TimerToken,
}

impl PollController {
    fn sync_data(&self, data: &mut UiData) {
        data.gain_db = util::gain_to_db(self.params.gain.unmodulated_plain_value()) as f64;
        data.peak_meter_db = util::gain_to_db(self.peak_meter.load(Ordering::Relaxed)) as f64;
    }

    fn set_gain_parameter(&self, gain_db: f64) {
        let clamped_db = gain_db.clamp(-30.0, 30.0) as f32;
        let setter = ParamSetter::new(self.context.as_ref());
        setter.begin_set_parameter(&self.params.gain);
        setter.set_parameter(&self.params.gain, util::db_to_gain(clamped_db));
        setter.end_set_parameter(&self.params.gain);
    }
}

impl<W: Widget<UiData>> Controller<UiData, W> for PollController {
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut UiData,
        env: &druid::Env,
    ) {
        match event {
            Event::WindowConnected => {
                self.sync_data(data);
                self.timer_id = ctx.request_timer(POLL_INTERVAL);
            }
            Event::Timer(token) if *token == self.timer_id => {
                self.sync_data(data);
                ctx.request_paint();
                self.timer_id = ctx.request_timer(POLL_INTERVAL);
            }
            _ => {}
        }

        child.event(ctx, event, data, env);
    }

    fn update(
        &mut self,
        child: &mut W,
        ctx: &mut UpdateCtx,
        old_data: &UiData,
        data: &UiData,
        env: &druid::Env,
    ) {
        if (old_data.gain_db - data.gain_db).abs() > f64::EPSILON {
            self.set_gain_parameter(data.gain_db);
        }

        child.update(ctx, old_data, data, env);
    }
}

struct HoverPointerController;

impl<T: Data, W: Widget<T>> Controller<T, W> for HoverPointerController {
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut T,
        env: &druid::Env,
    ) {
        if let Event::MouseMove(_) = event {
            if ctx.is_hot() {
                ctx.set_cursor(&Cursor::Pointer);
            }
        }

        child.event(ctx, event, data, env);
    }
}

struct KnobWidget {
    drag_anchor_y: f64,
    drag_anchor_value: f64,
    dragging: bool,
}

impl KnobWidget {
    fn new() -> Self {
        Self {
            drag_anchor_y: 0.0,
            drag_anchor_value: 0.0,
            dragging: false,
        }
    }

    fn value_to_angle(value: f64) -> f64 {
        let normalized = ((value + 30.0) / 60.0).clamp(0.0, 1.0);
        let start = (-225.0_f64).to_radians();
        let end = 45.0_f64.to_radians();
        start + normalized * (end - start)
    }
}

impl Widget<f64> for KnobWidget {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut f64, _env: &druid::Env) {
        match event {
            Event::MouseDown(mouse) => {
                self.dragging = true;
                self.drag_anchor_y = mouse.pos.y;
                self.drag_anchor_value = *data;
                ctx.set_active(true);
                ctx.request_paint();
            }
            Event::MouseMove(mouse) if self.dragging => {
                let delta = (self.drag_anchor_y - mouse.pos.y) * 0.15;
                *data = (self.drag_anchor_value + delta).clamp(-30.0, 30.0);
                ctx.request_paint();
            }
            Event::MouseUp(_) => {
                self.dragging = false;
                ctx.set_active(false);
                ctx.request_paint();
            }
            Event::Wheel(mouse) => {
                *data = (*data + mouse.wheel_delta.y * 0.01).clamp(-30.0, 30.0);
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn lifecycle(
        &mut self,
        _ctx: &mut LifeCycleCtx,
        _event: &LifeCycle,
        _data: &f64,
        _env: &druid::Env,
    ) {
    }

    fn update(&mut self, ctx: &mut UpdateCtx, old_data: &f64, data: &f64, _env: &druid::Env) {
        if (old_data - data).abs() > f64::EPSILON {
            ctx.request_paint();
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx,
        bc: &BoxConstraints,
        _data: &f64,
        _env: &druid::Env,
    ) -> Size {
        bc.constrain((84.0, 84.0))
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &f64, _env: &druid::Env) {
        let rect = ctx.size().to_rect().inset(-1.0);
        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.5 - 8.0;

        let body = druid::kurbo::Circle::new(center, radius);
        ctx.fill(body, &Color::grey8(35));
        ctx.stroke(body, &Color::grey8(90), 2.0);

        let angle = Self::value_to_angle(*data);
        let indicator_start = Point::new(
            center.x + angle.cos() * radius * 0.25,
            center.y + angle.sin() * radius * 0.25,
        );
        let indicator_end = Point::new(
            center.x + angle.cos() * radius * 0.8,
            center.y + angle.sin() * radius * 0.8,
        );
        ctx.stroke(druid::kurbo::Line::new(indicator_start, indicator_end), &Color::rgb8(102, 217, 239), 3.0);

        let marker = druid::kurbo::Circle::new(indicator_end, 4.0);
        ctx.fill(marker, &Color::rgb8(230, 180, 70));
    }
}

pub(crate) fn default_state() -> Arc<DruidState> {
    DruidState::from_size(360, 260)
}

pub(crate) fn create(
    params: Arc<GainParams>,
    peak_meter: Arc<AtomicF32>,
    editor_state: Arc<DruidState>,
) -> Option<Box<dyn Editor>> {
    create_druid_editor(
        editor_state.clone(),
        {
            let params = params.clone();
            let peak_meter = peak_meter.clone();
            move || UiData {
                gain_db: util::gain_to_db(params.gain.unmodulated_plain_value()) as f64,
                peak_meter_db: util::gain_to_db(peak_meter.load(Ordering::Relaxed)) as f64,
            }
        },
        move |context| {
            let slider = Slider::new()
                .with_range(-30.0, 30.0)
                .with_step(0.1)
                .expand_width()
                .controller(HoverPointerController)
                .lens(UiData::gain_db);

            let knob = KnobWidget::new()
                .controller(HoverPointerController)
                .lens(UiData::gain_db);

            let minus_params = params.clone();
            let minus_context = context.clone();
            let minus_button = Button::new("-1 dB").on_click(move |_ctx, data: &mut UiData, _env| {
                let new_db = (data.gain_db as f32 - 1.0).clamp(-30.0, 30.0);
                let setter = ParamSetter::new(minus_context.as_ref());
                setter.begin_set_parameter(&minus_params.gain);
                setter.set_parameter(&minus_params.gain, util::db_to_gain(new_db));
                setter.end_set_parameter(&minus_params.gain);
                data.gain_db = new_db as f64;
            })
            .controller(HoverPointerController);

            let plus_params = params.clone();
            let plus_context = context.clone();
            let plus_button = Button::new("+1 dB").on_click(move |_ctx, data: &mut UiData, _env| {
                let new_db = (data.gain_db as f32 + 1.0).clamp(-30.0, 30.0);
                let setter = ParamSetter::new(plus_context.as_ref());
                setter.begin_set_parameter(&plus_params.gain);
                setter.set_parameter(&plus_params.gain, util::db_to_gain(new_db));
                setter.end_set_parameter(&plus_params.gain);
                data.gain_db = new_db as f64;
            })
            .controller(HoverPointerController);

            let content = Flex::column()
                .with_child(Label::new("Gain GUI (Druid)").center())
                .with_spacer(8.0)
                .with_child(knob.center())
                .with_spacer(10.0)
                .with_child(
                    Label::new(|data: &UiData, _env: &_| format!("Gain: {:.1} dB", data.gain_db))
                        .center(),
                )
                .with_spacer(8.0)
                .with_child(slider)
                .with_spacer(8.0)
                .with_child(
                    Flex::row()
                        .with_flex_spacer(1.0)
                        .with_child(minus_button)
                        .with_spacer(8.0)
                        .with_child(plus_button)
                        .with_flex_spacer(1.0),
                )
                .with_spacer(12.0)
                .with_child(
                    Label::new(|data: &UiData, _env: &_| {
                        format!("Peak: {:>5.1} dB", data.peak_meter_db.max(util::MINUS_INFINITY_DB as f64))
                    })
                    .center(),
                )
                .padding(12.0)
                .controller(PollController {
                    params: params.clone(),
                    peak_meter: peak_meter.clone(),
                    context: context.clone(),
                    timer_id: TimerToken::INVALID,
                });

            let content = wrap_with_scale(
                editor_state.clone(),
                context.clone(),
                ResizableScaleConfig {
                    handle_size: RESIZE_HANDLE_SIZE,
                    min_scale_factor: MIN_USER_SCALE_FACTOR,
                    max_scale_factor: MAX_USER_SCALE_FACTOR,
                },
                content,
            );

            let (width, height) = editor_state.size();
            WindowDesc::new(content)
                .title("Gain GUI (Druid)")
                .window_size((width as f64, height as f64))
                .resizable(false)
        },
    )
}
