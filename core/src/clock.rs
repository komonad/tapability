//! Monotonic clock in milliseconds.
//!
//! `std::time::Instant` panics on `wasm32-unknown-unknown` because there is no
//! clock behind it, so the WebAssembly build imports the host clock instead
//! (`worker.js` supplies it as `env.tapa_now_ms`). The native build uses
//! `Instant`, which is monotonic and cheap.

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::sync::OnceLock;
    use std::time::Instant;

    fn origin() -> Instant {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        *ORIGIN.get_or_init(Instant::now)
    }

    pub fn now_ms() -> f64 {
        origin().elapsed().as_secs_f64() * 1000.0
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    // Supplied by the JavaScript loader; see `web/worker.js`.
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn tapa_now_ms() -> f64;
    }

    pub fn now_ms() -> f64 {
        unsafe { tapa_now_ms() }
    }
}

pub use imp::now_ms;

/// Report candidate search progress to the browser host.
#[cfg(target_arch = "wasm32")]
pub fn generation_progress(attempts: usize, qualified: usize) {
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn tapa_generation_progress(attempts: u32, qualified: u32);
    }
    unsafe { tapa_generation_progress(attempts.min(u32::MAX as usize) as u32, qualified as u32) }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn generation_progress(_attempts: usize, _qualified: usize) {}

/// Milliseconds since the clock was started.
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    start_ms: f64,
}

impl Timer {
    pub fn start() -> Timer {
        Timer {
            start_ms: now_ms(),
        }
    }

    pub fn elapsed_ms(&self) -> f64 {
        (now_ms() - self.start_ms).max(0.0)
    }

    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.elapsed_ms() / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_moves_forward() {
        let timer = Timer::start();
        let before = timer.elapsed_ms();
        let mut sum = 0u64;
        for i in 0..200_000u64 {
            sum = sum.wrapping_add(i);
        }
        assert!(sum > 0);
        assert!(timer.elapsed_ms() >= before);
        assert!(now_ms() > 0.0);
    }
}
