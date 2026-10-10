use anyhow::{anyhow, Context, Result};
use crate::actor::{Group, Message, Out, ActorConfig};
use crate::host::RunLimits;
use compiler::vm::Limits;
use serde::Deserialize;
use std::path::Path;

// The actor.yml shape, groups keyed by name with per-group overrides.
#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    runtime: Runtime,
    #[serde(default)]
    groups: std::collections::BTreeMap<String, GroupSpec>,
}

#[derive(Deserialize, Default)]
struct Runtime {
    #[serde(default)]
    max_actors: Option<usize>,
    // "auto" for one scheduler per core, or a fixed thread count.
    #[serde(default)]
    schedulers: Option<serde_yaml_ng::Value>,
    // Host and port for the live ingress, its presence turns the actor into a server.
    #[serde(default)]
    listen: Option<String>,
    // Path to the durable log that survives restarts, defaults beside the manifest.
    #[serde(default)]
    durable: Option<String>,
    // Host and port for the metrics endpoint, healthz and stats for orchestrators.
    #[serde(default)]
    control: Option<String>,
}

#[derive(Deserialize)]
struct GroupSpec {
    // A script path relative to the manifest, or use `code` for an inline body.
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    replicas: Option<usize>,
    // Untrusted mode, each message is compiled as its own program with no send access.
    #[serde(default)]
    eval: bool,
    // Times a crashing message is retried on another actor before it is dropped.
    #[serde(default)]
    retry: usize,
    #[serde(default)]
    limits: LimitSpec,
    #[serde(default)]
    out: Option<String>,
    // Seed messages, the entry point that kicks a actor run.
    #[serde(default)]
    seed: Vec<String>,
}

// Seconds an eval run lasts unless its group says, and the most a group may give it, an instance held all along.
const EVAL_TIMEOUT: u64 = 10;
const EVAL_TIMEOUT_MAX: u64 = 300;

/* What a group may hold and spend, memory in MB, and how often it yields. */
#[derive(Deserialize, Default)]
struct LimitSpec {
    memory: Option<usize>,
    ops: Option<usize>,
    preempt: Option<usize>,
    // Seconds an eval run may last, its waits included.
    timeout: Option<u64>,
    // Gone, kept only so a file still naming them hears what replaced them.
    heap: Option<serde::de::IgnoredAny>,
    calls: Option<serde::de::IgnoredAny>,
}

// Loads actor.yml, boots the described pool, `manifest_path` overrides every group's manifest walk-up.
pub fn run(path: &Path, manifest_path: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: Manifest = serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut dir = path.parent().and_then(|p| p.to_str()).unwrap_or(".").replace('\\', "/");
    // The manifest walk-up probes `{dir}edge.json`, so a named directory needs its slash.
    if !dir.is_empty() && !dir.ends_with('/') {
        dir.push('/');
    }
    let manifest_path = manifest_path.map(|p| p.to_string_lossy().replace('\\', "/"));
    // Read once, so a malformed eval grant stops the boot instead of a run.
    let ceiling = eval_ceiling(&manifest_path.clone().unwrap_or_else(|| format!("{dir}edge.json")))?;

    let mut groups = Vec::new();
    for (name, spec) in manifest.groups {
        // A directory target runs its main.py with the project as base dir.
        let (source, group_dir) = match (&spec.code, &spec.run, spec.eval) {
            (Some(code), _, _) => (code.clone(), dir.clone()),
            (None, Some(run), _) => load_run(&dir, run).with_context(|| format!("loading '{run}' for group '{name}'"))?,
            (None, None, true) => (String::new(), dir.clone()),
            (None, None, false) => return Err(anyhow!("group '{name}' needs run, code or eval")),
        };
        if spec.limits.heap.is_some() {
            return Err(anyhow!("group '{name}' sets limits.heap, which is gone, limits.memory caps what a run holds, in MB"));
        }
        if spec.limits.calls.is_some() {
            return Err(anyhow!("group '{name}' sets limits.calls, which is gone, the call depth is fixed at 256"));
        }
        if spec.limits.timeout.is_some() && !spec.eval {
            return Err(anyhow!("group '{name}' sets limits.timeout, which only an eval group takes"));
        }
        let timeout = spec.limits.timeout.unwrap_or(EVAL_TIMEOUT);
        if !(1..=EVAL_TIMEOUT_MAX).contains(&timeout) {
            return Err(anyhow!("group '{name}' sets limits.timeout to {timeout}, it takes 1 to {EVAL_TIMEOUT_MAX} seconds"));
        }
        let limits = RunLimits { memory: spec.limits.memory, ops: spec.limits.ops }.engine().unwrap_or_else(Limits::sandbox);
        let inbox = spec.seed.into_iter().map(|body| Message { group: name.clone(), body, attempts: 0, reply: None }).collect();
        groups.push(Group {
            name,
            source,
            dir: group_dir,
            manifest: manifest_path.clone(),
            replicas: spec.replicas.unwrap_or(1),
            eval: spec.eval,
            timeout,
            ceiling: ceiling.clone(),
            retry: spec.retry,
            limits,
            preempt: spec.limits.preempt.unwrap_or(2000),
            out: parse_out(spec.out.as_deref()),
            inbox,
        });
    }
    if groups.is_empty() {
        return Err(anyhow!("actor has no groups"));
    }

    let config = ActorConfig { groups, max_actors: manifest.runtime.max_actors.unwrap_or(usize::MAX) };
    // A listen address turns the actor into a live server, else it processes to quiescence.
    let code = match &manifest.runtime.listen {
        Some(listen) => {
            let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
            // A relative durable path sits beside the manifest, the default is actor.wal there.
            let wal = match &manifest.runtime.durable {
                Some(d) => Path::new(&dir).join(d),
                None => Path::new(&dir).join("actor.wal"),
            };
            // A control address serves healthz, stats and eval replies on its own thread.
            let control = manifest.runtime.control.as_deref().map(|c| {
                (c.strip_prefix("tcp://").unwrap_or(c).to_string(), std::sync::Arc::new(crate::actor::Stats::default()))
            });
            let stats = control.as_ref().map(|(_, s)| s.clone());
            // The groups the control endpoint answers, captured before config moves into serve.
            let eval: Vec<(String, u64)> = config.groups.iter().filter(|g| g.eval).map(|g| (g.name.clone(), g.timeout)).collect();
            let names: Vec<String> = config.groups.iter().map(|g| g.name.clone()).collect();
            crate::actor::serve(config, addr, &wal, stats, move |tx, wal| {
                if let Some((addr, stats)) = control {
                    spawn_control(&addr, tx, wal, names, eval, stats);
                }
            })
        }
        None => crate::actor::run(config, resolve_schedulers(manifest.runtime.schedulers.as_ref())),
    };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/* What the pool edge.json grants its eval groups, nothing when it names no eval holder. */
fn eval_ceiling(path: &str) -> Result<Vec<String>> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(Vec::new()) };
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| format!("parsing {path}"))?;
    let Some(entries) = manifest.pointer("/permissions/eval") else { return Ok(Vec::new()) };
    if let Some(problem) = crate::host::system::check(&serde_json::json!({ "eval": entries })) {
        return Err(anyhow!("edge.json at '{path}': {problem}"));
    }
    Ok(serde_json::from_value(entries.clone())?)
}

/* Loads a run target as source plus base dir, a directory runs its main.py from inside it. */
fn load_run(dir: &str, run: &str) -> Result<(String, String)> {
    let path = Path::new(dir).join(run);
    if path.is_dir() {
        let entry = path.join("main.py");
        let source = std::fs::read_to_string(&entry).with_context(|| format!("reading {}", entry.display()))?;
        // The resolver walks up from `{base}edge.json`, so a directory base needs a trailing slash.
        let mut base = path.to_string_lossy().replace('\\', "/");
        if !base.ends_with('/') { base.push('/'); }
        return Ok((source, base));
    }
    Ok((std::fs::read_to_string(&path)?, dir.to_string()))
}

// Resolves the scheduler count, auto or absent means one per core.
fn resolve_schedulers(value: Option<&serde_yaml_ng::Value>) -> usize {
    let cores = || std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    match value {
        Some(serde_yaml_ng::Value::Number(n)) => n.as_u64().map(|n| n as usize).unwrap_or(1).max(1),
        _ => cores(),
    }
}

// Caps a request body, an eval bundle can be a few MB of base64.
const MAX_BODY: u64 = 16 << 20;
// Seconds an eval caller waits past the group timeout, room for the runs queued ahead.
const EVAL_QUEUE_WAIT: u64 = 20;

type HttpResp = tiny_http::Response<std::io::Cursor<Vec<u8>>>;

// Serves counters at /stats, publishing at /pub/<group> and eval replies at /eval/<group>.
fn spawn_control(addr: &str, tx: std::sync::mpsc::Sender<Message>, wal: std::sync::Arc<std::sync::Mutex<crate::actor::Wal>>, groups: Vec<String>, eval: Vec<(String, u64)>, stats: std::sync::Arc<crate::actor::Stats>) {
    let Ok(server) = tiny_http::Server::http(addr) else {
        eprintln!("warning: cannot bind control endpoint '{addr}'");
        return;
    };
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let url = req.url().to_string();
            let post = *req.method() == tiny_http::Method::Post;
            let resp = match (post, url.as_str()) {
                (_, "/stats") => json(stats.to_json()),
                (true, p) if let Some(g) = p.strip_prefix("/eval/") => run_eval(g, &mut req, &tx, &eval),
                (true, p) if let Some(g) = p.strip_prefix("/pub/") => match read_body(&mut req) {
                    Ok(body) => publish(g, body, &tx, &wal, &groups),
                    Err(resp) => resp,
                },
                _ => not_found(),
            };
            let _ = req.respond(resp);
        }
    });
}

// Queues a message for a group, appended to the wal first like the tcp ingress does.
fn publish(group: &str, body: String, tx: &std::sync::mpsc::Sender<Message>, wal: &std::sync::Arc<std::sync::Mutex<crate::actor::Wal>>, groups: &[String]) -> HttpResp {
    if !groups.iter().any(|g| g == group) {
        return not_found();
    }
    let msg = Message { group: group.to_string(), body, attempts: 0, reply: None };
    wal.lock().unwrap().append(&msg);
    if tx.send(msg).is_err() {
        return tiny_http::Response::from_string("actor is down").with_status_code(503);
    }
    json("{\"ok\":true}".to_string()).with_status_code(202)
}

// Answers an eval run, the body is a snippet or an EDGEPKG bundle, the reply its print.
fn run_eval(group: &str, req: &mut tiny_http::Request, tx: &std::sync::mpsc::Sender<Message>, eval: &[(String, u64)]) -> HttpResp {
    let Some(&(_, timeout)) = eval.iter().find(|(g, _)| g == group) else {
        return not_found();
    };
    let body = match read_body(req) {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    let (reply, result) = std::sync::mpsc::channel();
    let msg = Message { group: group.to_string(), body, attempts: 0, reply: Some(reply) };
    if tx.send(msg).is_err() {
        return tiny_http::Response::from_string("actor is down").with_status_code(503);
    }
    match result.recv_timeout(std::time::Duration::from_secs(timeout + EVAL_QUEUE_WAIT)) {
        Ok(Ok(stdout)) => json(format!("{{\"ok\":true,\"stdout\":{}}}", json_str(&stdout))),
        Ok(Err(e)) => json(format!("{{\"ok\":false,\"error\":{}}}", json_str(&e))).with_status_code(500),
        Err(_) => tiny_http::Response::from_string("eval timed out").with_status_code(504),
    }
}

// Reads a request body up to the cap, 413 when it spills over.
fn read_body(req: &mut tiny_http::Request) -> Result<String, HttpResp> {
    use std::io::Read;
    let mut body = String::new();
    let _ = req.as_reader().take(MAX_BODY + 1).read_to_string(&mut body);
    if body.len() as u64 > MAX_BODY {
        return Err(tiny_http::Response::from_string("body too large").with_status_code(413));
    }
    Ok(body)
}

fn not_found() -> HttpResp {
    tiny_http::Response::from_string("not found").with_status_code(404)
}

// A JSON response with the content type set.
fn json(body: String) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(body)
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap())
}

// Renders s as a quoted JSON string, escaping the characters JSON reserves.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// Maps the out uri to a sink, stdout by default.
fn parse_out(out: Option<&str>) -> Out {
    match out {
        None | Some("stdout") => Out::Stdout,
        Some("null") => Out::Null,
        Some(uri) => match uri.strip_prefix("file://") {
            Some(path) => Out::File(path.to_string()),
            None => Out::Stdout,
        },
    }
}
