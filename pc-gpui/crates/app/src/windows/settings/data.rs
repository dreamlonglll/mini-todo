//! 「数据与同步」面板（§2.9）：导出 / 导入、WebDAV 表单（地址 / 用户名 / 密码，`hasPassword` 语义、http 明文提示）、
//! 测试连接、保存配置、自动同步与间隔、立即同步、「高级」里的强制覆盖（带二次确认）。
//!
//! 保存同步设置（保存配置、切换自动同步、改间隔、清除密码）后立即调用 `services::auto_sync::start` 按新设置重建
//! 自动同步定时器（旧版发 `app-settings-changed {key:'sync'}` 让主窗口重建；现在不用等设置窗口关闭）。

use chrono::{DateTime, Local};
use gpui_kit::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use mini_todo_core::sync::{self, SyncMode, SyncSettings};

use super::{
    DataBtnKind, SettingsWindow, SyncAction, card_hint, data_btn, form_label, panel_title,
    section_divider, settings_row,
};
use crate::assets::EpIcon;
use crate::db::{spawn_db, spawn_io_db};
use crate::platform::file_dialog::{self, FileFilter};
use crate::services;
use crate::state::AppState;
use crate::theme::{self, settings_tokens};
use crate::ui::ep_switch::EpSwitch;
use crate::ui::icon::icon;
use crate::ui::input::EpInput;
use crate::ui::message::{self, MessageKind};
use crate::ui::message_box::{self, BoxKind, ConfirmOptions};

const ZIP_FILTER: FileFilter = FileFilter {
    name: "ZIP 压缩包",
    extensions: &["zip"],
};
const IMPORT_FILTER: FileFilter = FileFilter {
    name: "ZIP / JSON 文件",
    extensions: &["zip", "json"],
};

/// 默认导出文件名（本地日期）
pub(crate) fn export_file_name(today: chrono::NaiveDate) -> String {
    format!("mini-todo-backup-{}.zip", today.format("%Y-%m-%d"))
}

/// 「上次同步: YYYY-MM-DD HH:mm」（`lastSyncAt` 为带偏移的 ISO 时间，按本地时间显示；解析不了显示原文）
pub(crate) fn format_last_sync(value: &str) -> String {
    let local = DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Local).naive_local())
        .ok()
        .or_else(|| mini_todo_core::db::time::parse_local_datetime(value));
    match local {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => value.to_string(),
    }
}

/// 地址是否为明文 http://（`/^http:\/\//i`，去首尾空白）
pub(crate) fn is_plain_http(url: &str) -> bool {
    let url = url.trim();
    url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://")
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if (bytes as f64) < MB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.2} MB", bytes as f64 / MB)
    }
}

impl SettingsWindow {
    pub(super) fn load_sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = spawn_db(cx, sync::get_settings);
        cx.spawn_in(window, async move |this, cx| match task.await {
            Ok(settings) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.apply_sync_settings(settings, window, cx)
                });
            }
            Err(e) => log::warn!("[settings] 读取同步设置失败: {e}"),
        })
        .detach();
    }

    fn apply_sync_settings(
        &mut self,
        settings: SyncSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.url_input.update(cx, |s, cx| {
            s.set_value(settings.webdav_url.clone(), window, cx)
        });
        self.user_input.update(cx, |s, cx| {
            s.set_value(settings.webdav_username.clone(), window, cx)
        });
        // 输入框永远不回显已保存的密码
        self.has_password = settings.has_password;
        let placeholder = if settings.has_password {
            "已保存（留空则不修改）"
        } else {
            "密码"
        };
        self.password_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(placeholder, window, cx);
        });
        self.has_sync_passphrase = settings.has_sync_passphrase;
        let pass_placeholder = if settings.has_sync_passphrase {
            "已设置加密口令（留空则不修改）"
        } else {
            "留空表示不加密"
        };
        self.passphrase_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(pass_placeholder, window, cx);
        });
        self.auto_sync = settings.auto_sync;
        self.sync_interval = settings.sync_interval;
        self.last_sync_at = settings.last_sync_at.clone();
        self.device_id = settings.device_id.clone();
        let interval = settings.sync_interval.to_string();
        self.interval_select
            .update(cx, |s, cx| s.set_value(Some(interval.into()), cx));
        cx.notify();
    }

    fn form_settings(
        &self,
        cx: &Context<Self>,
        clear_password: bool,
        clear_sync_passphrase: bool,
    ) -> SyncSettings {
        SyncSettings {
            webdav_url: self.url_input.read(cx).value().to_string(),
            webdav_username: self.user_input.read(cx).value().to_string(),
            webdav_password: if clear_password {
                String::new()
            } else {
                self.password_input.read(cx).value().to_string()
            },
            has_password: self.has_password,
            auto_sync: self.auto_sync,
            sync_interval: self.sync_interval,
            last_sync_at: self.last_sync_at.clone(),
            device_id: self.device_id.clone(),
            clear_password,
            sync_passphrase: if clear_sync_passphrase {
                String::new()
            } else {
                self.passphrase_input.read(cx).value().to_string()
            },
            has_sync_passphrase: self.has_sync_passphrase,
            clear_sync_passphrase,
        }
    }

    /// 旧 `persistSyncSettings`：保存整个表单 → 重建自动同步 → 重新读取（刷新 hasPassword、清空密码框）
    fn persist_sync_settings(
        &mut self,
        clear_password: bool,
        clear_sync_passphrase: bool,
        success_text: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.form_settings(cx, clear_password, clear_sync_passphrase);
        let task = spawn_db(cx, move |db| {
            sync::save_settings(db, &settings)?;
            sync::get_settings(db)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.clearing_password = false;
                this.clearing_passphrase = false;
                match result {
                    Ok(fresh) => {
                        services::auto_sync::start(cx);
                        this.apply_sync_settings(fresh, window, cx);
                        message::success(window, cx, success_text);
                    }
                    Err(e) => message::error_with(window, cx, "保存同步设置失败", &e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_sync_settings(false, false, "同步设置已保存", window, cx);
    }

    fn clear_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing_password {
            return;
        }
        let this = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            ConfirmOptions::new(
                "清除密码",
                "确定清除已保存的 WebDAV 密码吗？清除后需要重新输入密码才能同步。",
            )
            .confirm_text("清除")
            .cancel_text("取消")
            .kind(BoxKind::Warning),
            move |confirmed, window, cx| {
                if !confirmed {
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    this.clearing_password = true;
                    this.persist_sync_settings(true, false, "已清除保存的密码", window, cx);
                });
            },
        );
    }

    fn clear_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing_passphrase {
            return;
        }
        let this = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            ConfirmOptions::new(
                "清除加密口令",
                "确定清除已保存的端到端同步加密口令吗？如果云端数据目前处于加密状态，清除后将无法解密，除非先用此设备推送明文或重新输入原口令。",
            )
            .confirm_text("清除")
            .cancel_text("取消")
            .kind(BoxKind::Warning),
            move |confirmed, window, cx| {
                if !confirmed {
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    this.clearing_passphrase = true;
                    this.persist_sync_settings(false, true, "已清除加密口令", window, cx);
                });
            },
        );
    }

    fn test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).value().trim().to_string();
        if url.is_empty() {
            message::warning(window, cx, "请输入 WebDAV 服务器地址");
            return;
        }
        if self.testing {
            return;
        }
        let username = self.user_input.read(cx).value().to_string();
        let password = self.password_input.read(cx).value().to_string();
        self.testing = true;
        cx.notify();
        let task = spawn_io_db(cx, "webdav-test", move |db| {
            let password = (!password.is_empty()).then_some(password);
            sync::test_connection(db, &url, &username, password.as_deref())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.testing = false;
                match result {
                    Ok(_) => message::success(window, cx, "连接成功"),
                    Err(e) => message::error_with(window, cx, "连接失败", &e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 旧 `runSyncCommand`
    fn run_sync_command(
        &mut self,
        action: SyncAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.url_input.read(cx).value().trim().is_empty() {
            message::warning(window, cx, "请先配置 WebDAV 服务器");
            return;
        }
        if self.sync_action.is_some() {
            return;
        }
        self.sync_action = Some(action);
        cx.notify();
        let (mode, failure) = match action {
            SyncAction::Sync => (SyncMode::Smart { full: true }, "同步失败"),
            SyncAction::Pull => (SyncMode::ForcePull, "用云端覆盖本地失败"),
            SyncAction::Push => (SyncMode::ForcePush, "用本地覆盖云端失败"),
        };
        let task = services::sync::run_sync(cx, mode);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sync_action = None;
                match result {
                    Ok(report) => {
                        this.last_sync_at = Some(report.last_sync_at.clone());
                        message::show(
                            window,
                            cx,
                            services::sync::report_message_kind(&report),
                            services::sync::describe_sync_report(&report),
                        );
                        if let Some(text) = services::sync::describe_skipped_records(&report) {
                            message::warning(window, cx, text);
                        }
                    }
                    Err(e) if services::sync::is_sync_busy_error(&e) => {
                        message::info(window, cx, "已有同步正在进行，请稍后再试")
                    }
                    Err(e) => message::error_with(window, cx, failure, &e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_force(&mut self, action: SyncAction, window: &mut Window, cx: &mut Context<Self>) {
        let options = match action {
            SyncAction::Pull => ConfirmOptions::new(
                "用云端覆盖本地",
                "将让本地数据与云端完全一致：本地有而云端没有的待办和子任务会被删除，应用设置也会被云端覆盖（窗口位置等设备相关设置除外）。此操作无法撤销，建议先导出备份。确定继续吗？",
            )
            .confirm_text("覆盖本地"),
            SyncAction::Push => ConfirmOptions::new(
                "用本地覆盖云端",
                "将让云端数据与本地完全一致：云端有而本地没有的待办和子任务会被删除，其它设备与云端服务下次同步时也会随之删除。此操作无法撤销。确定继续吗？",
            )
            .confirm_text("覆盖云端"),
            SyncAction::Sync => return,
        };
        let this = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            options.cancel_text("取消").kind(BoxKind::Warning),
            move |confirmed, window, cx| {
                if confirmed {
                    let _ = this.update(cx, |this, cx| this.run_sync_command(action, window, cx));
                }
            },
        );
    }

    fn export_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exporting {
            return;
        }
        self.exporting = true;
        cx.notify();
        let name = export_file_name(Local::now().date_naive());
        let dialog = file_dialog::save_file(window, cx, "导出待办数据", &name, ZIP_FILTER);
        cx.spawn_in(window, async move |this, cx| {
            let result = match dialog.await {
                Some(path) => match cx.update(|_, cx| {
                    spawn_io_db(cx, "data-export", move |db| {
                        mini_todo_core::data::export_to_file(db, &path)
                    })
                }) {
                    Ok(task) => Some(task.await),
                    Err(_) => None,
                },
                None => None,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.exporting = false;
                match result {
                    Some(Ok(())) => message::success(window, cx, "导出成功"),
                    Some(Err(e)) => {
                        log::error!("[settings] 导出失败: {e}");
                        message::error(window, cx, format!("导出失败: {e}"));
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.importing {
            return;
        }
        let dialog = file_dialog::open_file(window, cx, "导入待办数据", IMPORT_FILTER);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = dialog.await else {
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                let weak = cx.entity().downgrade();
                message_box::confirm(
                    window,
                    cx,
                    ConfirmOptions::new("导入确认", "导入将覆盖现有的所有待办数据，确定继续吗？")
                        .confirm_text("确定导入")
                        .cancel_text("取消")
                        .kind(BoxKind::Warning),
                    move |confirmed, window, cx| {
                        if confirmed {
                            let _ = weak.update(cx, |this, cx| this.run_import(path, window, cx));
                        }
                    },
                );
            });
        })
        .detach();
    }

    fn run_import(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.importing = true;
        cx.notify();
        // ZIP / JSON 都交给 core 按文件头判断（大写 .ZIP 也按 ZIP 处理）
        let task = spawn_io_db(cx, "data-import", move |db| {
            mini_todo_core::data::import_from_file(db, &path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.importing = false;
                match result {
                    Ok(()) => {
                        // 替代 `data-imported`：重新加载设置（含运行时偏好）与待办，提示后关闭设置窗口。提示显示在打开设置的
                        // 清单 / 日历窗口（看不见时换一个看得见的，`services::notice`）
                        AppState::global(cx).update(cx, |state, cx| {
                            state.reload_app_settings(cx).detach();
                            state.reload_todos(cx);
                        });
                        services::notice::show_from(
                            cx,
                            this.target,
                            MessageKind::Success,
                            "数据导入成功",
                        );
                        window.remove_window();
                    }
                    Err(e) => {
                        log::error!("[settings] 导入失败: {e}");
                        message::error(window, cx, format!("导入失败: {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_cleanup_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cleaning_images {
            return;
        }
        let this = cx.entity().downgrade();
        message_box::confirm(
            window,
            cx,
            ConfirmOptions::new(
                "清理未引用图片",
                "将扫描本地图片目录并删除不再被任何待办或子任务引用的历史图片（最近 1 小时内创建的文件会保留，避免误删编辑中的草稿）。确定继续吗？",
            )
            .confirm_text("开始清理")
            .cancel_text("取消")
            .kind(BoxKind::Warning),
            move |confirmed, window, cx| {
                if confirmed {
                    let _ = this.update(cx, |this, cx| this.run_cleanup_images(window, cx));
                }
            },
        );
    }

    fn run_cleanup_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cleaning_images = true;
        cx.notify();
        let task = spawn_io_db(cx, "cleanup-images", |db| {
            mini_todo_core::todo::cleanup_unused_images(db, 3600)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.cleaning_images = false;
                match result {
                    Ok(report) if report.removed_count == 0 && report.skipped_recent == 0 => {
                        message::info(window, cx, "没有需要清理的未引用图片");
                    }
                    Ok(report) if report.removed_count == 0 => {
                        message::info(
                            window,
                            cx,
                            format!(
                                "未删除文件（有 {} 张未引用图片在 1 小时保护期内）",
                                report.skipped_recent
                            ),
                        );
                    }
                    Ok(report) => {
                        message::success(
                            window,
                            cx,
                            format!(
                                "已清理 {} 张未引用图片，释放 {}",
                                report.removed_count,
                                format_bytes(report.freed_bytes)
                            ),
                        );
                    }
                    Err(e) => {
                        log::error!("[settings] 清理未引用图片失败: {e}");
                        message::error_with(window, cx, "清理未引用图片失败", &e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_data(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = settings_tokens();
        let url = self.url_input.read(cx).value().to_string();
        let url_empty = url.trim().is_empty();
        let busy = self.sync_action.is_some();
        let sync_disabled = busy || url_empty;

        let on_export = cx.listener(|this, _, window, cx| this.export_data(window, cx));
        let on_import = cx.listener(|this, _, window, cx| this.import_data(window, cx));
        let on_cleanup = cx.listener(|this, _, window, cx| this.confirm_cleanup_images(window, cx));
        let data_actions = div()
            .flex()
            .gap(px(12.))
            .mb(px(12.))
            .child(data_btn(
                "export-btn",
                EpIcon::Download,
                if self.exporting {
                    "导出中..."
                } else {
                    "导出数据"
                },
                DataBtnKind::Primary,
                self.exporting,
                false,
                cx,
                on_export,
            ))
            .child(data_btn(
                "import-btn",
                EpIcon::Upload,
                if self.importing {
                    "导入中..."
                } else {
                    "导入数据"
                },
                DataBtnKind::Normal,
                self.importing,
                false,
                cx,
                on_import,
            ))
            .child(data_btn(
                "cleanup-images-btn",
                EpIcon::Delete,
                if self.cleaning_images {
                    "清理中..."
                } else {
                    "清理未引用图片"
                },
                DataBtnKind::Normal,
                self.cleaning_images,
                self.cleaning_images,
                cx,
                on_cleanup,
            ));

        let sync_time_size = theme::text_px(cx, 11.);
        let clear_size = theme::text_px(cx, 12.);
        let webdav_title = panel_title("云同步 (WebDAV)", cx).when_some(
            self.last_sync_at.as_deref().filter(|s| !s.is_empty()),
            |this, at| {
                this.child(
                    div()
                        .ml_auto()
                        .text_size(sync_time_size)
                        .font_weight(gpui_kit::FontWeight::NORMAL)
                        .text_color(t.tertiary)
                        .child(format!("上次同步: {}", format_last_sync(at))),
                )
            },
        );

        let on_clear_password = cx.listener(|this, _, window, cx| this.clear_password(window, cx));
        let password_label = form_label(cx)
            .child("密码")
            .when(self.has_password, |this| {
                let clearing = self.clearing_password;
                this.child(
                    div()
                        .id("clear-password")
                        .text_size(clear_size)
                        .font_weight(gpui_kit::FontWeight::NORMAL)
                        .text_color(t.primary)
                        .when(clearing, |this| this.opacity(0.6))
                        .when(!clearing, |this| {
                            this.cursor_pointer()
                                .hover(move |s| s.text_color(t.primary_dark).underline())
                                .on_click(on_clear_password)
                        })
                        .child("清除已保存的密码"),
                )
            });
        let toggle_icon = if self.password_visible {
            EpIcon::View
        } else {
            EpIcon::Hide
        };
        let password_toggle = div()
            .id("password-toggle")
            .flex()
            .items_center()
            .cursor_pointer()
            .child(icon(toggle_icon, 14., t.tertiary))
            .on_click(cx.listener(|this, _, window, cx| {
                this.password_visible = !this.password_visible;
                let masked = !this.password_visible;
                this.password_input
                    .update(cx, |s, cx| s.set_masked(masked, window, cx));
                cx.notify();
            }));

        // `el-input clearable`：非空且悬停 / 聚焦时右侧出现 CircleClose（`#A8ABB2`，悬停 `#909399`）
        let url_focused = self
            .url_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let show_clear = (self.url_hovered || url_focused) && !url.is_empty();
        let ep = theme::ep();
        let clear_button = div()
            .id("url-clear")
            .flex()
            .items_center()
            .cursor_pointer()
            .group("url-clear")
            .child(
                icon(EpIcon::CircleClose, 14., ep.text_placeholder)
                    .group_hover("url-clear", move |s| s.text_color(ep.text_secondary)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.url_input
                    .update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
            }));
        let url_field = div()
            .id("url-field")
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.url_hovered = *hovered;
                cx.notify();
            }))
            .child(
                EpInput::new(&self.url_input)
                    .small()
                    .w_full()
                    .when(show_clear, |input| input.suffix(clear_button)),
            );

        let on_test = cx.listener(|this, _, window, cx| this.test_connection(window, cx));
        let on_save_sync = cx.listener(|this, _, window, cx| this.save_sync_settings(window, cx));
        let form = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(form_label(cx).child("服务器地址"))
                    .child(url_field)
                    .when(is_plain_http(&url), |this| {
                        this.child(card_hint(
                            EpIcon::WarningFilled,
                            "当前地址使用 http://，账号密码与待办数据将以明文传输，建议改用 https://",
                            true,
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(form_label(cx).child("用户名"))
                            .child(EpInput::new(&self.user_input).small().w_full()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(password_label)
                            .child(
                                EpInput::new(&self.password_input)
                                    .small()
                                    .suffix(password_toggle)
                                    .w_full(),
                            ),
                    ),
            )
            // macOS 没有 DPAPI：密码与旧 2.5.0 mac 版一样明文存在本机数据库里（钥匙串要开发者签名，暂不做）
            .when(cfg!(target_os = "macos"), |this| {
                this.child(card_hint(
                    EpIcon::InfoFilled,
                    "macOS 上 WebDAV 密码以明文保存在本机数据库中",
                    false,
                    cx,
                ))
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .mt(px(4.))
                    .child(data_btn(
                        "test-connection",
                        EpIcon::Connection,
                        if self.testing { "测试中..." } else { "测试连接" },
                        DataBtnKind::Normal,
                        self.testing,
                        false,
                        cx,
                        on_test,
                    ))
                    .child(data_btn(
                        "save-sync",
                        EpIcon::Check,
                        "保存配置",
                        DataBtnKind::Primary,
                        false,
                        false,
                        cx,
                        on_save_sync,
                    )),
            );

        let auto_sync_switch = EpSwitch::new("auto-sync")
            .checked(self.auto_sync)
            .on_change(cx.listener(|this, value: &bool, window, cx| {
                this.auto_sync = *value;
                cx.notify();
                this.save_sync_settings(window, cx);
            }));

        let advanced_icon = if self.advanced_open {
            EpIcon::ArrowDown
        } else {
            EpIcon::ArrowRight
        };
        let advanced_toggle = div()
            .id("advanced-toggle")
            .mt(px(16.))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(clear_size)
            .text_color(t.secondary)
            .cursor_pointer()
            .group("advanced-toggle")
            .hover(move |s| s.text_color(t.label))
            .on_click(cx.listener(|this, _, _, cx| {
                this.advanced_open = !this.advanced_open;
                cx.notify();
            }))
            .child(
                icon(advanced_icon, 12., t.secondary)
                    .group_hover("advanced-toggle", move |s| s.text_color(t.label)),
            )
            .child("高级");

        let action_label = |action: SyncAction, idle: &'static str, running: &'static str| {
            if self.sync_action == Some(action) {
                running
            } else {
                idle
            }
        };

        let on_clear_passphrase =
            cx.listener(|this, _, window, cx| this.clear_passphrase(window, cx));
        let passphrase_label = form_label(cx).child("端到端同步加密口令（可选）").when(
            self.has_sync_passphrase,
            |this| {
                let clearing = self.clearing_passphrase;
                this.child(
                    div()
                        .id("clear-passphrase")
                        .text_size(clear_size)
                        .font_weight(gpui_kit::FontWeight::NORMAL)
                        .text_color(t.primary)
                        .when(clearing, |this| this.opacity(0.6))
                        .when(!clearing, |this| {
                            this.cursor_pointer()
                                .hover(move |s| s.text_color(t.primary_dark).underline())
                                .on_click(on_clear_passphrase)
                        })
                        .child("清除已保存的口令"),
                )
            },
        );
        let passphrase_toggle_icon = if self.passphrase_visible {
            EpIcon::View
        } else {
            EpIcon::Hide
        };
        let passphrase_toggle = div()
            .id("passphrase-toggle")
            .flex()
            .items_center()
            .cursor_pointer()
            .child(icon(passphrase_toggle_icon, 14., t.tertiary))
            .on_click(cx.listener(|this, _, window, cx| {
                this.passphrase_visible = !this.passphrase_visible;
                let masked = !this.passphrase_visible;
                this.passphrase_input
                    .update(cx, |s, cx| s.set_masked(masked, window, cx));
                cx.notify();
            }));

        let on_sync_now =
            cx.listener(|this, _, window, cx| this.run_sync_command(SyncAction::Sync, window, cx));
        let on_save_passphrase =
            cx.listener(|this, _, window, cx| this.save_sync_settings(window, cx));
        let on_force_pull =
            cx.listener(|this, _, window, cx| this.confirm_force(SyncAction::Pull, window, cx));
        let on_force_push =
            cx.listener(|this, _, window, cx| this.confirm_force(SyncAction::Push, window, cx));
        let passphrase_btn_w = (108. * theme::font_scale(cx)).round();

        div()
            .flex()
            .flex_col()
            .child(panel_title("数据管理", cx))
            .child(data_actions)
            .child(card_hint(
                EpIcon::InfoFilled,
                "导出为 ZIP 压缩包，可用于备份或迁移到其他设备；清理图片只删除超过 1 小时未被引用的本地图片",
                false,
                cx,
            ))
            .child(section_divider())
            .child(webdav_title)
            .child(form)
            .child(section_divider())
            .child(settings_row(
                EpIcon::Timer,
                "自动同步",
                Some("定时自动与云端合并同步"),
                auto_sync_switch,
                false,
                cx,
            ))
            .when(self.auto_sync, |this| {
                this.child(settings_row(
                    EpIcon::Clock,
                    "同步间隔",
                    None,
                    div().w(px(120.)).child(self.interval_select.clone()),
                    false,
                    cx,
                ))
            })
            .child(
                div().flex().gap(px(12.)).mt(px(16.)).mb(px(12.)).child(data_btn(
                    "sync-now",
                    EpIcon::Refresh,
                    action_label(SyncAction::Sync, "立即同步", "同步中..."),
                    DataBtnKind::Primary,
                    sync_disabled,
                    self.sync_action == Some(SyncAction::Sync),
                    cx,
                    on_sync_now,
                )),
            )
            .child(card_hint(
                EpIcon::InfoFilled,
                "通过 WebDAV 协议同步待办数据和图片：本地与云端的新增、修改、删除会自动合并，以较新的修改为准",
                false,
                cx,
            ))
            .child(div().flex().child(advanced_toggle))
            .when(self.advanced_open, |this| {
                this.child(
                    div()
                        .mt(px(8.))
                        .p(px(12.))
                        .border_1()
                        .border_color(t.advanced_border)
                        .rounded(theme::radius::MD)
                        .bg(t.advanced_bg)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .mb(px(12.))
                                .pb(px(12.))
                                .border_b_1()
                                .border_color(t.border)
                                .child(passphrase_label)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            div().flex_1().min_w_0().child(
                                                EpInput::new(&self.passphrase_input)
                                                    .small()
                                                    .suffix(passphrase_toggle)
                                                    .w_full(),
                                            ),
                                        )
                                        .child(div().min_w(px(passphrase_btn_w)).flex_none().child(data_btn(
                                            "save-passphrase",
                                            EpIcon::Check,
                                            "保存口令",
                                            DataBtnKind::Normal,
                                            false,
                                            false,
                                            cx,
                                            on_save_passphrase,
                                        ))),
                                )
                                .child(card_hint(
                                    EpIcon::InfoFilled,
                                    "开启后同步文档在本地加密再上传（ChaCha20-Poly1305）；所有 PC 与云端服务必须使用相同口令，遗忘口令将无法解密云端数据",
                                    false,
                                    cx,
                                )),
                        )
                        .child(card_hint(
                            EpIcon::WarningFilled,
                            "仅在两端数据出现异常时使用：会以一端为准整体覆盖另一端，被覆盖一端独有的数据将被删除",
                            false,
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .mt(px(12.))
                                .child(data_btn(
                                    "force-pull",
                                    EpIcon::Download,
                                    action_label(SyncAction::Pull, "用云端覆盖本地", "覆盖中..."),
                                    DataBtnKind::Danger,
                                    sync_disabled,
                                    false,
                                    cx,
                                    on_force_pull,
                                ))
                                .child(data_btn(
                                    "force-push",
                                    EpIcon::Upload,
                                    action_label(SyncAction::Push, "用本地覆盖云端", "覆盖中..."),
                                    DataBtnKind::Danger,
                                    sync_disabled,
                                    false,
                                    cx,
                                    on_force_push,
                                )),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_name_uses_local_date() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(export_file_name(day), "mini-todo-backup-2026-10-15.zip");
    }

    #[test]
    fn plain_http_detection_is_case_insensitive() {
        assert!(is_plain_http("http://dav.example.com"));
        assert!(is_plain_http("  HTTP://x"));
        assert!(!is_plain_http("https://dav.example.com"));
        assert!(!is_plain_http("http:/x"));
        assert!(!is_plain_http(""));
    }

    #[test]
    fn last_sync_is_shown_in_local_time() {
        // 带偏移的 ISO 时间按本地时区显示；解析不了显示原文
        let utc = "2026-10-15T01:42:00+00:00";
        let expected = DateTime::parse_from_rfc3339(utc)
            .unwrap()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        assert_eq!(format_last_sync(utc), expected);
        assert_eq!(format_last_sync("2026-10-15 09:42:13"), "2026-10-15 09:42");
        assert_eq!(format_last_sync("garbage"), "garbage");
    }

    #[test]
    fn format_bytes_human_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.00 MB");
    }
}
