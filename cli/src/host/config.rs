use wasmtime::Config;

/* The settings the precompiled artifacts and the runtime engine must agree on. */
pub fn base() -> Config {
    let mut cfg = Config::new();
    cfg.epoch_interruption(true);
    cfg.consume_fuel(false);
    // A full 4 GiB reservation lets Cranelift drop the bounds check on every load and store.
    cfg.memory_reservation(4 << 30);
    cfg.memory_guard_size(32 << 20);
    cfg.memory_init_cow(true);
    cfg
}
