use std::{
    ops::{Add, Sub},
    time::Duration,
};

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Instant {
    ts: u128,
}

impl Instant {
    pub fn now() -> Instant {
        {
            Instant {
                ts: (web_sys::window()
                    .expect("should have a Window")
                    .performance()
                    .expect("should have a Performance")
                    .now()
                    * 1000.0) as u128,
            }
        }
    }

    pub fn duration_since(&self, other: Instant) -> Duration {
        Duration::from_micros((self.ts - other.ts) as u64)
    }
}

impl Sub for Instant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration::from_micros((self.ts - rhs.ts) as u64)
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Self::Output {
        Instant {
            ts: self.ts + rhs.as_micros(),
        }
    }
}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.ts.partial_cmp(&other.ts)
    }
}

pub async fn sleep(duration: std::time::Duration) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::utils::futures::spawn(async move {
        gloo_timers::future::TimeoutFuture::new(duration.as_millis() as u32).await;
        tx.send(true).await.unwrap();
    });
    let _ = rx.recv().await;
}
