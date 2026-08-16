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

/* Direct tests against the public bridge entry points, a dedicated cdylib fixture would only forward to these same symbols. */
mod bridge_ops {
    use super::*;
    use compiler::abi::{Op, Tag, WireValue};
    use compiler::bridge::{host_edge_decode, host_edge_encode, host_edge_op, take_error, VmGuard};

    struct Fixture {
        vm: compiler::vm::VM<'static>,
    }

    impl Fixture {
        fn new() -> Self {
            let chunk = parse_source("x = 1\n", "", None).expect("parse");
            Self { vm: boot_vm(chunk, Limits::sandbox(), 0) }
        }

        fn encode_raw(&mut self, data: &[u8]) -> u32 {
            let _guard = VmGuard::new(&mut self.vm);
            let h = unsafe { host_edge_encode(Tag::Raw as u32, data.as_ptr(), data.len() as u32) };
            assert_ne!(h, 0, "encode failed");
            h
        }

        fn op(&mut self, op: Op, recv: u32, name: &str, argv: &[u32]) -> Result<u32, String> {
            let _guard = VmGuard::new(&mut self.vm);
            let mut out = 0u32;
            let rc = unsafe {
                host_edge_op(op as u32, recv, name.as_ptr(), name.len() as u32, argv.as_ptr(), argv.len() as u32, &mut out)
            };
            if rc == 0 { Ok(out) } else { Err(take_error().map(|(_, m)| m).unwrap_or_default()) }
        }

        fn decode(&mut self, h: u32) -> (u32, Vec<u8>) {
            let _guard = VmGuard::new(&mut self.vm);
            let mut tag = 0u32;
            let mut buf = vec![0u8; 4096];
            let n = unsafe { host_edge_decode(h, &mut tag, buf.as_mut_ptr(), buf.len() as u32) };
            assert!(n >= 0, "decode failed");
            buf.truncate(n as usize);
            (tag, buf)
        }

        fn decode_int(&mut self, h: u32) -> i128 {
            let (tag, bytes) = self.decode(h);
            assert_eq!(tag, Tag::Int as u32, "expected an int");
            i128::from_le_bytes(bytes.try_into().unwrap())
        }

        fn decode_list(&mut self, h: u32) -> Vec<i128> {
            let (tag, bytes) = self.decode(h);
            match WireValue::decode_body(tag, &bytes) {
                Some(WireValue::List(items)) => items.into_iter().map(|i| match i {
                    WireValue::Int(v) => v,
                    other => panic!("expected int item, got {other:?}"),
                }).collect(),
                other => panic!("expected a list, got {other:?}"),
            }
        }

        fn len_of(&mut self, h: u32) -> i128 {
            let r = self.op(Op::Len, h, "", &[]).unwrap();
            self.decode_int(r)
        }
    }

    #[test]
    fn ffi_len_accepts_bytes() {
        let mut f = Fixture::new();
        let h = f.encode_raw(&[1, 2, 255]);
        assert_eq!(f.len_of(h), 3);
    }

    #[test]
    fn ffi_len_of_empty_bytes_is_zero() {
        let mut f = Fixture::new();
        let h = f.encode_raw(&[]);
        assert_eq!(f.len_of(h), 0);
    }

    #[test]
    fn ffi_iter_flattens_bytes_to_ints() {
        let mut f = Fixture::new();
        let h = f.encode_raw(&[1, 2, 255]);
        let list = f.op(Op::Iter, h, "", &[]).unwrap();
        assert_eq!(f.decode_list(list), vec![1, 2, 255]);
    }

    #[test]
    fn ffi_iter_of_empty_bytes_is_empty() {
        let mut f = Fixture::new();
        let h = f.encode_raw(&[]);
        let list = f.op(Op::Iter, h, "", &[]).unwrap();
        assert_eq!(f.decode_list(list), Vec::<i128>::new());
    }

    #[test]
    fn ffi_iter_result_is_a_real_list() {
        let mut f = Fixture::new();
        let h = f.encode_raw(&[7, 8, 9]);
        let list = f.op(Op::Iter, h, "", &[]).unwrap();
        assert_eq!(f.len_of(list), 3);
    }

    fn call_with_n_args(f: &mut Fixture, n: usize) -> Result<u32, String> {
        let recv = f.encode_raw(&[0]);
        let argv: Vec<u32> = (0..n).map(|_| f.encode_raw(&[0])).collect();
        f.op(Op::Call, recv, "__call__", &argv)
    }

    #[test]
    fn ffi_call_zero_args_still_dispatches() {
        let mut f = Fixture::new();
        let err = call_with_n_args(&mut f, 0).unwrap_err();
        assert!(err.contains("not callable"), "unexpected error: {err}");
    }

    #[test]
    fn ffi_call_255_args_passes_the_guard() {
        let mut f = Fixture::new();
        let err = call_with_n_args(&mut f, 255).unwrap_err();
        assert!(err.contains("not callable"), "unexpected error: {err}");
    }

    #[test]
    fn ffi_call_256_args_raises_arity_error() {
        let mut f = Fixture::new();
        let err = call_with_n_args(&mut f, 256).unwrap_err();
        assert!(err.contains("too many arguments"), "unexpected error: {err}");
    }

    #[test]
    fn ffi_call_300_args_raises_arity_error() {
        let mut f = Fixture::new();
        let err = call_with_n_args(&mut f, 300).unwrap_err();
        assert!(err.contains("too many arguments"), "unexpected error: {err}");
    }

    #[test]
    fn ffi_call_arity_error_leaves_no_stack_desync() {
        let mut f = Fixture::new();
        let _ = call_with_n_args(&mut f, 256).unwrap_err();
        let h = f.encode_raw(&[1, 2, 3]);
        assert_eq!(f.len_of(h), 3);
    }
}
