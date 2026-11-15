use crate::planner::{Candidate, SearchPool, Snapshot, Task, fallback, CrossBus, ELITE_INTERVAL};
use std::sync::Arc;
use crate::wire::{ClientMessage, FramedSocket, ServerMessage, VERSION, token_matches};
use std::error::Error;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tester::model::{Problem, Request};

#[derive(Clone)]
struct PeerSpec {
    address: SocketAddr,
    threads: usize,
    offset: usize,
}
pub struct Options {
    pub mode: String,
    pub threads: usize,
    pub seed: u64,
    offset: usize,
    listen: SocketAddr,
    peers: Vec<PeerSpec>,
    token: String,
    cross_elite: bool,
}
impl Options {
    pub fn parse() -> Result<Self, Box<dyn Error>> {
        let mut args = std::env::args().skip(1);
        let mut mode = "coordinator".to_owned();
        let mut threads = 32;
        let mut seed = 0;
        let mut offset = 0;
        let mut listen = "127.0.0.1:39370".parse()?;
        let mut peers = Vec::new();
        let mut cross_elite = false;
        while let Some(arg) = args.next() {
            if arg == "--help" {
                println!(
                    "distributed-solver --mode coordinator|worker|parallel --threads N --seed N\n  coordinator: --peers IP:PORT/TOTAL_THREADS/OFFSET,...\n  worker: --listen IP:PORT --worker-offset N\n  network modes: PROCON37_DISTRIBUTED_TOKEN_FILE (32..128 printable bytes)\n  threads includes the main/control thread; cluster maximum is 80\n  optional: --cross-elite on|off (default off; VERSION=2 cohort)"
                );
                std::process::exit(0);
            }
            let value = args
                .next()
                .ok_or_else(|| format!("value missing for {arg}"))?;
            match arg.as_str() {
                "--cross-elite" => cross_elite = match value.as_str() {
                    "on" => true, "off" => false, _ => return Err("cross-elite must be on|off".into()),
                },
                "--mode" => mode = value,
                "--threads" => threads = value.parse()?,
                "--seed" => seed = value.parse()?,
                "--worker-offset" => offset = value.parse()?,
                "--listen" => listen = value.parse()?,
                "--peers" => {
                    for part in value.split(',').filter(|part| !part.is_empty()) {
                        let fields: Vec<_> = part.split('/').collect();
                        if fields.len() != 3 {
                            return Err("peer must be IP:PORT/TOTAL_THREADS/OFFSET".into());
                        }
                        peers.push(PeerSpec {
                            address: fields[0].parse()?,
                            threads: fields[1].parse()?,
                            offset: fields[2].parse()?,
                        });
                    }
                }
                _ => return Err(format!("unknown option {arg}").into()),
            }
        }
        if !["coordinator", "worker", "parallel"].contains(&mode.as_str())
            || !(2..=80).contains(&threads)
        {
            return Err("invalid mode or total thread count".into());
        }
        if mode != "coordinator" && !peers.is_empty() {
            return Err("only coordinator accepts peers".into());
        }
        if peers.len() > 2
            || peers.iter().any(|peer| !(2..=79).contains(&peer.threads))
            || threads + peers.iter().map(|peer| peer.threads).sum::<usize>() > 80
            || offset + threads - 1 > 79
        {
            return Err("cluster thread/worker limit exceeded".into());
        }
        let mut worker_ids: Vec<usize> = (offset..offset + threads - 1).collect();
        for peer in &peers {
            if peer.offset + peer.threads - 1 > 79 {
                return Err("peer worker range exceeded".into());
            }
            for worker in peer.offset..peer.offset + peer.threads - 1 {
                if worker_ids.contains(&worker) {
                    return Err("overlapping worker ranges".into());
                }
                worker_ids.push(worker);
            }
        }
        if cross_elite && mode == "parallel" { return Err("cross-elite requires a network mode".into()); }
        let token = if mode != "parallel" {
            let path = std::env::var("PROCON37_DISTRIBUTED_TOKEN_FILE")
                .map_err(|_| "PROCON37_DISTRIBUTED_TOKEN_FILE is required")?;
            let token = std::fs::read_to_string(path)?.trim().to_owned();
            if !(32..=128).contains(&token.len())
                || !token.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err("invalid shared token length or characters".into());
            }
            token
        } else {
            String::new()
        };
        Ok(Self {
            mode,
            threads,
            seed,
            offset,
            listen,
            peers,
            token,
            cross_elite,
        })
    }
}
struct EliteLink {
    next: Instant,
    sent: [u64;80],
}
impl EliteLink {
    fn new() -> Self { Self { next: Instant::now() + ELITE_INTERVAL, sent: [0;80] } }
    fn poll(&mut self, socket: &mut FramedSocket, bus: &CrossBus, destination: Option<usize>, relay: bool) {
        if Instant::now() < self.next { return; }
        self.next = Instant::now() + ELITE_INTERVAL;
        let Some(packet) = bus.outgoing(destination, relay, &self.sent) else { return; };
        let result = if relay { socket.send_elite(&ClientMessage::Elite((*packet).clone())) }
                     else { socket.send_elite(&ServerMessage::Elite((*packet).clone())) };
        // Optional sharing cannot turn a valid final Result into a transport error.
        if let Ok(Some(sent)) = result {
            let (origin, sequence) = packet.identity();
            self.sent[origin] = sequence;
            bus.note_send(!sent);
        }
    }
}
struct Peer {
    spec: PeerSpec,
    socket: Option<FramedSocket>,
    ready: bool,
    pending: bool,
    elite_link: EliteLink,
    cross_elite: bool,
}
impl Peer {
    fn start(&mut self, token: &str, session: u64, request: u64, task: &Task, deadline: Instant) {
        self.elite_link = EliteLink::new();
        let result = (|| -> io::Result<()> {
            let left = deadline.saturating_duration_since(Instant::now());
            if left < Duration::from_millis(150) {
                return Err(io::Error::other("no network budget"));
            }
            if self.socket.is_none() {
                let stream = TcpStream::connect_timeout(
                    &self.spec.address,
                    Duration::from_millis(45).min(left),
                )?;
                let mut socket = FramedSocket::new(stream)?;
                socket.send(&ClientMessage::Authenticate {
                    version: VERSION,
                    token: token.to_owned(),
                    cross_elite: self.cross_elite,
                })?;
                self.socket = Some(socket);
                self.ready = false;
            }
            let budget_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .saturating_sub(150) as u64;
            self.socket.as_mut().unwrap().send(&ClientMessage::Solve {
                session,
                request,
                digest: task.digest(),
                budget_ms,
                task: task.clone(),
            })?;
            self.pending = true;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!(
                "dist peer={} request={} unavailable={error}",
                self.spec.address, request
            );
            self.socket = None;
            self.ready = false;
            self.pending = false;
        }
    }
    fn poll(&mut self, session: u64, request: u64, digest: u64, cross: Option<&CrossBus>) -> Option<Candidate> {
        let Some(socket) = &mut self.socket else {
            return None;
        };
        let messages = match socket.poll::<ServerMessage>() {
            Ok(messages) => messages,
            Err(error) => {
                eprintln!(
                    "dist peer={} request={} disconnected={error}",
                    self.spec.address, request
                );
                self.socket = None;
                self.ready = false;
                self.pending = false;
                return None;
            }
        };
        let mut result = None;
        for message in messages {
            match message {
                ServerMessage::Elite(packet) => {
                    if self.ready && self.pending {
                        if let Some(bus) = cross { bus.receive(packet, Some((self.spec.offset, self.spec.threads - 1))); }
                    }
                }
                ServerMessage::Ready {
                    version,
                    total_threads,
                    cross_elite,
                    offset,
                } => {
                    if version != VERSION
                        || cross_elite != self.cross_elite
                        || total_threads != self.spec.threads
                        || offset != self.spec.offset
                    {
                        eprintln!(
                            "dist peer={} rejected_configuration version={version} threads={total_threads} offset={offset}",
                            self.spec.address
                        );
                        self.socket = None;
                        self.ready = false;
                        self.pending = false;
                        return None;
                    }
                    self.ready = true;
                }
                ServerMessage::Result {
                    session: got_session,
                    request: got_request,
                    digest: got_digest,
                    started,
                    completed,
                    elapsed_ms,
                    candidate,
                } => {
                    if !self.ready
                        || got_session != session
                        || got_request != request
                        || got_digest != digest
                    {
                        eprintln!(
                            "dist peer={} ignored_stale_or_mismatched_result request={got_request}",
                            self.spec.address
                        );
                        continue;
                    }
                    self.pending = false;
                    eprintln!(
                        "dist peer={} request={} completed={completed}/{started} elapsed_ms={elapsed_ms} rx_bytes={} tx_bytes={}",
                        self.spec.address,
                        request,
                        self.socket.as_ref().unwrap().received_bytes(),
                        self.socket.as_ref().unwrap().sent_bytes()
                    );
                    if let Some(candidate) = candidate {
                        if (self.spec.offset..self.spec.offset + self.spec.threads - 1)
                            .contains(&candidate.worker)
                        {
                            result = Some(candidate);
                        } else {
                            eprintln!("dist peer={} rejected_worker_id", self.spec.address);
                        }
                    }
                }
                ServerMessage::Error { message } => {
                    eprintln!("dist peer={} error={message}", self.spec.address);
                    self.socket = None;
                    self.ready = false;
                    self.pending = false;
                    return None;
                }
            }
        }
        if self.ready && self.pending {
            if let (Some(socket), Some(bus)) = (&mut self.socket, cross) {
                self.elite_link.poll(socket, bus, Some(self.spec.offset), true);
            }
        }
        result
    }
}

pub struct Coordinator {
    options: Options,
    pool: SearchPool,
    peers: Vec<Peer>,
    session: u64,
    sequence: u64,
    problem: Option<Problem>,
    state: Option<Snapshot>,
}
impl Coordinator {
    pub fn new(options: Options) -> Result<Self, Box<dyn Error>> {
        let pool = SearchPool::new(options.threads - 1, options.offset);
        let peers = options
            .peers
            .iter()
            .cloned()
            .map(|spec| Peer {
                spec,
                socket: None,
                ready: false,
                pending: false,
                elite_link: EliteLink::new(),
                cross_elite: options.cross_elite,
            })
            .collect();
        let session = (SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64)
            ^ u64::from(std::process::id());
        eprintln!(
            "dist mode=coordinator total_threads={} cluster_threads={} search_threads={} session={session} cross_elite={}",
            options.threads,
            options.threads + options.peers.iter().map(|peer| peer.threads).sum::<usize>(),
            options.threads - 1
                + options
                    .peers
                    .iter()
                    .map(|peer| peer.threads - 1)
                    .sum::<usize>(),
            options.cross_elite
        );
        Ok(Self {
            options,
            pool,
            peers,
            session,
            sequence: 0,
            problem: None,
            state: None,
        })
    }
    pub fn solve(&mut self, request: &Request) -> Result<serde_json::Value, Box<dyn Error>> {
        let started = Instant::now();
        self.sequence += 1;
        let id = self.sequence;
        let task = if request.phase == "select" {
            let problem = request
                .problem
                .clone()
                .ok_or("select request without problem")?;
            problem.validate_players(problem.players)?;
            self.problem = Some(problem.clone());
            self.state = None;
            Task::selection(problem, self.options.seed)
        } else {
            Task::planning(
                self.problem.clone().ok_or("plan before selection")?,
                self.options.seed,
                self.state.clone().ok_or("plan without adopted state")?,
                request.state.clone().ok_or("plan without observation")?,
                request.steps.unwrap_or(1),
                request.previous_accepted.unwrap_or(true),
            )
        };
        let safe = fallback(&task);
        let mut best = safe.clone();
        let digest = task.digest();
        let margin = (request.time_limit_ms / 20)
            .clamp(250, 1500)
            .min(request.time_limit_ms);
        let search_deadline =
            started + Duration::from_millis(request.time_limit_ms.saturating_sub(margin));
        let cutoff =
            started + Duration::from_millis(request.time_limit_ms.saturating_sub(margin / 2));
        let cross = (self.options.cross_elite && !task.is_select()).then(||
            CrossBus::new(self.session, id, digest, self.options.offset, self.options.threads - 1, search_deadline));
        let mut count = 0;
        let mut remote_count = 0;
        let mut launched = 0;
        if request.time_limit_ms > 250 {
            launched = self.pool.begin_cross(id, &task, search_deadline, cross.clone());
            for peer in &mut self.peers {
                peer.start(
                    &self.options.token,
                    self.session,
                    id,
                    &task,
                    search_deadline,
                );
            }
            loop {
                for candidate in self.pool.poll(id) {
                    count += 1;
                    if candidate.better_than(&best) && candidate.valid_for(&task, &safe) {
                        best = candidate;
                    }
                }
                for peer in &mut self.peers {
                    if let Some(candidate) = peer.poll(self.session, id, digest, cross.as_deref()) {
                        remote_count += 1;
                        if candidate.better_than(&best) && candidate.valid_for(&task, &safe) {
                            best = candidate;
                        }
                    }
                }
                if Instant::now() >= cutoff
                    || (self.pool.pending(id) == 0 && self.peers.iter().all(|peer| !peer.pending))
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        eprintln!(
            "dist request={id} external_request={} phase={} input_digest={digest:016x} selected_worker={} value={:.3} local_completed={count}/{launched} remote_results={remote_count}/{} elapsed_ms={} state_digest={:016x}",
            request.request_id,
            request.phase,
            best.worker,
            best.value,
            self.peers.len(),
            started.elapsed().as_millis(),
            best.state_digest()
        );
        if let Some(bus) = cross { bus.close(); }
        self.state = Some(best.state);
        Ok(best.answer)
    }
}

struct Active {
    session: u64,
    request: u64,
    digest: u64,
    job_id: u64,
    task: Task,
    safe: Candidate,
    best: Option<Candidate>,
    started: usize,
    completed: usize,
    began: Instant,
    deadline: Instant,
    cross: Option<Arc<CrossBus>>,
    elite_link: EliteLink,
}
impl Drop for Active {
    fn drop(&mut self) { if let Some(bus) = &self.cross { bus.close(); } }
}
struct Connection {
    socket: FramedSocket,
    authenticated: bool,
    last_request: Option<(u64, u64)>,
    last_activity: Instant,
}
pub fn daemon(options: Options) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(options.listen)?;
    listener.set_nonblocking(true)?;
    let mut pool = SearchPool::new(options.threads - 1, options.offset);
    let mut connection: Option<Connection> = None;
    let mut active: Option<Active> = None;
    let mut next_job = 0u64;
    eprintln!(
        "dist mode=worker listen={} total_threads={} search_threads={} offset={} pid={} cross_elite={}",
        options.listen,
        options.threads,
        options.threads - 1,
        options.offset,
        std::process::id(),
        options.cross_elite
    );
    loop {
        if connection.is_none() {
            match listener.accept() {
                Ok((socket, address)) => {
                    eprintln!("dist accepted_peer={address}");
                    connection = Some(Connection {
                        socket: FramedSocket::new(socket)?,
                        authenticated: false,
                        last_request: None,
                        last_activity: Instant::now(),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        let messages = if let Some(conn) = &mut connection {
            match conn.socket.poll::<ClientMessage>() {
                Ok(messages) => messages,
                Err(error) => {
                    eprintln!("dist worker connection_closed={error}");
                    connection = None;
                    active = None;
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        for message in messages {
            let Some(conn) = &mut connection else {
                break;
            };
            conn.last_activity = Instant::now();
            match message {
                ClientMessage::Elite(packet) => {
                    if conn.authenticated {
                        if let Some(work) = &active {
                            if let Some(bus) = &work.cross { bus.receive(packet, None); }
                        }
                    }
                }
                ClientMessage::Authenticate { version, token, cross_elite } => {
                    if conn.authenticated
                        || version != VERSION
                        || cross_elite != options.cross_elite
                        || !token_matches(&options.token, &token)
                    {
                        eprintln!("dist worker authentication_rejected");
                        connection = None;
                        active = None;
                        break;
                    }
                    conn.authenticated = true;
                    conn.socket.send(&ServerMessage::Ready {
                        version: VERSION,
                        total_threads: options.threads,
                        cross_elite: options.cross_elite,
                        offset: options.offset,
                    })?;
                }
                ClientMessage::Solve {
                    session,
                    request,
                    digest,
                    budget_ms,
                    task,
                } => {
                    if !conn.authenticated
                        || task.digest() != digest
                        || budget_ms > 600000
                        || conn.last_request.is_some_and(|(old_session, old_request)| {
                            old_session == session && request <= old_request
                        })
                        || task.problem.validate_players(task.problem.players).is_err()
                    {
                        eprintln!("dist worker request_rejected request={request}");
                        connection = None;
                        active = None;
                        break;
                    }
                    conn.last_request = Some((session, request));
                    next_job = next_job.wrapping_add(1);
                    let began = Instant::now();
                    let deadline = began + Duration::from_millis(budget_ms);
                    let safe = fallback(&task);
                    // Drop the old request bus before any new job can publish.
                    active = None;
                    let cross = (options.cross_elite && !task.is_select()).then(||
                        CrossBus::new(session, request, digest, options.offset, options.threads - 1, deadline));
                    let started = pool.begin_cross(next_job, &task, deadline, cross.clone());
                    eprintln!(
                        "dist worker request={request} job={next_job} phase={} input_digest={digest:016x} started={started} budget_ms={budget_ms}",
                        if task.is_select() { "select" } else { "plan" }
                    );
                    active = Some(Active {
                        session,
                        request,
                        digest,
                        job_id: next_job,
                        task,
                        safe,
                        best: None,
                        started,
                        completed: 0,
                        began,
                        deadline,
                        cross,
                        elite_link: EliteLink::new(),
                    });
                }
            }
        }
        if let Some(work) = &mut active {
            for candidate in pool.poll(work.job_id) {
                work.completed += 1;
                if work
                    .best
                    .as_ref()
                    .is_none_or(|best| candidate.better_than(best))
                    && candidate.valid_for(&work.task, &work.safe)
                {
                    work.best = Some(candidate);
                }
            }
            if Instant::now() >= work.deadline || pool.pending(work.job_id) == 0 {
                let mut work = active.take().unwrap();
                if let Some(conn) = &mut connection {
                    eprintln!(
                        "dist worker request={} completed={}/{} selected_worker={} elapsed_ms={} rx_bytes={} tx_bytes={}",
                        work.request,
                        work.completed,
                        work.started,
                        work.best.as_ref().map_or(usize::MAX, |best| best.worker),
                        work.began.elapsed().as_millis(),
                        conn.socket.received_bytes(),
                        conn.socket.sent_bytes()
                    );
                    conn.socket.send(&ServerMessage::Result {
                        session: work.session,
                        request: work.request,
                        digest: work.digest,
                        started: work.started,
                        completed: work.completed,
                        elapsed_ms: work.began.elapsed().as_millis() as u64,
                        candidate: work.best.take(),
                    })?;
                    conn.last_activity = Instant::now();
                }
            } else if let (Some(conn), Some(bus)) = (&mut connection, &work.cross) {
                work.elite_link.poll(&mut conn.socket, bus, None, false);
            }
        } else {
            let _ = pool.poll(u64::MAX);
        }
        if connection.as_ref().is_some_and(|conn| {
            conn.last_activity.elapsed()
                > Duration::from_secs(if conn.authenticated { 180 } else { 5 })
        }) && active.is_none()
        {
            eprintln!("dist worker idle_connection_expired");
            connection = None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
