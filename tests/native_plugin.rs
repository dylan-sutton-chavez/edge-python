use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;

use compiler::native::{boot_vm, drive, parse_source, RunOpts};
use compiler::vm::Limits;

// Builds the reference plugin as a native cdylib and returns the artifact path.
fn build_slugify() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["build", "-p", "slugify-mod", "--message-format=json"]);
    // macOS leaves the host edge_* symbols undefined so dlopen resolves them at load.
    if cfg!(target_os = "macos") {
        cmd.env("RUSTFLAGS", "-C link-arg=-undefined -C link-arg=dynamic_lookup");
    }
    let out = cmd.output().expect("spawn cargo");
    assert!(out.status.success(), "cargo build -p slugify-mod failed\n{}", String::from_utf8_lossy(&out.stderr));
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["reason"] != "compiler-artifact" {
            continue;
        }
        for f in v["filenames"].as_array().into_iter().flatten() {
            let p = PathBuf::from(f.as_str().unwrap_or_default());
            let is_lib = matches!(p.extension().and_then(|e| e.to_str()), Some("dylib" | "so"));
            if is_lib && p.to_string_lossy().contains("slugify") {
                return p;
            }
        }
    }
    panic!("no cdylib artifact for slugify-mod");
}

thread_local! {
    static PRINTED: RefCell<String> = const { RefCell::new(String::new()) };
}

fn collect(s: &str) {
    PRINTED.with(|o| o.borrow_mut().push_str(s));
}

// The whole production chain, script imports the real plugin, resolver and loader run it.
#[test]
fn a_real_plugin_runs_through_the_full_native_chain() {
    let so = build_slugify();
    let dir = std::env::temp_dir().join("edge_proxy_fullchain");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("packages.json");
    std::fs::write(&manifest, format!("{{ \"imports\": {{ \"slugify_mod\": \"{}\" }} }}", so.display())).unwrap();

    let src = "from slugify_mod import Slugger\ns = Slugger()\ns.add(\"Hello World\")\nprint(s.build())\n";
    let dir_str = format!("{}/", dir.display());
    let chunk = parse_source(src, &dir_str, Some(&manifest.to_string_lossy())).expect("parse and load the plugin");

    let mut vm = boot_vm(chunk, Limits::sandbox(), 0);
    vm.print_hook = Some(collect);
    PRINTED.with(|o| o.borrow_mut().clear());
    let code = drive(&mut vm, src, None, &RunOpts::default());
    assert_eq!(code, 0, "the full native chain should run the plugin without error");
    let out = PRINTED.with(|o| o.borrow().clone());
    assert!(out.contains("hello-world"), "the plugin should slugify its input, got {out:?}");
}

// A plugin with a raw reachable syscall, the jail must gate it, proven on real compiled code.
mod gating {
    use super::*;

    type PluginFn = unsafe extern "C" fn(*const u32, u32, *mut u32) -> i32;

    const SRC: &str = r#"
#[no_mangle]
pub extern "C" fn __fn_pid(_a: *const u32, _c: u32, _o: *mut u32) -> i32 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let r: i64;
        core::arch::asm!("mov eax, 39", "syscall", lateout("rax") r, out("rcx") _, out("r11") _);
        return r as i32;
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let r: i64;
        core::arch::asm!("mov x16, #20", "svc #0x80", lateout("x0") r, out("x16") _);
        return r as i32;
    }
}
"#;

    fn build_pid() -> PathBuf {
        let dir = std::env::temp_dir().join("edge_proxy_gating");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("plugin.rs");
        std::fs::write(&src, SRC).unwrap();
        let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        let so = dir.join(format!("libplugin.{ext}"));
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let ok = Command::new(rustc)
            .args(["--edition", "2021", "--crate-type", "cdylib", "-O", "-o"])
            .arg(&so)
            .arg(&src)
            .status()
            .expect("spawn rustc");
        assert!(ok.success(), "rustc failed to build the syscall fixture");
        so
    }

    #[test]
    fn run_plugin_traps_a_real_reachable_syscall() {
        let so = build_pid();
        let lib = unsafe { libloading::Library::new(&so) }.expect("dlopen");
        let sym: libloading::Symbol<PluginFn> = unsafe { lib.get(b"__fn_pid\0") }.expect("__fn_pid");
        let f: PluginFn = *sym;
        // A leaf fits in the bytes past its entry, so neutralizing that span reaches its syscall.
        let span = 64;
        let n = unsafe { proxy::neutralize(f as *mut u8, span) };
        assert!(n >= 1, "the reachable syscall instruction must be neutralized");
        proxy::register_range(f as usize, f as usize + span);
        let _ = proxy::take_block();
        let args = [0u32];
        let mut out = 0u32;
        let _ = unsafe { proxy::run_plugin(f, args.as_ptr(), 1, &mut out) };
        assert!(proxy::take_block(), "the reachable syscall must be trapped, never a real pid");
    }
}
