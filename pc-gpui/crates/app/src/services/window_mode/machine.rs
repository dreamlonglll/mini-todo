//! 每个桌面块窗口的模式状态机（纯逻辑：不碰 GPUI / Win32 / AppKit，全平台编译带单测）。
//!
//! `window_mode`（`mod.rs`）在 `cx.update` 里调这里的方法算出要做的 [`Op`] 并更新状态，再在 update 之外执行
//! （`.trellis/spec/pc-gpui/gpui-conventions.md` §2.2）。每个窗口一份 [`ModeState`]，互不影响；
//! 嵌入桌面 / 贴边隐藏 / 唤起置顶三个偏好是全局的（`RuntimePrefs`、`fixed_embed_desktop`），作用于每个锁定的窗口。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use mini_todo_core::db::WindowPosition;
use mini_todo_core::settings::RuntimePrefs;
use mini_todo_core::window_logic::{
    self, AutoHideState, AutoHideTransition, MonitorBounds, WindowRect,
};

use crate::platform::PhysRect;
use crate::windows::block::BlockKey;
use crate::windows::calendar_window::logic as calendar_logic;

/// 目标模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Normal,
    ClassicFixed,
    Desktop,
}

impl TargetMode {
    /// `applyFixedMode` / `applyNormalMode` 的选择：固定时 Windows 且开了"嵌入桌面"→ 嵌入，否则经典固定
    pub fn for_state(is_fixed: bool, embed_desktop: bool) -> TargetMode {
        match (is_fixed, embed_desktop && cfg!(windows)) {
            (false, _) => TargetMode::Normal,
            (true, true) => TargetMode::Desktop,
            (true, false) => TargetMode::ClassicFixed,
        }
    }
}

/// 锁定的窗口是否按"经典固定"的外观画（无圆角、无描边）：锁定且没有嵌入桌面（嵌入只在 Windows 上生效）
pub fn classic_fixed_look(fixed: bool, embed_desktop: bool) -> bool {
    fixed && !(cfg!(windows) && embed_desktop)
}

/// 一个要在 App 借用之外执行的窗口操作（`mod.rs` 的 `run_op`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    MoveTo(i32, i32),
    Topmost(bool),
    ToolWindow(bool),
    LockGeometry(bool),
    Attach,
    Detach { tool_window: bool },
    Restore,
}

/// 一组按顺序执行的窗口操作
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ops {
    pub steps: Vec<Op>,
}

impl Ops {
    fn push(&mut self, op: Op) {
        self.steps.push(op);
    }
}

/// 200ms 轮询读到的窗口实况（只读调用，`mod.rs` 按窗口当前模式只读需要的几项）
#[derive(Debug, Clone, Copy, Default)]
pub struct TickFacts {
    /// 嵌入态：`platform::embed::needs_reattach`（Explorer 重启 / owner 被清 / 样式被抹）
    pub needs_reattach: bool,
    /// 经典固定：被 Win+D / Win+M（macOS ⌘M）最小化了
    pub minimized: bool,
    /// 经典固定：`WS_EX_TOOLWINDOW` 丢了（只在 Windows 上判断；macOS 的 ex style 恒 0）
    pub lacks_tool_window: bool,
    /// 经典固定且开着贴边隐藏：窗口客户区（物理像素）
    pub client_rect: Option<PhysRect>,
    /// 客户区所在的整块显示器（贴边判断用整块显示器，不是工作区）
    pub monitor: Option<MonitorBounds>,
    /// 光标（物理像素；Linux 取不到）
    pub cursor: Option<(i32, i32)>,
}

/// 贴边隐藏这一 tick 要把窗口客户区左上角挪到 `(x, y)`（物理像素；失败要回滚状态，见 [`ModeState::hide_failed`] /
/// [`ModeState::restore_failed`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoHideMove {
    Hide { x: i32, y: i32 },
    Restore { x: i32, y: i32, topmost: bool },
}

/// 一个窗口这一 tick 要做的事
#[derive(Debug, Default, PartialEq)]
pub struct TickPlan {
    pub ops: Ops,
    pub auto_hide: Option<AutoHideMove>,
}

/// 托盘 / 单实例 / Dock 唤起一个窗口时要做的事
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wake {
    /// 嵌入桌面：只显示 + 聚焦，不置顶
    pub desktop: bool,
    /// 贴边隐藏中：先拉回的锚点（客户区左上角，物理像素）
    pub anchor: Option<(i32, i32)>,
}

fn point(position: WindowPosition) -> (i32, i32) {
    (position.x, position.y)
}

/// 一个窗口的模式运行时状态（原 `WindowMode` 单例的字段，旧版 `IS_FIXED_MODE` / `IS_DESKTOP_MODE` / `AUTO_HIDE_STATE` /
/// `TOP_ON_WAKE` / `DESIRED_TOPMOST` / `PENDING_AUTO_HIDE_RELEASE`）
#[derive(Debug)]
pub struct ModeState {
    /// 用户可见的锁定（标题栏锁按钮；经典固定与嵌入桌面都为 true；按屏幕组合持久化）
    pub fixed: bool,
    /// 窗口侧：经典固定
    pub classic_fixed: bool,
    /// 窗口侧：固定 + 嵌入桌面（与 `classic_fixed` 不会同时为 true）
    pub desktop: bool,
    pub auto_hide: AutoHideState,
    /// 唤起置顶偏好的缓存（全局偏好；每个窗口一份，`apply_runtime_prefs` 才能各自发现"被关掉了"）
    top_on_wake: AtomicBool,
    /// 期望的置顶态（贴边唤起 / 托盘唤起）
    pub desired_topmost: bool,
    /// 偏好被关掉后欠一次窗口收尾（拉回藏起的窗口、撤置顶），下个 tick 处理
    pending_release: bool,
}

impl ModeState {
    /// 新登记的窗口：窗口侧总是从普通模式开始（锁定的窗口建好后由调用方 `apply_mode` 进入固定）
    pub fn new(fixed: bool, top_on_wake: bool) -> Self {
        Self {
            fixed,
            classic_fixed: false,
            desktop: false,
            auto_hide: AutoHideState::default(),
            top_on_wake: AtomicBool::new(top_on_wake),
            desired_topmost: false,
            pending_release: false,
        }
    }

    /// 窗口侧是否处在某种固定模式
    pub fn is_window_fixed(&self) -> bool {
        self.classic_fixed || self.desktop
    }

    /// 唤起置顶偏好的缓存
    pub fn top_on_wake(&self) -> bool {
        self.top_on_wake.load(Ordering::SeqCst)
    }

    /// 经典固定 + 贴边隐藏藏在屏幕边缘（不算吸附目标；macOS 上被 Dock / ⌘Tab 激活时要拉回）
    pub fn is_edge_hidden(&self) -> bool {
        self.classic_fixed && self.auto_hide.hidden
    }

    /// 计算切到 `target` 要做的事，并更新状态
    pub fn plan_mode(&mut self, prefs: RuntimePrefs, target: TargetMode) -> Ops {
        let mut ops = Ops::default();
        self.top_on_wake.store(prefs.top_on_wake, Ordering::SeqCst);
        match target {
            TargetMode::ClassicFixed => {
                if self.desktop {
                    self.desktop = false;
                    // 嵌入 → 经典固定：保持 TOOLWINDOW，不闪任务栏
                    ops.push(Op::Detach { tool_window: true });
                }
                self.classic_fixed = true;
                if let Some(anchor) = self.auto_hide.enter_fixed_mode(prefs.auto_hide_enabled) {
                    ops.push(Op::MoveTo(anchor.x, anchor.y));
                }
                ops.push(Op::LockGeometry(true));
                ops.push(Op::ToolWindow(true));
                ops.push(Op::Topmost(self.desired_topmost));
            }
            TargetMode::Desktop => {
                if self.classic_fixed {
                    // 先撤置顶再退出经典固定（否则解锁后一直置顶）
                    ops.push(Op::Topmost(false));
                    self.desired_topmost = false;
                    self.classic_fixed = false;
                }
                if let Some(anchor) = self.auto_hide.leave_fixed_mode() {
                    ops.push(Op::MoveTo(anchor.x, anchor.y));
                }
                self.desktop = true;
                ops.push(Op::LockGeometry(true));
                ops.push(Op::Attach);
            }
            TargetMode::Normal => {
                if self.classic_fixed || self.desired_topmost {
                    ops.push(Op::Topmost(false));
                }
                self.desired_topmost = false;
                self.classic_fixed = false;
                if let Some(anchor) = self.auto_hide.leave_fixed_mode() {
                    ops.push(Op::MoveTo(anchor.x, anchor.y));
                }
                if self.desktop {
                    self.desktop = false;
                    ops.push(Op::Detach { tool_window: false });
                }
                ops.push(Op::LockGeometry(false));
                ops.push(Op::ToolWindow(false));
            }
        }
        ops
    }

    /// 运行时偏好变化（设置窗口改了贴边隐藏 / 唤起置顶、导入、同步应用了远端设置、保存窗口状态后）：刷新状态机，
    /// 有功能被关掉时下个 tick 收尾（旧 `reload_runtime_prefs`）
    pub fn apply_runtime_prefs(&mut self, prefs: RuntimePrefs) {
        let active = prefs.auto_hide_enabled && self.classic_fixed;
        if window_logic::apply_runtime_prefs(
            &mut self.auto_hide,
            &self.top_on_wake,
            prefs.top_on_wake,
            active,
        ) {
            self.pending_release = true;
        }
    }

    /// "嵌入桌面"偏好在已锁定时被切换（设置窗口 / 同步）：要直接切到的模式；未锁定、非 Windows、已是目标形态时 `None`
    pub fn embed_switch(&self, enabled: bool) -> Option<TargetMode> {
        if !self.fixed || !cfg!(windows) || enabled == self.desktop {
            return None;
        }
        Some(TargetMode::for_state(true, enabled))
    }

    /// 200ms 轮询：按窗口实况决定这一 tick 的操作（嵌入重挂、经典固定的最小化守护与样式自愈、偏好关掉后的收尾、贴边隐藏）
    pub fn plan_tick(&mut self, facts: &TickFacts, now: Instant) -> TickPlan {
        let mut plan = TickPlan::default();
        if self.desktop {
            if facts.needs_reattach {
                plan.ops.push(Op::Attach);
            }
        } else if self.classic_fixed {
            if facts.minimized {
                // 固定模式没有任务栏入口：被 Win+D / Win+M 最小化后立刻还原
                plan.ops.push(Op::Restore);
            }
            if facts.lacks_tool_window {
                plan.ops.push(Op::ToolWindow(true));
            }
            if self.pending_release {
                self.pending_release = false;
                if let Some(anchor) = window_logic::take_disabled_hidden_anchor(&mut self.auto_hide)
                {
                    plan.ops.push(Op::MoveTo(anchor.x, anchor.y));
                }
                if !self.top_on_wake() || !self.auto_hide.enabled {
                    self.desired_topmost = false;
                    plan.ops.push(Op::Topmost(false));
                }
            }
            if self.auto_hide.enabled
                && let Some(rect) = facts.client_rect
                && let Some(monitor) = facts.monitor.or(self.auto_hide.monitor_bounds)
            {
                let window = WindowRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                };
                match window_logic::evaluate_auto_hide_transition(
                    &mut self.auto_hide,
                    window,
                    monitor,
                    facts.cursor,
                    now,
                ) {
                    AutoHideTransition::None => {}
                    AutoHideTransition::Hide { hidden, .. } => {
                        plan.auto_hide = Some(AutoHideMove::Hide {
                            x: hidden.x,
                            y: hidden.y,
                        });
                    }
                    AutoHideTransition::Restore { anchor } => {
                        plan.auto_hide = Some(AutoHideMove::Restore {
                            x: anchor.x,
                            y: anchor.y,
                            topmost: self.top_on_wake(),
                        });
                    }
                }
            }
        } else if self.pending_release {
            self.pending_release = false;
        }
        plan
    }

    /// 贴边隐藏：已挪到屏幕边缘（随后撤置顶）
    pub fn hidden(&mut self) {
        self.desired_topmost = false;
    }

    /// 贴边隐藏：挪窗口失败，回滚为显示态
    pub fn hide_failed(&mut self) {
        self.auto_hide.hidden = false;
        self.auto_hide.edge_stick_started_at = None;
    }

    /// 贴边唤起：挪回锚点失败，仍算藏着
    pub fn restore_failed(&mut self) {
        self.auto_hide.hidden = true;
    }

    /// 托盘唤起（旧 `bring_main_window_to_front` 的状态部分）：嵌入桌面只显示 + 聚焦；其它模式贴边隐藏开着时给 3s 宽限、
    /// 藏起中返回锚点，并期望置顶
    pub fn wake(&mut self, now: Instant) -> Wake {
        if self.desktop {
            return Wake {
                desktop: true,
                anchor: None,
            };
        }
        let anchor = self.auto_hide.wake_from_tray(now).map(point);
        self.desired_topmost = true;
        Wake {
            desktop: false,
            anchor,
        }
    }

    /// 托盘唤起的临时置顶到点（`latest`：这仍是最近一次唤起）：贴边唤起态需要保持置顶时不撤。返回是否要撤置顶
    pub fn release_wake_topmost(&mut self, latest: bool) -> bool {
        let stay = self.auto_hide.should_stay_on_top(self.top_on_wake());
        let release = latest && !stay;
        if release {
            self.desired_topmost = false;
        }
        release
    }

    /// 托盘「重置位置」：清贴边运行时状态、不再期望置顶（调用方随后撤置顶、摆窗）
    pub fn reset_runtime(&mut self) {
        self.auto_hide.clear_runtime();
        self.desired_topmost = false;
    }
}

/// 「重置位置」时相邻两个窗口错开的距离（逻辑像素）
pub const RESET_CASCADE_STEP_DIP: f64 = 32.;

/// 「重置位置」后窗口的尺寸（逻辑像素）：清单窗口 380×600（以前的主窗口）；日历窗口与清单窗口等高、宽度按日格大致是方的
/// （`calendar_window::logic::default_width`，600 高时 596，不小于日历窗口的最小宽度）
pub fn reset_size(key: BlockKey) -> (f64, f64) {
    let height = window_logic::RESET_WINDOW_HEIGHT;
    match key {
        BlockKey::List(_) => (window_logic::RESET_WINDOW_WIDTH, height),
        BlockKey::Calendar => (calendar_logic::default_width(height), height),
    }
}

/// 托盘「重置位置」第 `index` 个窗口（逻辑尺寸 `size`）的客户区（物理像素）：第一个与以前的主窗口相同——主显示器左上起各留
/// `尺寸 × 10% ÷ 缩放` 的边距；之后每个向右下错开 32 逻辑像素，这个窗口再错开就超出显示器右边或下边时从第一个的位置重新开始
pub fn reset_rect(monitor: MonitorBounds, scale: f64, index: usize, size: (f64, f64)) -> PhysRect {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let base = window_logic::reset_window_position(monitor, scale);
    let width = (size.0 * scale).round() as i32;
    let height = (size.1 * scale).round() as i32;
    let step = (RESET_CASCADE_STEP_DIP * scale).round().max(1.) as i32;
    let room_x = (monitor.right() - (base.x + width)).max(0) / step;
    let room_y = (monitor.bottom() - (base.y + height)).max(0) / step;
    let slots = (room_x.min(room_y) + 1).max(1) as usize;
    let offset = (index % slots) as i32 * step;
    PhysRect::new(base.x + offset, base.y + offset, width, height)
}

/// 托盘「重置位置」的排列：`keys` 按给出的顺序（默认清单、其它清单按 id、日历最后）在主显示器 `monitor` 上依次错开
/// （[`reset_rect`]，尺寸见 [`reset_size`]）。返回每个窗口的客户区（物理像素）
pub fn reset_plan(
    monitor: MonitorBounds,
    scale: f64,
    keys: &[BlockKey],
) -> Vec<(BlockKey, PhysRect)> {
    keys.iter()
        .enumerate()
        .map(|(index, key)| (*key, reset_rect(monitor, scale, index, reset_size(*key))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MONITOR: MonitorBounds = MonitorBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn prefs(top_on_wake: bool, auto_hide_enabled: bool) -> RuntimePrefs {
        RuntimePrefs {
            top_on_wake,
            auto_hide_enabled,
        }
    }

    /// 经典固定、开着贴边隐藏、窗口贴在屏幕顶边
    fn docked_classic() -> ModeState {
        let mut state = ModeState::new(true, true);
        state.plan_mode(prefs(true, true), TargetMode::ClassicFixed);
        state
    }

    fn docked_facts(cursor: (i32, i32)) -> TickFacts {
        TickFacts {
            client_rect: Some(PhysRect::new(100, 0, 380, 600)),
            monitor: Some(MONITOR),
            cursor: Some(cursor),
            ..TickFacts::default()
        }
    }

    #[test]
    fn target_mode_follows_embed_preference_only_on_windows() {
        assert_eq!(TargetMode::for_state(false, true), TargetMode::Normal);
        assert_eq!(TargetMode::for_state(true, false), TargetMode::ClassicFixed);
        let embedded = TargetMode::for_state(true, true);
        if cfg!(windows) {
            assert_eq!(embedded, TargetMode::Desktop);
        } else {
            // 非 Windows：同步来的"嵌入桌面"偏好不能让固定失败，回退为经典固定
            assert_eq!(embedded, TargetMode::ClassicFixed);
        }
    }

    #[test]
    fn classic_look_only_when_fixed_without_embedding() {
        assert!(!classic_fixed_look(false, false));
        assert!(!classic_fixed_look(false, true));
        assert!(classic_fixed_look(true, false));
        // 嵌入桌面只在 Windows 上存在：其它平台锁定一律按经典固定画
        assert_eq!(classic_fixed_look(true, true), !cfg!(windows));
    }

    #[test]
    fn mode_switches_keep_the_win32_ordering_rules() {
        let mut state = ModeState::new(true, true);
        let ops = state.plan_mode(prefs(true, false), TargetMode::ClassicFixed);
        assert_eq!(
            ops.steps,
            vec![
                Op::LockGeometry(true),
                Op::ToolWindow(true),
                Op::Topmost(false)
            ]
        );
        assert!(state.classic_fixed && !state.desktop && state.is_window_fixed());

        // 经典固定 → 嵌入：先撤置顶再离开经典固定
        state.desired_topmost = true;
        let ops = state.plan_mode(prefs(true, false), TargetMode::Desktop);
        assert_eq!(
            ops.steps,
            vec![Op::Topmost(false), Op::LockGeometry(true), Op::Attach]
        );
        assert!(!state.classic_fixed && state.desktop && !state.desired_topmost);

        // 嵌入 → 经典固定：Detach 时保持 TOOLWINDOW（不闪任务栏）
        let ops = state.plan_mode(prefs(true, false), TargetMode::ClassicFixed);
        assert_eq!(ops.steps[0], Op::Detach { tool_window: true });
        assert!(state.classic_fixed && !state.desktop);

        // 回普通模式：撤置顶、解锁、回到任务栏
        let ops = state.plan_mode(prefs(true, false), TargetMode::Normal);
        assert_eq!(
            ops.steps,
            vec![
                Op::Topmost(false),
                Op::LockGeometry(false),
                Op::ToolWindow(false)
            ]
        );
        assert!(!state.is_window_fixed());
    }

    #[test]
    fn leaving_fixed_mode_pulls_an_edge_hidden_window_back_first() {
        let mut state = docked_classic();
        state.auto_hide.hidden = true;
        state.auto_hide.anchor_position = Some(WindowPosition { x: 100, y: 0 });
        assert!(state.is_edge_hidden());
        let ops = state.plan_mode(prefs(true, true), TargetMode::Normal);
        assert_eq!(ops.steps[..2], [Op::Topmost(false), Op::MoveTo(100, 0)]);
        assert!(!state.is_edge_hidden());
    }

    #[test]
    fn embed_preference_switches_only_locked_windows_on_windows() {
        let locked_classic = docked_classic();
        let mut locked_desktop = ModeState::new(true, true);
        locked_desktop.plan_mode(prefs(true, false), TargetMode::Desktop);
        let unlocked = ModeState::new(false, true);

        if cfg!(windows) {
            assert_eq!(locked_classic.embed_switch(true), Some(TargetMode::Desktop));
            assert_eq!(locked_classic.embed_switch(false), None, "已是经典固定");
            assert_eq!(
                locked_desktop.embed_switch(false),
                Some(TargetMode::ClassicFixed)
            );
            assert_eq!(locked_desktop.embed_switch(true), None, "已嵌入");
        } else {
            assert_eq!(locked_classic.embed_switch(true), None);
            assert_eq!(locked_desktop.embed_switch(false), None);
        }
        assert_eq!(
            unlocked.embed_switch(true),
            None,
            "未锁定：下次锁定时才生效"
        );
    }

    #[test]
    fn tick_guards_each_mode_independently() {
        let now = Instant::now();
        // 经典固定：被最小化 → 还原；样式被抹 → 补 TOOLWINDOW；嵌入相关的实况不看
        let mut classic = ModeState::new(true, true);
        classic.plan_mode(prefs(true, false), TargetMode::ClassicFixed);
        let facts = TickFacts {
            minimized: true,
            lacks_tool_window: true,
            needs_reattach: true,
            ..TickFacts::default()
        };
        assert_eq!(
            classic.plan_tick(&facts, now).ops.steps,
            vec![Op::Restore, Op::ToolWindow(true)]
        );

        // 嵌入：只看要不要重挂
        let mut desktop = ModeState::new(true, true);
        desktop.plan_mode(prefs(true, false), TargetMode::Desktop);
        assert_eq!(desktop.plan_tick(&facts, now).ops.steps, vec![Op::Attach]);
        let healthy = TickFacts::default();
        assert!(desktop.plan_tick(&healthy, now).ops.steps.is_empty());

        // 普通窗口：什么都不做
        let mut normal = ModeState::new(false, true);
        assert_eq!(normal.plan_tick(&facts, now), TickPlan::default());
    }

    #[test]
    fn edge_hiding_runs_per_window() {
        let now = Instant::now();
        let mut docked = docked_classic();
        let mut other = docked_classic();
        // 两个窗口都贴在顶边：只有光标离开的那个藏起来
        let away = docked_facts((1500, 900));
        let inside = docked_facts((200, 100));
        assert_eq!(
            docked.plan_tick(&away, now).auto_hide,
            None,
            "刚贴边：开始计时"
        );
        assert_eq!(other.plan_tick(&inside, now).auto_hide, None);
        let later = now + window_logic::EDGE_HIDE_DELAY + Duration::from_millis(50);
        let plan = docked.plan_tick(&away, later);
        assert_eq!(
            plan.auto_hide,
            Some(AutoHideMove::Hide {
                x: 100,
                y: -600 + window_logic::HIDDEN_VISIBLE_STRIP_PX
            })
        );
        assert!(docked.is_edge_hidden());
        assert_eq!(other.plan_tick(&inside, later).auto_hide, None);
        assert!(!other.is_edge_hidden(), "另一个窗口不受影响");

        // 挪窗失败：回滚为显示态
        docked.hide_failed();
        assert!(!docked.is_edge_hidden());
    }

    #[test]
    fn turning_a_preference_off_releases_every_window() {
        let now = Instant::now();
        let mut a = docked_classic();
        let mut b = docked_classic();
        a.desired_topmost = true;
        b.desired_topmost = true;
        // 同一个全局偏好对每个窗口都生效：两个窗口都欠一次收尾（各自的缓存都能发现"被关掉了"）
        a.apply_runtime_prefs(prefs(false, true));
        b.apply_runtime_prefs(prefs(false, true));
        let facts = TickFacts::default();
        assert!(
            a.plan_tick(&facts, now)
                .ops
                .steps
                .contains(&Op::Topmost(false))
        );
        assert!(
            b.plan_tick(&facts, now)
                .ops
                .steps
                .contains(&Op::Topmost(false))
        );
        assert!(!a.desired_topmost && !b.desired_topmost);
        // 只收尾一次
        assert!(a.plan_tick(&facts, now).ops.steps.is_empty());
    }

    #[test]
    fn tray_wake_pulls_hidden_windows_back_and_releases_the_topmost_later() {
        let now = Instant::now();
        let mut hidden = docked_classic();
        hidden.auto_hide.hidden = true;
        hidden.auto_hide.anchor_position = Some(WindowPosition { x: 100, y: 0 });
        let wake = hidden.wake(now);
        assert_eq!(
            wake,
            Wake {
                desktop: false,
                anchor: Some((100, 0))
            }
        );
        assert!(hidden.desired_topmost && !hidden.is_edge_hidden());
        assert!(hidden.release_wake_topmost(true));
        assert!(!hidden.desired_topmost);

        // 嵌入桌面：只显示 + 聚焦
        let mut desktop = ModeState::new(true, true);
        desktop.plan_mode(prefs(true, false), TargetMode::Desktop);
        assert_eq!(
            desktop.wake(now),
            Wake {
                desktop: true,
                anchor: None
            }
        );
        assert!(!desktop.desired_topmost);

        // 贴边唤起态且开着唤起置顶：到点不撤；不是最近一次唤起：也不撤
        let mut docked = docked_classic();
        docked.auto_hide.docked_edge = Some(window_logic::DockEdge::Top);
        docked.wake(now);
        assert!(!docked.release_wake_topmost(true));
        let mut normal = ModeState::new(false, true);
        normal.wake(now);
        assert!(!normal.release_wake_topmost(false));
        assert!(normal.desired_topmost);
    }

    #[test]
    fn reset_keeps_the_first_window_where_the_main_window_used_to_go() {
        let monitor = MonitorBounds {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        const LIST: (f64, f64) = (380., 600.);
        let first = reset_rect(monitor, 1.25, 0, LIST);
        let pos = window_logic::reset_window_position(monitor, 1.25);
        assert_eq!(first, PhysRect::new(pos.x, pos.y, 475, 750));
        // 之后的窗口向右下错开 32 逻辑像素（125% 下 40 物理像素）
        assert_eq!(
            reset_rect(monitor, 1.25, 1, LIST),
            PhysRect::new(pos.x + 40, pos.y + 40, 475, 750)
        );
        assert_eq!(
            reset_rect(monitor, 1.25, 3, LIST),
            PhysRect::new(pos.x + 120, pos.y + 120, 475, 750)
        );
        // 偏移的显示器
        let offset = MonitorBounds {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            reset_rect(offset, 0., 0, LIST),
            PhysRect::new(-1920 + 192, 108, 380, 600)
        );
    }

    #[test]
    fn reset_wraps_around_instead_of_leaving_the_monitor() {
        const LIST: (f64, f64) = (380., 600.);
        // 1920×1080 @100%：第一个在 (192, 108)，下边还能错开 (1080 − 708) / 32 = 11 次
        let slots = 12;
        let last = reset_rect(MONITOR, 1., slots - 1, LIST);
        assert!(last.bottom() <= MONITOR.bottom() && last.right() <= MONITOR.right());
        assert_eq!(
            reset_rect(MONITOR, 1., slots, LIST),
            reset_rect(MONITOR, 1., 0, LIST)
        );
        // 每个窗口按自己的尺寸判断：1366×768 上第一个在 (136, 76)
        let small = MonitorBounds {
            x: 0,
            y: 0,
            width: 1366,
            height: 768,
        };
        // 600 高：下边只剩 768 − 76 − 600 = 92，只够错开 2 次
        assert_eq!(reset_rect(small, 1., 2, LIST).y, 76 + 64);
        assert_eq!(
            reset_rect(small, 1., 3, LIST),
            reset_rect(small, 1., 0, LIST)
        );
        // 1200 宽：右边只剩 30，不够错开一次，都叠在第一个位置；同样高的 380 宽窗口照常错开
        let wide = (1200., 400.);
        assert_eq!(
            reset_rect(small, 1., 1, wide),
            reset_rect(small, 1., 0, wide)
        );
        assert_eq!(reset_rect(small, 1., 1, (380., 400.)).x, 136 + 32);
        // 显示器比窗口还小：都叠在第一个位置
        let tiny = MonitorBounds {
            x: 0,
            y: 0,
            width: 300,
            height: 300,
        };
        assert_eq!(reset_rect(tiny, 1., 5, LIST), reset_rect(tiny, 1., 0, LIST));
    }

    #[test]
    fn reset_plan_cascades_lists_then_the_calendar() {
        let keys = [
            BlockKey::DEFAULT_LIST,
            BlockKey::List(1_762_000_000_000_001),
            BlockKey::Calendar,
        ];
        let plan = reset_plan(MONITOR, 1., &keys);
        assert_eq!(
            plan,
            vec![
                (BlockKey::DEFAULT_LIST, PhysRect::new(192, 108, 380, 600)),
                (
                    BlockKey::List(1_762_000_000_000_001),
                    PhysRect::new(224, 140, 380, 600)
                ),
                // 日历与清单窗口等高，宽度让日格大致是方的
                (BlockKey::Calendar, PhysRect::new(256, 172, 596, 600)),
            ]
        );
        assert_eq!(reset_size(BlockKey::Calendar), (596., 600.));
        assert!(reset_size(BlockKey::Calendar).0 >= f64::from(calendar_logic::MIN_WIDTH));
        // 150%：尺寸与步长都按缩放换算
        let hidpi = MonitorBounds {
            x: 0,
            y: 0,
            width: 2880,
            height: 1620,
        };
        let plan = reset_plan(hidpi, 1.5, &keys);
        let base = window_logic::reset_window_position(hidpi, 1.5);
        assert_eq!(plan[2].1, PhysRect::new(base.x + 96, base.y + 96, 894, 900));
        assert!(reset_plan(MONITOR, 1., &[]).is_empty());
    }
}
