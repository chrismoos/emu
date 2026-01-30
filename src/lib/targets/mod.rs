
pub mod appleii;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Stopped,
    Running,
}
