//! Bounded host-clock accounting around command submission and completion.
//! This diagnoses serialized workers without keeping an unbounded event trace.
//! It measures overlapping in-flight dispatches, not simultaneous ALU occupancy:
//! the intervals also include driver submission and waiting for completion.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceDispatchTiming {
    pub dispatches: u64,
    pub first_ms: Option<f64>,
    pub last_ms: Option<f64>,
    pub wall_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DispatchTiming {
    pub cpu: DeviceDispatchTiming,
    pub gpu: DeviceDispatchTiming,
    pub overlap_wall_ms: f64,
}

#[derive(Default)]
struct State {
    active: [u32; 2],
    dispatches: [u64; 2],
    first: [Option<Duration>; 2],
    last: [Option<Duration>; 2],
    wall: [Duration; 2],
    overlap: Duration,
    observed: Duration,
}

impl State {
    fn advance(&mut self, now: Duration) {
        let dt = now.saturating_sub(self.observed);
        for i in 0..2 {
            if self.active[i] > 0 {
                self.wall[i] += dt;
            }
        }
        if self.active.iter().all(|&n| n > 0) {
            self.overlap += dt;
        }
        self.observed = now;
    }

    fn begin(&mut self, device: usize, now: Duration) {
        self.advance(now);
        self.active[device] += 1;
        self.dispatches[device] += 1;
        self.first[device].get_or_insert(now);
    }

    fn end(&mut self, device: usize, now: Duration) {
        self.advance(now);
        self.active[device] -= 1;
        self.last[device] = Some(now);
    }

    fn timing(&self) -> DispatchTiming {
        let ms = |t: Duration| t.as_secs_f64() * 1000.0;
        let device = |i: usize| DeviceDispatchTiming {
            dispatches: self.dispatches[i],
            first_ms: self.first[i].map(ms),
            last_ms: self.last[i].map(ms),
            wall_ms: ms(self.wall[i]),
        };
        DispatchTiming {
            cpu: device(0),
            gpu: device(1),
            overlap_wall_ms: ms(self.overlap),
        }
    }
}

pub(crate) struct ExecutionProfile {
    started: Instant,
    state: Mutex<State>,
}

impl ExecutionProfile {
    pub fn new(started: Instant) -> Self {
        Self {
            started,
            state: Mutex::new(State::default()),
        }
    }

    pub fn enter(&self, cpu: bool) -> DispatchGuard<'_> {
        let device = usize::from(!cpu);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.begin(device, self.started.elapsed());
        DispatchGuard {
            profile: self,
            device,
        }
    }

    pub fn timing(&self) -> DispatchTiming {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.advance(self.started.elapsed());
        state.timing()
    }
}

pub(crate) struct DispatchGuard<'a> {
    profile: &'a ExecutionProfile,
    device: usize,
}

impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.profile.state.lock().unwrap_or_else(|e| e.into_inner());
        state.end(self.device, self.profile.started.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_counts_intersections_not_the_span_between_first_and_last_work() {
        let mut state = State::default();
        let ms = Duration::from_millis;
        state.begin(0, ms(1));
        state.end(0, ms(3));
        state.begin(1, ms(4));
        state.begin(0, ms(5));
        state.end(1, ms(8));
        state.end(0, ms(10));
        let timing = state.timing();
        assert_eq!(timing.cpu.dispatches, 2);
        assert_eq!(timing.gpu.dispatches, 1);
        assert_eq!(timing.cpu.wall_ms, 7.0);
        assert_eq!(timing.gpu.wall_ms, 4.0);
        assert_eq!(timing.overlap_wall_ms, 3.0);
        assert_eq!(timing.cpu.first_ms, Some(1.0));
        assert_eq!(timing.cpu.last_ms, Some(10.0));
    }

    #[test]
    fn nested_same_device_dispatches_are_not_double_counted() {
        let mut state = State::default();
        let ms = Duration::from_millis;
        state.begin(0, ms(1));
        state.begin(0, ms(2));
        state.begin(1, ms(3));
        state.end(0, ms(4));
        state.end(1, ms(5));
        state.end(0, ms(6));
        let timing = state.timing();
        assert_eq!(timing.cpu.wall_ms, 5.0);
        assert_eq!(timing.gpu.wall_ms, 2.0);
        assert_eq!(timing.overlap_wall_ms, 2.0);
    }

    #[test]
    fn guard_records_completion_on_early_return_and_unwind() {
        let profile = ExecutionProfile::new(Instant::now());
        let _ = std::panic::catch_unwind(|| {
            let _guard = profile.enter(true);
            panic!("simulated dispatch failure");
        });
        let timing = profile.timing();
        assert_eq!(timing.cpu.dispatches, 1);
        assert!(timing.cpu.last_ms >= timing.cpu.first_ms);
        assert_eq!(profile.state.lock().unwrap().active, [0, 0]);
        assert_eq!(timing.overlap_wall_ms, 0.0);
    }
}
