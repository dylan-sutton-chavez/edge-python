use wasmtime::Config;

// Linear memory reserved per instance, a pooled instance cannot grow past it.
pub const MEMORY_RESERVATION: u64 = 256 << 20;

/* The settings the precompiled artifacts and the runtime engine must agree on. */
pub fn base() -> Config {
    let mut cfg = Config::new();
    cfg.epoch_interruption(true);
    cfg.consume_fuel(false);
    cfg.memory_reservation(MEMORY_RESERVATION);
    cfg.memory_guard_size(32 << 20);
    cfg.memory_init_cow(true);
    cfg
}
