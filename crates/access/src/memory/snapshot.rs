/// One conversation turn crossing the typed memory Access boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTurn {
    pub role: String,
    pub content: String,
    pub ts: Option<i64>,
}

/// A bounded memory snapshot and the total turn count it represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub turns: Vec<MemoryTurn>,
    pub turn_count: usize,
}
