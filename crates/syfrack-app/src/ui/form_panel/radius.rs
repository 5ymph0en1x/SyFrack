//! The inspector's « Rayon estimé », computed off the UI thread: the
//! estimate evaluates the graph along many rays (0.2 to 0.4 s on the
//! compositions), so a worker runs it, at most one at a time and at most
//! one start per `RADIUS_PERIOD_S`; the inspector shows « … » meanwhile.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use syfrack_core::Graph;

use super::RADIUS_PERIOD_S;

/// The form as the estimate sees it: card positions and names cleared.
fn form_of(g: &Graph) -> Graph {
    let mut g = g.clone();
    for n in &mut g.nodes {
        n.pos = [0.0; 2];
        n.name.clear();
    }
    g
}

#[derive(Default)]
pub struct RadiusEstimate {
    /// The last finished estimate: of which form, value.
    done: Option<(Graph, f32)>,
    /// The estimate under way: of which form, its result.
    running: Option<(Graph, Receiver<f32>)>,
    /// When the last worker started (UI time, s).
    started: Option<f64>,
}

impl RadiusEstimate {
    /// The estimated radius of `g` when known, `None` while it is computed.
    /// Never blocks: a stale form starts a worker (when none runs and the
    /// last one started `RADIUS_PERIOD_S` ago or more).
    pub fn poll(&mut self, g: &Graph, now: f64) -> Option<f32> {
        if let Some((form, rx)) = &self.running {
            match rx.try_recv() {
                Ok(r) => {
                    self.done = Some((form.clone(), r));
                    self.running = None;
                }
                Err(TryRecvError::Disconnected) => self.running = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let form = form_of(g);
        if let Some((f, r)) = &self.done {
            if *f == form {
                return Some(*r);
            }
        }
        let due = self.started.is_none_or(|t| now - t >= RADIUS_PERIOD_S);
        if self.running.is_none() && due {
            let (tx, rx) = mpsc::channel();
            let job = form.clone();
            let spawned = std::thread::Builder::new()
                .name("syfrack-radius".into())
                .spawn(move || {
                    let _ = tx.send(job.estimate_bounding_radius());
                });
            if spawned.is_ok() {
                self.running = Some((form, rx));
                self.started = Some(now);
            }
        }
        None
    }

    /// A worker is computing an estimate.
    #[cfg(test)]
    fn pending(&self) -> bool {
        self.running.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(est: &mut RadiusEstimate, g: &Graph, now: f64) -> f32 {
        let t0 = Instant::now();
        loop {
            if let Some(r) = est.poll(g, now) {
                return r;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "estimate never came"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_estimate_runs_off_the_ui_thread_and_is_cached_per_form() {
        let g = Graph::bundled("cathedrale").unwrap();
        let mut est = RadiusEstimate::default();
        let t0 = Instant::now();
        assert_eq!(est.poll(&g, 0.0), None, "pending: « … »");
        assert!(
            t0.elapsed() < Duration::from_millis(16),
            "the UI thread never runs the estimate ({:?})",
            t0.elapsed()
        );
        assert!(est.pending());
        let r = wait(&mut est, &g, 0.1);
        assert_eq!(r, g.estimate_bounding_radius());
        // A card moved or renamed: same form, served from the cache.
        let mut moved = g.clone();
        moved.nodes[1].pos = [500.0, 500.0];
        moved.nodes[1].name = "ailleurs".into();
        assert_eq!(est.poll(&moved, 0.2), Some(r));
        assert!(!est.pending());
        // Another form: pending, and not restarted within the period.
        let other = Graph::bundled("sphere").unwrap();
        assert_eq!(est.poll(&other, 0.5), None);
        assert!(!est.pending(), "at most one start per period");
        assert_eq!(est.poll(&other, 1.2), None);
        assert!(est.pending());
        let r2 = wait(&mut est, &other, 1.3);
        assert_eq!(r2, other.estimate_bounding_radius());
    }
}
