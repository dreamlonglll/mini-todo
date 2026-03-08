/// 子任务调度状态
#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleStatus {
    /// 初始状态 - 未纳入调度
    None,
    /// 已就绪 - 等待调度器拾取
    Pending,
    /// 排队中 - 在优先级队列中
    Queued,
    /// 执行中 - Agent 正在执行
    Running,
    /// 待审核 - 执行完成，等待人工审核确认
    Reviewing,
    /// 已完成
    Completed,
    /// 失败（超时/错误）
    Failed,
    /// 已取消
    Cancelled,
}

impl ScheduleStatus {
    pub fn as_str(&self) -> &str {
        match self {
            ScheduleStatus::None => "none",
            ScheduleStatus::Pending => "pending",
            ScheduleStatus::Queued => "queued",
            ScheduleStatus::Running => "running",
            ScheduleStatus::Reviewing => "reviewing",
            ScheduleStatus::Completed => "completed",
            ScheduleStatus::Failed => "failed",
            ScheduleStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => ScheduleStatus::Pending,
            "queued" => ScheduleStatus::Queued,
            "running" => ScheduleStatus::Running,
            "reviewing" => ScheduleStatus::Reviewing,
            "completed" => ScheduleStatus::Completed,
            "failed" => ScheduleStatus::Failed,
            "cancelled" => ScheduleStatus::Cancelled,
            _ => ScheduleStatus::None,
        }
    }
}

/// 合法状态转移规则
pub fn can_transition(from: &ScheduleStatus, to: &ScheduleStatus) -> bool {
    matches!(
        (from, to),
        // none -> pending: 任务被提交到调度
        (ScheduleStatus::None, ScheduleStatus::Pending) |
        // pending -> none: 撤回提交，回到初始
        (ScheduleStatus::Pending, ScheduleStatus::None) |
        // pending -> queued: 调度器拾取到队列
        (ScheduleStatus::Pending, ScheduleStatus::Queued) |
        // pending -> cancelled: 取消未排队的任务
        (ScheduleStatus::Pending, ScheduleStatus::Cancelled) |
        // queued -> running: 开始执行
        (ScheduleStatus::Queued, ScheduleStatus::Running) |
        // queued -> cancelled: 取消排队中的任务
        (ScheduleStatus::Queued, ScheduleStatus::Cancelled) |
        // running -> completed: 执行成功
        (ScheduleStatus::Running, ScheduleStatus::Completed) |
        // running -> reviewing: 执行完成，进入人工审核
        (ScheduleStatus::Running, ScheduleStatus::Reviewing) |
        // running -> failed: 执行失败/超时
        (ScheduleStatus::Running, ScheduleStatus::Failed) |
        // running -> cancelled: 用户取消执行中的任务
        (ScheduleStatus::Running, ScheduleStatus::Cancelled) |
        // reviewing -> completed: 审核通过
        (ScheduleStatus::Reviewing, ScheduleStatus::Completed) |
        // reviewing -> failed: 审核拒绝
        (ScheduleStatus::Reviewing, ScheduleStatus::Failed) |
        // reviewing -> pending: 审核拒绝并要求重新执行
        (ScheduleStatus::Reviewing, ScheduleStatus::Pending) |
        // reviewing -> cancelled: 取消审核
        (ScheduleStatus::Reviewing, ScheduleStatus::Cancelled) |
        // failed -> pending: 重试 (重新排入)
        (ScheduleStatus::Failed, ScheduleStatus::Pending) |
        // cancelled -> pending: 重新提交已取消的任务
        (ScheduleStatus::Cancelled, ScheduleStatus::Pending) |
        // completed -> pending: 再次执行已完成的任务
        (ScheduleStatus::Completed, ScheduleStatus::Pending) |
        // failed -> none: 放弃重试，回到初始
        (ScheduleStatus::Failed, ScheduleStatus::None) |
        // cancelled -> none: 回到初始
        (ScheduleStatus::Cancelled, ScheduleStatus::None) |
        // completed -> none: 重置
        (ScheduleStatus::Completed, ScheduleStatus::None)
    )
}

/// 尝试状态转移，返回转移是否成功
pub fn try_transition(
    current: &str,
    target: &str,
) -> Result<String, String> {
    let from = ScheduleStatus::from_str(current);
    let to = ScheduleStatus::from_str(target);

    if can_transition(&from, &to) {
        Ok(to.as_str().to_string())
    } else {
        Err(format!(
            "非法状态转移: {} -> {}",
            current, target
        ))
    }
}
