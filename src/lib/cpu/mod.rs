use std::sync::Arc;

pub mod mos6502;

pub type InterruptSource = u32;

pub trait InterruptTarget: Send + Sync {
    fn trigger_irq(&self, nmi: bool, source: InterruptSource);
    fn release_irq(&self, nmi: bool, source: InterruptSource);
}

pub struct InterruptConnection {
    target: Arc<dyn InterruptTarget>,
    source: InterruptSource,
}

impl InterruptConnection {
    pub fn new(target: Arc<dyn InterruptTarget>, source: InterruptSource) -> InterruptConnection {
        InterruptConnection { target, source }
    }

    pub fn trigger(&self, nmi: bool) {
        self.target.trigger_irq(nmi, self.source);
    }

    pub fn release(&self, nmi: bool) {
        self.target.release_irq(nmi, self.source);
    }
}
