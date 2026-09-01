//! The game's random number generator.
//!
//! Park–Miller/Lehmer minimal standard generator using Schrage's method
//! (a=16807, m=2^31-1, q=m/a=127773, r=m%a=2836). `rand_number` keeps the
//! generator's modulo bias on purpose — the draw sequence is observable, so
//! a seeded run must reproduce it draw for draw. Do not "fix" the bias.

const A: i64 = 16807;
const M: i64 = 2147483647;
const Q: i64 = 127773;
const R: i64 = 2836;

/// Owned RNG state. There is one process-global seed,
/// the game will own exactly one `CircleRng` to match, while
/// tests can hold seeded instances freely.
#[derive(Debug, Clone)]
pub struct CircleRng {
    state: i64,
}

/// MUD_RNG_TRACE=<path>: log "seed <s>" then "<n> <value>" per draw, so two
/// seeded runs that drift apart can be narrowed to the first draw where
/// they stopped agreeing.
fn trace_sink() -> Option<&'static std::sync::Mutex<std::fs::File>> {
    static SINK: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> =
        std::sync::OnceLock::new();
    SINK.get_or_init(|| {
        std::env::var("MUD_RNG_TRACE")
            .ok()
            .and_then(|p| std::fs::File::create(p).ok())
            .map(std::sync::Mutex::new)
    })
    .as_ref()
}

static TRACE_N: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Interleave a log line into the RNG trace, so a draw can be placed
/// within a boot phase when reading it back.
pub fn rng_trace_note(msg: &str) {
    if let Some(sink) = trace_sink() {
        use std::io::Write;
        let _ = writeln!(sink.lock().unwrap(), "# {}", msg);
    }
}

impl CircleRng {
    /// Equivalent of `circle_srandom(seed)`.
    pub fn new(seed: i64) -> Self {
        if let Some(sink) = trace_sink() {
            use std::io::Write;
            let _ = writeln!(sink.lock().unwrap(), "seed {}", seed);
        }
        Self { state: seed }
    }

    /// Equivalent of `circle_random`: returns the next raw value in
    /// 1..=2146483646 (never 0 for any nonzero seed; a zero seed sticks at
    /// zero).
    pub fn circle_random(&mut self) -> i64 {
        let hi = self.state / Q;
        let lo = self.state % Q;
        let test = A * lo - R * hi;
        self.state = if test > 0 { test } else { test + M };
        if let Some(sink) = trace_sink() {
            use std::io::Write;
            let n = TRACE_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let _ = writeln!(sink.lock().unwrap(), "{} {}", n, self.state);
        }
        self.state
    }

    /// Equivalent of `rand_number(from, to)`: uniform-ish integer in
    /// from..=to, modulo bias included. Reversed arguments are swapped
    /// (the SYSERR arrives with the game layer).
    pub fn rand_number(&mut self, from: i32, to: i32) -> i32 {
        let (from, to) = if from > to { (to, from) } else { (from, to) };
        let span = (to as i64) - (from as i64) + 1;
        (self.circle_random() % span + from as i64) as i32
    }

    /// Equivalent of `dice(num, size)`: sum of `num` rolls of 1..=size;
    /// zero when either argument is non-positive.
    pub fn dice(&mut self, num: i32, size: i32) -> i32 {
        if size <= 0 || num <= 0 {
            return 0;
        }
        let mut sum = 0;
        for _ in 0..num {
            sum += self.rand_number(1, size);
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first three raw draws from seed 1, pinned.
    #[test]
    fn golden_sequence_from_seed_1() {
        let mut rng = CircleRng::new(1);
        assert_eq!(rng.circle_random(), 16807);
        assert_eq!(rng.circle_random(), 282475249);
        assert_eq!(rng.circle_random(), 1622650073);
    }

    #[test]
    fn full_period_bounds_hold() {
        let mut rng = CircleRng::new(12345);
        for _ in 0..100_000 {
            let v = rng.circle_random();
            assert!((1..=M - 1).contains(&v));
        }
    }

    #[test]
    fn rand_number_swaps_reversed_bounds() {
        let mut a = CircleRng::new(99);
        let mut b = CircleRng::new(99);
        assert_eq!(a.rand_number(10, 1), b.rand_number(1, 10));
    }

    #[test]
    fn rand_number_matches_raw_draw_arithmetic() {
        let mut raw = CircleRng::new(777);
        let mut cooked = CircleRng::new(777);
        for _ in 0..1_000 {
            let expect = (raw.circle_random() % 6 + 1) as i32;
            assert_eq!(cooked.rand_number(1, 6), expect);
        }
    }

    #[test]
    fn dice_degenerate_inputs_roll_nothing() {
        let mut rng = CircleRng::new(5);
        let before = rng.clone();
        assert_eq!(rng.dice(0, 6), 0);
        assert_eq!(rng.dice(3, 0), 0);
        assert_eq!(rng.dice(-1, 6), 0);
        // No draws may be consumed by degenerate calls.
        assert_eq!(rng.state, before.state);
    }
}
