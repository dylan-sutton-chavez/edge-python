use crate::vm::Limits;

// The whole actor, groups plus the runtime knobs the CLI resolved from actor.yml.
pub struct ActorConfig {
    pub groups: Vec<Group>,
    pub max_actors: usize,
}

// One program run as a pool of actors, with its limits and output.
pub struct Group {
    pub name: String,
    // Parsed source plus its directory, the scheduler boots a fresh VM per actor from these.
    pub source: String,
    pub dir: String,
    pub replicas: usize,
    pub limits: Limits,
    pub preempt: usize,
    pub out: Out,
    // Untrusted mode, actors compile each message as code and cannot send to other groups.
    pub eval: bool,
    // Times a crashing message is retried before it is dropped to the dead count.
    pub retry: usize,
    // Seed messages delivered before the actor starts, the test and CLI entry points.
    pub inbox: Vec<Message>,
}

// Where a actor's print output goes.
pub enum Out {
    Stdout,
    File(String),
    Null,
}

// A message in flight, body is the string receive() hands the actor.
#[derive(Clone)]
pub struct Message {
    pub group: String,
    pub body: String,
    // Failed delivery attempts so far, a retrying group drops it past its retry count.
    pub attempts: usize,
    // A live caller waiting on the result, set only by the control endpoint for eval runs.
    pub reply: Option<std::sync::mpsc::Sender<Result<String, String>>>,
}
