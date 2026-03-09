use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct QueuedTask {
    pub subtask_id: i64,
    pub todo_id: i64,
    pub priority: i64,
    pub enqueued_at: u64,
}

impl Ord for QueuedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.enqueued_at.cmp(&self.enqueued_at))
    }
}

impl PartialOrd for QueuedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct PriorityQueue {
    heap: BinaryHeap<QueuedTask>,
    max_size: usize,
}

impl PriorityQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            heap: BinaryHeap::new(),
            max_size,
        }
    }

    pub fn enqueue(&mut self, task: QueuedTask) -> Result<(), String> {
        if self.heap.len() >= self.max_size {
            return Err("队列已满".to_string());
        }
        self.heap.push(task);
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<QueuedTask> {
        self.heap.pop()
    }

    pub fn peek(&self) -> Option<&QueuedTask> {
        self.heap.peek()
    }

    pub fn contains(&self, subtask_id: i64) -> bool {
        self.heap.iter().any(|t| t.subtask_id == subtask_id)
    }

    pub fn get_all(&self) -> Vec<&QueuedTask> {
        self.heap.iter().collect()
    }
}

/// 基于四象限和子任务属性计算优先级
pub fn calculate_priority(
    quadrant: &str,
    priority_score: i64,
    created_at_minutes_ago: i64,
    hours_until_due: Option<i64>,
) -> i64 {
    let mut score: i64 = 0;

    score += match quadrant {
        "important_urgent" => 30,
        "important_not_urgent" => 20,
        "urgent_not_important" => 15,
        _ => 10,
    } * 3;

    let wait_bonus = (created_at_minutes_ago / 10).min(50);
    score += wait_bonus;

    if let Some(hours) = hours_until_due {
        score += match hours {
            h if h <= 1 => 50,
            h if h <= 4 => 30,
            h if h <= 24 => 15,
            h if h <= 72 => 5,
            _ => 0,
        };
    }

    score += priority_score;

    score
}
