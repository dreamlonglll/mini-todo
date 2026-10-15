//! 收纳箱窗口用到的纯函数（旧 `pc/src/utils/fence.ts` 的 1:1 移植，测试向量照抄 `fence.test.ts` / 规格 §5）。

use std::time::Instant;

use mini_todo_core::fence::items::{ItemOpResult, ItemOpStatus};
use mini_todo_core::fence::{
    DeleteFenceResult, MAX_ICON_SIZE, MAX_TITLE_CHARS, MIN_ICON_SIZE, PxRect,
};

use crate::ui::message::MessageKind;

/// 图标尺寸档位（与 `fence_fs::ICON_SIZE_BUCKETS` 一致）
pub const ICON_BUCKETS: [u32; 8] = [16, 24, 32, 48, 64, 96, 128, 256];
/// Ctrl+滚轮步长
pub const ICON_SIZE_STEP: i32 = 8;

/// 时间常量（毫秒 / 时长）
pub mod timing {
    use std::time::Duration;
    /// 折叠态鼠标进入后多久临时展开
    pub const HOVER_EXPAND_DELAY: Duration = Duration::from_millis(150);
    /// 鼠标离开后多久收回
    pub const HOVER_COLLAPSE_DELAY: Duration = Duration::from_millis(400);
    /// 拖文件离开 / 放下后多久收回
    pub const DRAG_COLLAPSE_DELAY: Duration = Duration::from_millis(300);
    /// 拖动闸门续期窗口
    pub const GEOMETRY_ARM_MS: f64 = 3000.;
    /// 拖动停下多久后落库
    pub const GEOMETRY_COMMIT_DEBOUNCE: Duration = Duration::from_millis(300);
    /// Ctrl+滚轮调图标大小后的落库防抖
    pub const ICON_SIZE_SAVE_DELAY: Duration = Duration::from_millis(400);
    /// 删除中止提示的显示时长
    pub const DELETE_KEPT_MESSAGE_DURATION: Duration = Duration::from_millis(8000);
    /// 拖入结果提示时长
    pub const MOVE_IN_MESSAGE_DURATION: Duration = Duration::from_millis(5000);
    /// 触控板一次手势连发很多滚轮事件：节流
    pub const WHEEL_STEP_INTERVAL: Duration = Duration::from_millis(40);
    /// 同时在途的图标请求数（后端图标在单个 STA 线程排队）
    pub const MAX_ICON_REQUESTS: usize = 3;
    /// 可见区域上下外扩（像素），滚动时提前加载
    pub const ICON_PRELOAD_MARGIN: f32 = 64.;
}

/// 进程内单调毫秒时钟（几何闸门用）
pub fn now_ms() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}

/// 与后端 `normalize_title` 同规则：控制字符（U+0000–U+001F、U+007F–U+009F）换成空格、去首尾空白；
/// 为空或超过 40 个字符返回 `None`（界面静默不提交）
pub fn normalize_fence_title(raw: &str) -> Option<String> {
    let replaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let title = replaced.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        None
    } else {
        Some(title.to_string())
    }
}

/// 按下后移动超过阈值（逻辑像素，默认 3）才算拖动
pub fn exceeds_drag_threshold(dx: f64, dy: f64, threshold: f64) -> bool {
    dx.abs() > threshold || dy.abs() > threshold
}

/// 判定"用户拖动 / 缩放结束"的闸门（时间单位 ms）：用户开始拖动时 `arm`，之后的移动 / 缩放事件在闸门
/// 打开期间才算数（每次续期），停下后由调用方防抖落库；系统挪窗、程序调整一律忽略
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryCommitGate {
    window_ms: f64,
    armed_until: f64,
}

impl GeometryCommitGate {
    pub fn new(window_ms: f64) -> Self {
        Self {
            window_ms,
            armed_until: f64::NEG_INFINITY,
        }
    }

    pub fn arm(&mut self, now: f64) {
        self.armed_until = now + self.window_ms;
    }

    /// 收到一次移动 / 缩放事件：闸门开着就续期并返回 true（应安排落库），否则 false
    pub fn accept(&mut self, now: f64) -> bool {
        if now > self.armed_until {
            return false;
        }
        self.armed_until = now + self.window_ms;
        true
    }

    pub fn is_armed(&self, now: f64) -> bool {
        now <= self.armed_until
    }

    pub fn disarm(&mut self) {
        self.armed_until = f64::NEG_INFINITY;
    }
}

/// 缩放方向（8 个手柄）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeDir {
    N,
    S,
    W,
    E,
    NW,
    NE,
    SW,
    SE,
}

impl ResizeDir {
    pub const ALL: [ResizeDir; 8] = [
        ResizeDir::N,
        ResizeDir::S,
        ResizeDir::W,
        ResizeDir::E,
        ResizeDir::NW,
        ResizeDir::NE,
        ResizeDir::SW,
        ResizeDir::SE,
    ];

    fn has_n(self) -> bool {
        matches!(self, ResizeDir::N | ResizeDir::NW | ResizeDir::NE)
    }
    fn has_s(self) -> bool {
        matches!(self, ResizeDir::S | ResizeDir::SW | ResizeDir::SE)
    }
    fn has_w(self) -> bool {
        matches!(self, ResizeDir::W | ResizeDir::NW | ResizeDir::SW)
    }
    fn has_e(self) -> bool {
        matches!(self, ResizeDir::E | ResizeDir::NE | ResizeDir::SE)
    }

    pub fn key(self) -> &'static str {
        match self {
            ResizeDir::N => "n",
            ResizeDir::S => "s",
            ResizeDir::W => "w",
            ResizeDir::E => "e",
            ResizeDir::NW => "nw",
            ResizeDir::NE => "ne",
            ResizeDir::SW => "sw",
            ResizeDir::SE => "se",
        }
    }
}

/// JS `Math.round`：逢 .5 向 +∞
pub fn js_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

/// 拖哪条边改哪条边，对边不动；不小于最小尺寸（拖左 / 上边缩到最小就停，右下角不被推走）
pub fn resize_rect(
    start: PxRect,
    dir: ResizeDir,
    dx: f64,
    dy: f64,
    min_w: f64,
    min_h: f64,
) -> PxRect {
    let (mut x, mut y) = (start.x as f64, start.y as f64);
    let (mut w, mut h) = (start.width as f64, start.height as f64);
    if dir.has_e() {
        w = (start.width as f64 + dx).max(min_w);
    }
    if dir.has_s() {
        h = (start.height as f64 + dy).max(min_h);
    }
    if dir.has_w() {
        w = (start.width as f64 - dx).max(min_w);
        x = start.x as f64 + start.width as f64 - w;
    }
    if dir.has_n() {
        h = (start.height as f64 - dy).max(min_h);
        y = start.y as f64 + start.height as f64 - h;
    }
    PxRect {
        x: js_round(x),
        y: js_round(y),
        width: js_round(w),
        height: js_round(h),
    }
}

/// 向上取整到图标档位
pub fn icon_bucket(size_px: u32) -> u32 {
    ICON_BUCKETS
        .iter()
        .copied()
        .find(|b| *b >= size_px)
        .unwrap_or(256)
}

/// 请求的物理像素尺寸：`ceil(iconSize × dpr)`
pub fn icon_request_px(icon_size: f64, dpr: f64) -> u32 {
    let dpr = if dpr > 0. { dpr } else { 1. };
    (icon_size * dpr).ceil() as u32
}

pub fn clamp_icon_size(size: f64) -> i32 {
    js_round(size).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE)
}

/// 上滚放大一档、下滚缩小一档
pub fn next_icon_size(current: i32, delta_y: f64) -> i32 {
    if delta_y == 0. {
        return clamp_icon_size(current as f64);
    }
    let step = if delta_y < 0. {
        ICON_SIZE_STEP
    } else {
        -ICON_SIZE_STEP
    };
    clamp_icon_size((current + step) as f64)
}

/// 路径最后一段（`\` 与 `/` 都认；先去掉末尾分隔符）
pub fn base_name(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) {
        Some(i) => t[i + 1..].to_string(),
        None => t.to_string(),
    }
}

/// 上级目录（分隔符位置 > 0 才截断，否则原样返回去尾分隔符后的字符串）
pub fn parent_dir(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) {
        Some(i) if i > 0 => t[..i].to_string(),
        _ => t.to_string(),
    }
}

/// 逐项结果汇总
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpSummary {
    pub moved: usize,
    pub cancelled: usize,
    pub failed: Vec<(String, String)>,
    pub notes: Vec<(String, String)>,
}

pub fn summarize_op_results(results: &[ItemOpResult]) -> OpSummary {
    let mut summary = OpSummary::default();
    for result in results {
        let name = base_name(&result.source);
        let error = result.error.clone().filter(|e| !e.is_empty());
        match result.status {
            ItemOpStatus::Moved => {
                summary.moved += 1;
                if let Some(note) = error {
                    summary.notes.push((name, note));
                }
            }
            ItemOpStatus::Cancelled => summary.cancelled += 1,
            ItemOpStatus::Failed => summary
                .failed
                .push((name, error.unwrap_or_else(|| "未知原因".into()))),
            ItemOpStatus::Skipped => {}
        }
    }
    summary
}

const MAX_LISTED_ITEMS: usize = 2;

/// 拖入结果提示；`None` = 不提示
pub fn describe_move_in(s: &OpSummary) -> Option<(MessageKind, String)> {
    let mut parts: Vec<String> = Vec::new();
    if !s.failed.is_empty() {
        let listed = s.failed[..s.failed.len().min(MAX_LISTED_ITEMS)]
            .iter()
            .map(|(n, r)| format!("{n}（{r}）"))
            .collect::<Vec<_>>()
            .join("、");
        let more = if s.failed.len() > MAX_LISTED_ITEMS {
            " 等"
        } else {
            ""
        };
        parts.push(format!("{} 项未能收纳：{listed}{more}", s.failed.len()));
    }
    if s.cancelled > 0 {
        parts.push(format!("已取消 {} 项", s.cancelled));
    }
    if let Some((n, note)) = s.notes.first() {
        parts.push(format!("{n}：{note}"));
    }
    if parts.is_empty() {
        return None;
    }
    if s.moved > 0 {
        parts.insert(0, format!("已收纳 {} 项", s.moved));
    }
    let kind = if !s.failed.is_empty() {
        if s.moved > 0 {
            MessageKind::Warning
        } else {
            MessageKind::Error
        }
    } else if s.notes.is_empty() {
        MessageKind::Info
    } else {
        MessageKind::Warning
    };
    Some((kind, parts.join("；")))
}

/// 删除中止（收纳箱保留）提示
pub fn describe_delete_kept(r: &DeleteFenceResult) -> String {
    let reasons: std::collections::HashMap<String, String> = r
        .failures
        .iter()
        .map(|f| {
            (
                base_name(&f.source).to_lowercase(),
                f.error.clone().unwrap_or_default(),
            )
        })
        .collect();
    let names: Vec<String> = if !r.remaining.is_empty() {
        r.remaining.clone()
    } else {
        r.failures.iter().map(|f| base_name(&f.source)).collect()
    };
    if names.is_empty() {
        let restored = if r.restored > 0 {
            format!("{} 个项目已移回桌面，但", r.restored)
        } else {
            String::new()
        };
        return format!("{restored}收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除");
    }
    let listed = names[..names.len().min(MAX_LISTED_ITEMS)]
        .iter()
        .map(|n| match reasons.get(&n.to_lowercase()) {
            Some(reason) if !reason.is_empty() => format!("{n}（{reason}）"),
            _ => n.clone(),
        })
        .collect::<Vec<_>>()
        .join("、");
    let more = if names.len() > MAX_LISTED_ITEMS {
        " 等"
    } else {
        ""
    };
    let head = format!("{} 个项目未能移回桌面，收纳箱已保留", names.len());
    if listed.is_empty() {
        head
    } else {
        format!("{head}：{listed}{more}")
    }
}

/// 删除确认面板的说明文字
pub fn delete_hint(count: Option<usize>) -> String {
    match count {
        None => "正在统计其中的项目…".into(),
        Some(0) => "收纳箱是空的，收纳文件夹会一并删除".into(),
        Some(n) => format!("其中的 {n} 个项目会移回桌面"),
    }
}

/// 系统取色对话框的结果 → `#RRGGBB`（大写，与 core `fence::normalize_color` 一致）
pub fn color_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(source: &str, status: ItemOpStatus, error: Option<&str>) -> ItemOpResult {
        ItemOpResult {
            source: source.into(),
            status,
            file_name: None,
            dest_path: None,
            error: error.map(Into::into),
        }
    }

    #[test]
    fn titles_follow_backend_rules() {
        assert_eq!(normalize_fence_title("  工作  ").as_deref(), Some("工作"));
        assert_eq!(normalize_fence_title("a\nb\tc").as_deref(), Some("a b c"));
        assert_eq!(normalize_fence_title("   "), None);
        assert_eq!(normalize_fence_title("\n\t"), None);
        assert!(normalize_fence_title(&"字".repeat(40)).is_some());
        assert_eq!(normalize_fence_title(&"字".repeat(41)), None);
    }

    #[test]
    fn drag_threshold() {
        assert!(!exceeds_drag_threshold(0., 0., 3.));
        assert!(!exceeds_drag_threshold(3., -3., 3.));
        assert!(exceeds_drag_threshold(4., 0., 3.));
        assert!(exceeds_drag_threshold(0., -4., 3.));
    }

    #[test]
    fn geometry_gate_vectors() {
        let mut gate = GeometryCommitGate::new(1000.);
        assert!(!gate.accept(0.));
        assert!(!gate.is_armed(0.));
        gate.arm(0.);
        assert!(gate.accept(900.));
        assert!(gate.accept(1800.));
        assert!(gate.is_armed(2700.));
        assert!(!gate.accept(2900.));
        let mut gate = GeometryCommitGate::new(1000.);
        gate.arm(0.);
        assert!(!gate.accept(1001.));
        let mut gate = GeometryCommitGate::new(1000.);
        gate.arm(2000.);
        gate.disarm();
        assert!(!gate.accept(2001.));
    }

    #[test]
    fn resize_vectors() {
        let start = PxRect {
            x: 100,
            y: 200,
            width: 552,
            height: 504,
        };
        let r = |dir, dx, dy| {
            let r = resize_rect(start, dir, dx, dy, 240., 240.);
            (r.x, r.y, r.width, r.height)
        };
        assert_eq!(r(ResizeDir::SE, 30., 40.), (100, 200, 582, 544));
        assert_eq!(r(ResizeDir::E, 30., 40.), (100, 200, 582, 504));
        assert_eq!(r(ResizeDir::S, 30., 40.), (100, 200, 552, 544));
        assert_eq!(r(ResizeDir::NW, -20., -10.), (80, 190, 572, 514));
        assert_eq!(r(ResizeDir::NE, 20., 10.), (100, 210, 572, 494));
        assert_eq!(r(ResizeDir::SW, 20., 10.), (120, 200, 532, 514));
        assert_eq!(r(ResizeDir::SE, -1000., -1000.), (100, 200, 240, 240));
        assert_eq!(r(ResizeDir::NW, 1000., 1000.), (412, 464, 240, 240));
        assert_eq!(r(ResizeDir::E, 10.4, 0.).2, 562);
    }

    #[test]
    fn icon_size_vectors() {
        assert_eq!(icon_bucket(0), 16);
        assert_eq!(icon_bucket(48), 48);
        assert_eq!(icon_bucket(49), 64);
        assert_eq!(icon_bucket(72), 96);
        assert_eq!(icon_bucket(1000), 256);
        assert_eq!(icon_request_px(48., 1.5), 72);
        assert_eq!(icon_request_px(48., 1.25), 60);
        assert_eq!(icon_request_px(40., 1.75), 70);
        assert_eq!(icon_request_px(48., 0.), 48);
        assert_eq!(next_icon_size(48, -100.), 56);
        assert_eq!(next_icon_size(48, 100.), 40);
        assert_eq!(next_icon_size(48, 0.), 48);
        assert_eq!(next_icon_size(128, -100.), 128);
        assert_eq!(next_icon_size(24, 100.), 24);
        assert_eq!(clamp_icon_size(1000.), 128);
        assert_eq!(clamp_icon_size(-5.), 24);
    }

    #[test]
    fn path_helpers() {
        assert_eq!(base_name(r"C:\Users\me\Desktop\Chrome.lnk"), "Chrome.lnk");
        assert_eq!(base_name(r"C:\Users\me\Desktop\文件夹\"), "文件夹");
        assert_eq!(base_name("/home/me/a.txt"), "a.txt");
        assert_eq!(base_name("a.txt"), "a.txt");
        assert_eq!(
            parent_dir(r"C:\Users\me\Desktop\Chrome.lnk"),
            r"C:\Users\me\Desktop"
        );
        assert_eq!(parent_dir("/home/me/a.txt"), "/home/me");
    }

    #[test]
    fn move_in_messages() {
        let s = summarize_op_results(&[
            op(r"C:\d\a.lnk", ItemOpStatus::Moved, None),
            op(r"C:\d\desktop.ini", ItemOpStatus::Skipped, Some("已跳过")),
        ]);
        assert_eq!(
            s,
            OpSummary {
                moved: 1,
                ..Default::default()
            }
        );
        assert_eq!(describe_move_in(&s), None);

        let s = summarize_op_results(&[op(r"C:\d\a.lnk", ItemOpStatus::Cancelled, None)]);
        assert_eq!(
            describe_move_in(&s),
            Some((MessageKind::Info, "已取消 1 项".into()))
        );

        let s = summarize_op_results(&[
            op(
                r"C:\d\a.lnk",
                ItemOpStatus::Failed,
                Some("正被其他程序占用"),
            ),
            op(r"C:\d\b.lnk", ItemOpStatus::Failed, Some("不存在")),
            op(r"C:\d\c.lnk", ItemOpStatus::Failed, None),
        ]);
        assert_eq!(
            describe_move_in(&s),
            Some((
                MessageKind::Error,
                "3 项未能收纳：a.lnk（正被其他程序占用）、b.lnk（不存在） 等".into()
            ))
        );

        let s = summarize_op_results(&[
            op(r"C:\d\a.lnk", ItemOpStatus::Moved, None),
            op(r"C:\d\b.lnk", ItemOpStatus::Cancelled, None),
            op(r"C:\d\c.lnk", ItemOpStatus::Failed, Some("没有权限")),
        ]);
        assert_eq!(
            describe_move_in(&s),
            Some((
                MessageKind::Warning,
                "已收纳 1 项；1 项未能收纳：c.lnk（没有权限）；已取消 1 项".into()
            ))
        );

        let s = summarize_op_results(&[op(
            r"C:\d\a.lnk",
            ItemOpStatus::Moved,
            Some("源文件未能删除"),
        )]);
        assert_eq!(
            describe_move_in(&s),
            Some((
                MessageKind::Warning,
                "已收纳 1 项；a.lnk：源文件未能删除".into()
            ))
        );
    }

    #[test]
    fn delete_kept_messages() {
        let r = DeleteFenceResult {
            deleted: false,
            restored: 1,
            remaining: vec!["b.txt".into(), "C.lnk".into(), "hidden.tmp".into()],
            failures: vec![
                op(
                    r"C:\f\b.txt",
                    ItemOpStatus::Failed,
                    Some("正被其他程序占用"),
                ),
                op(
                    r"C:\f\c.lnk",
                    ItemOpStatus::Cancelled,
                    Some("用户取消了操作"),
                ),
            ],
        };
        assert_eq!(
            describe_delete_kept(&r),
            "3 个项目未能移回桌面，收纳箱已保留：b.txt（正被其他程序占用）、C.lnk（用户取消了操作） 等"
        );
        let r = DeleteFenceResult {
            deleted: false,
            restored: 0,
            remaining: vec![],
            failures: vec![op(r"C:\f\a.lnk", ItemOpStatus::Failed, Some("没有权限"))],
        };
        assert_eq!(
            describe_delete_kept(&r),
            "1 个项目未能移回桌面，收纳箱已保留：a.lnk（没有权限）"
        );
        let r = DeleteFenceResult {
            deleted: false,
            restored: 3,
            remaining: vec![],
            failures: vec![],
        };
        assert_eq!(
            describe_delete_kept(&r),
            "3 个项目已移回桌面，但收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除"
        );
        let r = DeleteFenceResult {
            deleted: false,
            restored: 0,
            remaining: vec![],
            failures: vec![],
        };
        assert_eq!(
            describe_delete_kept(&r),
            "收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除"
        );
    }

    #[test]
    fn color_hex_is_upper_case() {
        assert_eq!(color_hex(0x1E, 0x29, 0x3B), "#1E293B");
        assert_eq!(color_hex(0, 0, 0), "#000000");
    }

    #[test]
    fn delete_hint_text() {
        assert_eq!(delete_hint(None), "正在统计其中的项目…");
        assert_eq!(delete_hint(Some(0)), "收纳箱是空的，收纳文件夹会一并删除");
        assert_eq!(delete_hint(Some(6)), "其中的 6 个项目会移回桌面");
    }
}
