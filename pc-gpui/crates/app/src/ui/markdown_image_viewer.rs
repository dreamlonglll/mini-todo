//! 图片预览（`el-image-viewer`，Element Plus 2.13）：满窗黑色 50% 遮罩；右上 (40, 40) 44×44 圆形关闭按钮
//! （底 `#606266`、白色 Close 24px、不透明度 0.8）；多张时左右 40px 处 44×44 圆形翻页按钮；底部 30px 处高 44、
//! 圆角 22、`padding 0 23px` 的工具条（缩小 / 放大 / 适配 ⇄ 原始尺寸 / 左旋 / 右旋，图标 23px、间距 22）。
//! Esc 关闭（由宿主窗口的 Esc 处理转交）、←/→ 翻页、滚轮缩放（×1.2，0.2~7）、拖动平移、循环翻页。
//!
//! GPUI 的位图不支持变换：旋转时在后台用 `image` crate 转好再显示（按 (路径, 角度) 缓存）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{
    ClickEvent, Context, FocusHandle, Image, ImageSource, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, ParentElement as _,
    Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, div, img, point, prelude::FluentBuilder as _, px,
};

use crate::assets::EpIcon;
use crate::theme::{self, ep};
use crate::ui::icon::icon;
use crate::ui::markdown::image_size;

/// 工具条按钮动作
type ViewerAction = Box<dyn Fn(&mut ImageViewer, &mut Context<ImageViewer>)>;

const ZOOM_RATE: f32 = 1.2;
const MIN_SCALE: f32 = 0.2;
const MAX_SCALE: f32 = 7.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// 适配窗口（contain）
    Contain,
    /// 原始尺寸
    Original,
}

pub struct ImageViewer {
    images: Vec<PathBuf>,
    index: usize,
    scale: f32,
    /// 顺时针旋转 90° 的次数（0..4）
    rotation: u8,
    offset: Point<Pixels>,
    drag_from: Option<(Point<Pixels>, Point<Pixels>)>,
    fit: Fit,
    rotated: HashMap<(usize, u8), Arc<Image>>,
    focus: FocusHandle,
    closed: bool,
}

impl ImageViewer {
    pub fn new(
        images: Vec<PathBuf>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            index: index.min(images.len().saturating_sub(1)),
            images,
            scale: 1.,
            rotation: 0,
            offset: point(px(0.), px(0.)),
            drag_from: None,
            fit: Fit::Contain,
            rotated: HashMap::new(),
            focus,
            closed: false,
        }
    }

    /// 宿主窗口每次渲染时检查：用户点了关闭
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        cx.notify();
    }

    fn reset(&mut self) {
        self.scale = 1.;
        self.rotation = 0;
        self.offset = point(px(0.), px(0.));
        self.fit = Fit::Contain;
    }

    fn switch(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.images.len() as isize;
        if len <= 1 {
            return;
        }
        self.index = (self.index as isize + delta).rem_euclid(len) as usize;
        self.reset();
        cx.notify();
    }

    fn zoom(&mut self, zoom_in: bool, cx: &mut Context<Self>) {
        self.scale = if zoom_in {
            (self.scale * ZOOM_RATE).min(MAX_SCALE)
        } else {
            (self.scale / ZOOM_RATE).max(MIN_SCALE)
        };
        cx.notify();
    }

    fn rotate(&mut self, clockwise: bool, cx: &mut Context<Self>) {
        self.rotation = if clockwise {
            (self.rotation + 1) % 4
        } else {
            (self.rotation + 3) % 4
        };
        let key = (self.index, self.rotation);
        if self.rotation != 0 && !self.rotated.contains_key(&key) {
            let path = self.images[self.index].clone();
            let rotation = self.rotation;
            let task = cx.background_executor().spawn(async move {
                let image = image::open(&path).map_err(|e| e.to_string())?;
                let image = match rotation {
                    1 => image.rotate90(),
                    2 => image.rotate180(),
                    _ => image.rotate270(),
                };
                let mut bytes = Vec::new();
                image
                    .write_to(
                        &mut std::io::Cursor::new(&mut bytes),
                        image::ImageFormat::Png,
                    )
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(bytes)
            });
            cx.spawn(async move |this, cx| match task.await {
                Ok(bytes) => {
                    let _ = this.update(cx, |this, cx| {
                        this.rotated.insert(
                            key,
                            Arc::new(Image::from_bytes(gpui_kit::ImageFormat::Png, bytes)),
                        );
                        cx.notify();
                    });
                }
                Err(e) => log::warn!("[image-viewer] 旋转图片失败: {e}"),
            })
            .detach();
        }
        cx.notify();
    }

    fn round_button(
        id: &'static str,
        source: EpIcon,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .absolute()
            .size(px(44.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(ep().text_regular)
            .opacity(0.8)
            .cursor_pointer()
            .on_click(on_click)
            .child(icon(source, 24., ep().white))
    }
}

impl Render for ImageViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let Some(path) = self.images.get(self.index).cloned() else {
            return div().into_any_element();
        };
        let (mut w, mut h) = image_size(&path)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or((vw * 0.6, vh * 0.6));
        if self.rotation % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        let base = match self.fit {
            Fit::Contain => (vw / w).min(vh / h).min(1.),
            Fit::Original => 1.,
        };
        let (dw, dh) = (w * base * self.scale, h * base * self.scale);
        let left = (vw - dw) / 2. + f32::from(self.offset.x);
        let top = (vh - dh) / 2. + f32::from(self.offset.y);
        let source: ImageSource = match self.rotated.get(&(self.index, self.rotation)) {
            Some(image) if self.rotation != 0 => ImageSource::Image(image.clone()),
            _ if self.rotation != 0 => ImageSource::from(path.clone()),
            _ => ImageSource::from(path),
        };
        let multiple = self.images.len() > 1;
        let entity = cx.entity().downgrade();
        let action = |id: &'static str, source: EpIcon, f: ViewerAction, cx: &mut Context<Self>| {
            div()
                .id(id)
                .cursor_pointer()
                .child(icon(source, 23., ep().white))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| f(this, cx)))
        };
        let divider = || {
            div()
                .w(px(1.))
                .h(px(18.))
                .bg(theme::rgba_css(255, 255, 255, 0.5))
        };
        let toolbar = div()
            .absolute()
            .bottom(px(30.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .h(px(44.))
                    .px(px(23.))
                    .rounded(px(22.))
                    .bg(ep().text_regular)
                    .opacity(0.8)
                    .flex()
                    .items_center()
                    .gap(px(22.))
                    .child(action(
                        "iv-zoom-out",
                        EpIcon::ZoomOut,
                        Box::new(|t, cx| t.zoom(false, cx)),
                        cx,
                    ))
                    .child(action(
                        "iv-zoom-in",
                        EpIcon::ZoomIn,
                        Box::new(|t, cx| t.zoom(true, cx)),
                        cx,
                    ))
                    .child(divider())
                    .child(action(
                        "iv-fit",
                        if self.fit == Fit::Contain {
                            EpIcon::FullScreen
                        } else {
                            EpIcon::ScaleToOriginal
                        },
                        Box::new(|t, cx| {
                            t.fit = if t.fit == Fit::Contain {
                                Fit::Original
                            } else {
                                Fit::Contain
                            };
                            t.scale = 1.;
                            t.offset = point(px(0.), px(0.));
                            cx.notify();
                        }),
                        cx,
                    ))
                    .child(divider())
                    .child(action(
                        "iv-rot-left",
                        EpIcon::RefreshLeft,
                        Box::new(|t, cx| t.rotate(false, cx)),
                        cx,
                    ))
                    .child(action(
                        "iv-rot-right",
                        EpIcon::RefreshRight,
                        Box::new(|t, cx| t.rotate(true, cx)),
                        cx,
                    )),
            );
        let close = {
            let entity = entity.clone();
            Self::round_button("iv-close", EpIcon::Close, move |_, _, cx| {
                let _ = entity.update(cx, |this, cx| this.close(cx));
            })
            .top(px(40.))
            .right(px(40.))
        };
        div()
            .id("image-viewer")
            .absolute()
            .inset_0()
            .occlude()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "left" => this.switch(-1, cx),
                    "right" => this.switch(1, cx),
                    "up" => this.zoom(true, cx),
                    "down" => this.zoom(false, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let dy = match event.delta {
                    ScrollDelta::Lines(p) => p.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                };
                if dy != 0. {
                    this.zoom(dy > 0., cx);
                }
                cx.stop_propagation();
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some((start, origin)) = this.drag_from {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.offset = origin + (event.position - start);
                        cx.notify();
                    } else {
                        this.drag_from = None;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag_from = None),
            )
            .child(div().absolute().inset_0().bg(theme::rgba_css(0, 0, 0, 0.5)))
            .child(
                div()
                    .id("iv-image")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(dw))
                    .h(px(dh))
                    .cursor_move()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.drag_from = Some((event.position, this.offset));
                            cx.stop_propagation();
                        }),
                    )
                    .child(img(source).size_full().object_fit(ObjectFit::Fill)),
            )
            .child(close)
            .when(multiple, |this| {
                let prev = entity.clone();
                let next = entity.clone();
                this.child(
                    Self::round_button("iv-prev", EpIcon::ArrowLeft, move |_, _, cx| {
                        let _ = prev.update(cx, |this, cx| this.switch(-1, cx));
                    })
                    .left(px(40.))
                    .top(px(vh / 2. - 22.)),
                )
                .child(
                    Self::round_button("iv-next", EpIcon::ArrowRight, move |_, _, cx| {
                        let _ = next.update(cx, |this, cx| this.switch(1, cx));
                    })
                    .right(px(40.))
                    .top(px(vh / 2. - 22.)),
                )
            })
            .child(toolbar)
            .into_any_element()
    }
}
