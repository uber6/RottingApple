use std::time::{Duration, Instant};

/// Frame pacing to maintain target FPS.
pub struct FramePacer {
    frame_duration: Duration,
    next_frame: Instant,
}

impl FramePacer {
    pub fn new(fps: u32) -> Self {
        let frame_duration = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        Self {
            frame_duration,
            next_frame: Instant::now(),
        }
    }

    pub async fn wait(&mut self) {
        let now = Instant::now();
        if self.next_frame > now {
            tokio::time::sleep(self.next_frame - now).await;
        }
        self.next_frame += self.frame_duration;
        if self.next_frame < Instant::now() {
            self.next_frame = Instant::now();
        }
    }
}
