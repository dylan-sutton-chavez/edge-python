use super::Reactor;

// Drives the reactor on the executor thread, no separate reactor thread is spawned.
pub struct PollPark {
    reactor: Reactor,
}

impl PollPark {
    pub fn new(reactor: Reactor) -> Self {
        PollPark { reactor }
    }
}

impl rt::Park for PollPark {
    fn park(&self) {
        self.reactor.tick(None);
    }

    fn unpark(&self) {
        self.reactor.notify();
    }
}
