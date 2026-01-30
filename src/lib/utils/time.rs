use std::time::Duration;

pub type Instant = std::time::Instant;

pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await
}
