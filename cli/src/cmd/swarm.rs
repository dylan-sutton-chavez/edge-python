use anyhow::{anyhow, Context, Result};
use compiler::native::swarm::{Group, Message, Out, SwarmConfig};
use compiler::vm::Limits;
use serde::Deserialize;
use std::path::Path;

// The swarm.yml shape, groups keyed by name with per-group overrides.
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
    max_nodes: Option<usize>,
    // "auto" for one scheduler per core, or a fixed thread count.
    #[serde(default)]
    schedulers: Option<serde_yaml_ng::Value>,
    // Host:port for the live ingress, its presence turns the swarm into a server.
    #[serde(default)]
    listen: Option<String>,
    // Path to the durable log that survives restarts, defaults beside the manifest.
    #[serde(default)]
    durable: Option<String>,
    // Host:port for the metrics endpoint, healthz and stats for orchestrators.
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
    // Times a crashing message is retried on another node before it is dropped.
    #[serde(default)]
    retry: usize,
    #[serde(default)]
    limits: LimitSpec,
    #[serde(default)]
    out: Option<String>,
    // Seed messages, the entry point that kicks a swarm run.
    #[serde(default)]
    seed: Vec<String>,
}

#[derive(Deserialize, Default)]
struct LimitSpec {
    heap: Option<usize>,
    ops: Option<usize>,
    calls: Option<usize>,
    preempt: Option<usize>,
}

// Loads swarm.yml, boots the described swarm, returns its exit code.
pub fn run(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: Manifest = serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let dir = path.parent().and_then(|p| p.to_str()).unwrap_or(".").to_string();

    let mut groups = Vec::new();
    for (name, spec) in manifest.groups {
        // A run target may be a whole project directory, then the entry and base dir move into it.
        let (source, group_dir) = match (&spec.code, &spec.run, spec.eval) {
            (Some(code), _, _) => (code.clone(), dir.clone()),
            (None, Some(run), _) => load_run(&dir, run).with_context(|| format!("loading '{run}' for group '{name}'"))?,
            (None, None, true) => (String::new(), dir.clone()),
            (None, None, false) => return Err(anyhow!("group '{name}' needs run, code or eval")),
        };
        let sandbox = Limits::sandbox();
        let limits = Limits {
            heap: spec.limits.heap.unwrap_or(sandbox.heap),
            ops: spec.limits.ops.unwrap_or(sandbox.ops),
            calls: spec.limits.calls.unwrap_or(sandbox.calls),
        };
        let inbox = spec.seed.into_iter().map(|body| Message { group: name.clone(), body, attempts: 0, reply: None }).collect();
        groups.push(Group {
            name,
            source,
            dir: group_dir,
            replicas: spec.replicas.unwrap_or(1),
            eval: spec.eval,
            retry: spec.retry,
            limits,
            preempt: spec.limits.preempt.unwrap_or(2000),
            out: parse_out(spec.out.as_deref()),
            inbox,
        });
    }
    if groups.is_empty() {
        return Err(anyhow!("swarm has no groups"));
    }

    let config = SwarmConfig { groups, max_nodes: manifest.runtime.max_nodes.unwrap_or(usize::MAX) };
    // A listen address turns the swarm into a live server, else it processes to quiescence.
    let code = match &manifest.runtime.listen {
        Some(listen) => {
            let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
            // A relative durable path sits beside the manifest, the default is swarm.wal there.
            let wal = match &manifest.runtime.durable {
                Some(d) => Path::new(&dir).join(d),
                None => Path::new(&dir).join("swarm.wal"),
            };
            // A control address serves healthz, stats and eval replies on its own thread.
            let control = manifest.runtime.control.as_deref().map(|c| {
                (c.strip_prefix("tcp://").unwrap_or(c).to_string(), std::sync::Arc::new(compiler::native::swarm::Stats::default()))
            });
            let stats = control.as_ref().map(|(_, s)| s.clone());
            // The groups the control endpoint answers, captured before config moves into serve.
            let eval: Vec<String> = config.groups.iter().filter(|g| g.eval).map(|g| g.name.clone()).collect();
            let names: Vec<String> = config.groups.iter().map(|g| g.name.clone()).collect();
            compiler::native::swarm::serve(config, addr, &wal, stats, move |tx, wal| {
                if let Some((addr, stats)) = control {
                    spawn_control(&addr, tx, wal, names, eval, stats);
                }
            })
        }
        None => compiler::native::swarm::run(config, resolve_schedulers(manifest.runtime.schedulers.as_ref())),
    };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/* Loads a run target, returning its source and the base dir imports resolve against.
   A directory runs its main.py and resolves packages.json inside it, a file uses the manifest dir. */
fn load_run(dir: &str, run: &str) -> Result<(String, String)> {
    let path = Path::new(dir).join(run);
    if path.is_dir() {
        let entry = path.join("main.py");
        let source = std::fs::read_to_string(&entry).with_context(|| format!("reading {}", entry.display()))?;
        // The resolver walks up from `{base}packages.json`, so a directory base needs a trailing slash.
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

type HttpResp = tiny_http::Response<std::io::Cursor<Vec<u8>>>;

// Serves counters at /stats, publishing at /pub/<group> and eval replies at /eval/<group>.
fn spawn_control(addr: &str, tx: std::sync::mpsc::Sender<Message>, wal: std::sync::Arc<std::sync::Mutex<compiler::native::swarm::Wal>>, groups: Vec<String>, eval: Vec<String>, stats: std::sync::Arc<compiler::native::swarm::Stats>) {
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
fn publish(group: &str, body: String, tx: &std::sync::mpsc::Sender<Message>, wal: &std::sync::Arc<std::sync::Mutex<compiler::native::swarm::Wal>>, groups: &[String]) -> HttpResp {
    if !groups.iter().any(|g| g == group) {
        return not_found();
    }
    let msg = Message { group: group.to_string(), body, attempts: 0, reply: None };
    wal.lock().unwrap().append(&msg);
    if tx.send(msg).is_err() {
        return tiny_http::Response::from_string("swarm is down").with_status_code(503);
    }
    json("{\"ok\":true}".to_string()).with_status_code(202)
}

// Answers an eval run, the request body is a snippet or an EDGEPKG bundle and the reply its print.
fn run_eval(group: &str, req: &mut tiny_http::Request, tx: &std::sync::mpsc::Sender<Message>, eval: &[String]) -> HttpResp {
    if !eval.iter().any(|g| g == group) {
        return not_found();
    }
    let body = match read_body(req) {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    let (reply, result) = std::sync::mpsc::channel();
    let msg = Message { group: group.to_string(), body, attempts: 0, reply: Some(reply) };
    if tx.send(msg).is_err() {
        return tiny_http::Response::from_string("swarm is down").with_status_code(503);
    }
    match result.recv_timeout(std::time::Duration::from_secs(30)) {
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
