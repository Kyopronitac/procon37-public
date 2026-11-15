// 第3世代の保存用単一ファイル。元の分割ソースは solver03/ に保存。
// 保存元: solver03/rendezvous.rs
mod rendezvous {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    use std::time::{Duration, Instant};
    use tester::engine::Game;
    use tester::model::{Observation, Plan, Problem};

    const MODES: usize = 4;
    const STAGES: usize = 4;
    const INF: i32 = 100000;

    #[derive(Clone)]
    struct Random(u64);
    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn at(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn real(&mut self) -> f64 {
            (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
        }
    }

    #[derive(Clone, Default)]
    struct Leg {
        time: i32,
        fuel: i32,
        mask: u32,
        directions: Vec<i64>,
    }
    struct Network {
        nodes: Vec<usize>,
        meeting: Vec<i32>,
        meeting_fuels: usize,
        legs: Vec<Leg>,
        size: usize,
        spot_at: Vec<Option<usize>>,
    }
    impl Network {
        fn new(p: &Problem, obs: &Observation) -> Self {
            Self::with_nodes(p, obs, &[])
        }
        fn with_nodes(p: &Problem, obs: &Observation, extra: &[usize]) -> Self {
            let mut nodes: Vec<usize> = p.spots.iter().map(|s| s.pos).collect();
            nodes.extend(obs.agents.iter().map(|a| a.pos));
            for &cell in extra {
                if !nodes.contains(&cell) {
                    nodes.push(cell);
                }
            }
            // Add road and plain rendezvous cells spread across the map.
            // These let fuel delivery meet a route between shops.
            let stride = (p.map.width / 5).max(3);
            for y in (1..p.map.height).step_by(stride) {
                for x in (1..p.map.width).step_by(stride) {
                    let mut best = None;
                    for yy in y.saturating_sub(1)..=(y + 1).min(p.map.height - 1) {
                        for xx in x.saturating_sub(1)..=(x + 1).min(p.map.width - 1) {
                            let cell = yy * p.map.width + xx;
                            if p.map.terrain(cell) == 3 || nodes.contains(&cell) {
                                continue;
                            }
                            let merit = xx.abs_diff(x)
                                + yy.abs_diff(y)
                                + if p.map.terrain(cell) == 2 { 2 } else { 0 };
                            if best.is_none_or(|(v, _)| merit < v) {
                                best = Some((merit, cell));
                            }
                        }
                    }
                    if let Some((_, cell)) = best {
                        nodes.push(cell);
                    }
                }
            }
            let n = nodes.len();
            let cells = p.map.len();
            let mut traffic = vec![0; cells];
            for t in &obs.traffics {
                traffic[t.pos] = t.status;
            }
            let mut step = vec![0; cells];
            let mut fuel = vec![0; cells];
            let mut neighbors = vec![Vec::new(); cells];
            let mut spot_at = vec![None; cells];
            for (i, s) in p.spots.iter().enumerate() {
                spot_at[s.pos] = Some(i);
            }
            for i in 0..cells {
                (step[i], fuel[i]) = match p.map.terrain(i) {
                    0 => (2, 1),
                    1 => ([1, 2, 4][traffic[i] as usize], 2),
                    2 => (3, 2),
                    _ => (INF, INF),
                };
                for dir in 0..6 {
                    if let Some(q) = p.map.neighbor(i, dir) {
                        if p.map.terrain(q) != 3 {
                            neighbors[i].push((q, dir));
                        }
                    }
                }
            }
            let mut legs = vec![Leg::default(); MODES * n * n];
            for mode in 0..MODES {
                let fw = [0, 3, 10, 100][mode];
                for (src, &start) in nodes.iter().enumerate() {
                    let mut dist = vec![i32::MAX; cells];
                    let mut prev = vec![(usize::MAX, 0); cells];
                    let mut heap = BinaryHeap::new();
                    dist[start] = 0;
                    heap.push(Reverse((0, start)));
                    while let Some(Reverse((d, pos))) = heap.pop() {
                        if dist[pos] != d {
                            continue;
                        }
                        let cost = step[pos] * 10 + fuel[pos] * fw;
                        for &(q, dir) in &neighbors[pos] {
                            let nd = d + cost;
                            if nd < dist[q] {
                                dist[q] = nd;
                                prev[q] = (pos, dir);
                                heap.push(Reverse((nd, q)));
                            }
                        }
                    }
                    for (dst, &end) in nodes.iter().enumerate() {
                        let mut leg = Leg::default();
                        let mut pos = end;
                        while pos != start {
                            let (from, dir) = prev[pos];
                            if from == usize::MAX {
                                leg.time = INF;
                                leg.fuel = INF;
                                break;
                            }
                            leg.time += step[from];
                            leg.fuel += fuel[from];
                            if let Some(s) = spot_at[pos] {
                                leg.mask |= 1u32 << s;
                            }
                            leg.directions.push(dir as i64);
                            pos = from;
                        }
                        leg.directions.reverse();
                        legs[(mode * n + src) * n + dst] = leg;
                    }
                }
            }
            let meeting_fuels = p.fuel_limits as usize + 1;
            let mut meeting = vec![INF; n * n * meeting_fuels];
            for a in 0..n {
                for b in 0..n {
                    let off = (a * n + b) * meeting_fuels;
                    for dst in 0..n {
                        for mode in 0..MODES {
                            let patrol = &legs[(mode * n + a) * n + dst];
                            let supply = &legs[b * n + dst];
                            if patrol.fuel >= 0 && (patrol.fuel as usize) < meeting_fuels {
                                let idx = off + patrol.fuel as usize;
                                meeting[idx] = meeting[idx].min(patrol.time.max(supply.time));
                            }
                        }
                    }
                    for f in 1..meeting_fuels {
                        meeting[off + f] = meeting[off + f].min(meeting[off + f - 1]);
                    }
                }
            }
            Self {
                nodes,
                meeting,
                meeting_fuels,
                legs,
                size: n,
                spot_at,
            }
        }
        fn leg(&self, mode: usize, a: usize, b: usize) -> &Leg {
            &self.legs[(mode * self.size + a) * self.size + b]
        }
    }

    #[derive(Clone)]
    struct Solution {
        routes: Vec<Vec<usize>>,
        depots: [usize; STAGES],
        modes: Vec<[usize; STAGES + 1]>,
    }
    #[derive(Clone)]
    struct Evaluation {
        value: f64,
        feasible: bool,
        times: [i32; 8],
        fuels: [i32; 8],
        meetings: [i32; STAGES],
        car_meetings: [[i32; STAGES]; 8],
        score: (u32, u32, u32),
    }

    struct Context<'a> {
        p: &'a Problem,
        obs: &'a Observation,
        net: &'a Network,
        supply: usize,
        known: u32,
        steps: i32,
        future: f64,
    }
    impl Context<'_> {
        fn evaluate(&self, s: &Solution) -> Evaluation {
            let n = self.obs.agents.len();
            let m = self.p.spots.len();
            let mut times = [0; 8];
            let mut fuels = [0; 8];
            let mut ends = [0; 8];
            for i in 0..n {
                fuels[i] = self.obs.agents[i].fuel as i32;
                ends[i] = m + i;
            }
            let mut masks = [0u32; 8];
            let mut cursors = [0; 8];
            let mut meetings = [-1; STAGES];
            let mut car_meetings = [[-1; STAGES]; 8];
            let mut deficit = 0.;
            let mut supply_node = m + self.supply;
            let mut supply_time = 0;
            let mut mode_idx = [0; 8];
            for i in 0..n {
                if i != self.supply {
                    if let Some(k) = self.net.spot_at[self.obs.agents[i].pos] {
                        masks[i] |= 1u32 << k;
                    }
                }
            }
            for stage in 0..=STAGES {
                let mut users = Vec::new();
                for i in 0..n {
                    if i == self.supply {
                        continue;
                    }
                    let route = &s.routes[i];
                    while cursors[i] < route.len() {
                        let target = route[cursors[i]];
                        if target >= m && target - m != stage {
                            break;
                        }
                        let dest = if target >= m { s.depots[stage] } else { target };
                        let leg = self.net.leg(s.modes[i][mode_idx[i]], ends[i], dest);
                        times[i] += leg.time;
                        fuels[i] -= leg.fuel;
                        masks[i] |= leg.mask;
                        ends[i] = dest;
                        cursors[i] += 1;
                        if target >= m {
                            mode_idx[i] = stage + 1;
                            users.push(i);
                            break;
                        }
                    }
                    if stage == STAGES || users.last() == Some(&i) {
                        if fuels[i] < 0 {
                            deficit += (-fuels[i]) as f64;
                        }
                    }
                }
                if stage < STAGES && !users.is_empty() {
                    supply_time += self.net.leg(0, supply_node, s.depots[stage]).time;
                    supply_node = s.depots[stage];
                    let meet = users
                        .iter()
                        .map(|&i| times[i])
                        .max()
                        .unwrap()
                        .max(supply_time)
                        .max(1);
                    for &i in &users {
                        times[i] = times[i].max(supply_time).max(1);
                        car_meetings[i][stage] = times[i];
                        fuels[i] = self.p.fuel_limits as i32;
                    }
                    supply_time = meet;
                    meetings[stage] = meet;
                }
            }
            times[self.supply] = supply_time;
            ends[self.supply] = supply_node;
            let mut brands = 0u32;
            let mut count = [0u32; 32];
            for (i, &mask) in masks.iter().enumerate() {
                if i == self.supply {
                    continue;
                }
                for (j, c) in count.iter_mut().enumerate().take(m) {
                    if mask >> j & 1 != 0 {
                        *c += 1;
                    }
                }
            }
            let mut udon = 0;
            for (j, &count) in count.iter().enumerate().take(m) {
                if count > 0 {
                    brands |= 1u32 << self.p.spots[j].brand;
                    udon += count.min(self.p.spots[j].stocks);
                }
            }
            let new = (brands & !self.known).count_ones();
            let daily = brands.count_ones();
            let overtime: i32 = times.iter().map(|&x| (x - self.steps).max(0)).sum();
            let mut residual = 0.;
            for i in 0..n {
                if i == self.supply {
                    continue;
                }
                let fuel = fuels[i].max(0) as f64;
                let cap = self.steps as f64 * 0.7;
                residual += fuel.min(cap) * 0.018;
                // Low fuel is recoverable by tomorrow's mobile supplier, but takes time.
                let reach = self.net.leg(3, ends[i], supply_node).fuel as f64;
                residual -= (reach - fuel).max(0.) * 0.025;
                let rendezvous = self.net.meeting[(ends[i] * self.net.size + supply_node)
                    * self.net.meeting_fuels
                    + fuels[i].clamp(0, self.p.fuel_limits as i32) as usize]
                    as f64;
                let need = (1. - fuel / (self.steps as f64 * 0.7)).clamp(0., 1.);
                residual -= need
                    * (rendezvous * 0.25 + (rendezvous - self.steps as f64 * 0.75).max(0.) * 2.0);
                // Unused final steps are a weak tie breaker among equal collections.
            }
            // A soft coverage estimate keeps tomorrow's starting positions useful.
            // It includes the earliest independent fuel delivery for a depleted car.
            if self.future > 0. {
                let mut next_brand = [f64::INFINITY; 32];
                for (j, spot) in self.p.spots.iter().enumerate() {
                    let mut nearest = f64::INFINITY;
                    for i in 0..n {
                        if i == self.supply {
                            continue;
                        }
                        let mut duration = f64::INFINITY;
                        for mode in [0, 2, 3] {
                            let leg = self.net.leg(mode, ends[i], j);
                            if leg.fuel <= fuels[i] {
                                duration = duration.min(leg.time as f64);
                            }
                        }
                        if !duration.is_finite() {
                            let rv = self.net.meeting[(ends[i] * self.net.size + supply_node)
                                * self.net.meeting_fuels
                                + fuels[i].clamp(0, self.p.fuel_limits as i32) as usize]
                                as f64;
                            duration = rv
                                + 0.65
                                    * (self
                                        .net
                                        .leg(0, ends[i], j)
                                        .time
                                        .min(self.net.leg(0, supply_node, j).time)
                                        as f64);
                        }
                        nearest = nearest.min(duration);
                    }
                    next_brand[spot.brand] = next_brand[spot.brand].min(nearest);
                }
                for d in next_brand {
                    if d.is_finite() {
                        residual -= d * d / (self.steps as f64) * 0.10;
                    }
                }
            }
            let base = (new as f64) * 300. + (daily as f64) * 100. + udon as f64;
            let value = base + self.future * residual
                - (overtime as f64) * 24.
                - deficit * 28.
                - times.iter().map(|&t| t as f64).sum::<f64>() * 0.0005;
            Evaluation {
                value,
                feasible: overtime == 0 && deficit < 0.1,
                times,
                fuels,
                meetings,
                car_meetings,
                score: (new, daily, udon),
            }
        }
        fn emit(&self, s: &Solution, e: &Evaluation) -> Plan {
            let n = s.routes.len();
            let m = self.p.spots.len();
            let mut plans = vec![Vec::new(); n];
            for i in 0..n {
                let mut node = m + i;
                let mut time = 0;
                let mut mode_idx = 0;
                if i == self.supply {
                    for k in 0..STAGES {
                        if e.meetings[k] < 0 {
                            continue;
                        }
                        let leg = self.net.leg(0, node, s.depots[k]);
                        plans[i].extend(&leg.directions);
                        time += leg.time;
                        node = s.depots[k];
                        if e.meetings[k] > time {
                            plans[i].push(-i64::from(e.meetings[k] - time));
                            time = e.meetings[k];
                        }
                    }
                } else {
                    for &target in &s.routes[i] {
                        let dest = if target >= m {
                            s.depots[target - m]
                        } else {
                            target
                        };
                        let leg = self.net.leg(s.modes[i][mode_idx], node, dest);
                        plans[i].extend(&leg.directions);
                        time += leg.time;
                        node = dest;
                        if target >= m {
                            mode_idx = target - m + 1;
                        }
                        if target >= m && e.car_meetings[i][target - m] > time {
                            plans[i].push(-i64::from(e.car_meetings[i][target - m] - time));
                            time = e.car_meetings[i][target - m];
                        }
                    }
                }
                if time < self.steps {
                    plans[i].push(-i64::from(self.steps - time));
                }
            }
            plans
        }
    }

    #[derive(Clone)]
    struct RouteSeed {
        solution: Solution,
        depots: [usize; STAGES],
    }
    impl RouteSeed {
        fn capture(s: &Solution, net: &Network) -> Self {
            Self {
                solution: s.clone(),
                depots: s.depots.map(|i| net.nodes[i]),
            }
        }
        fn remap(&self, net: &Network) -> Solution {
            let mut s = self.solution.clone();
            for k in 0..STAGES {
                s.depots[k] = net.nodes.iter().position(|&p| p == self.depots[k]).unwrap();
            }
            s
        }
    }
    pub struct Solver {
        problem: Problem,
        supply: usize,
        random: Random,
        game: Game,
        pending: Option<Plan>,
        known: u32,
        previous: Option<RouteSeed>,
        hint: Option<RouteSeed>,
    }
    impl Solver {
        pub fn new(problem: Problem, seed: u64, limit: u64) -> Self {
            let selection_start = Instant::now();
            let mut best = (f64::MAX, 0);
            for (i, &a) in problem.agents.iter().enumerate() {
                let x = (a % problem.map.width) as f64;
                let y = (a / problem.map.width) as f64;
                let cost = problem
                    .agents
                    .iter()
                    .map(|&b| {
                        let dx = x - (b % problem.map.width) as f64;
                        let dy = y - (b / problem.map.width) as f64;
                        (dx * dx + dy * dy).sqrt()
                    })
                    .sum::<f64>();
                if cost < best.0 {
                    best = (cost, i);
                }
            }
            let supply = best.1;
            let mut kinds = vec![0; problem.agents.len()];
            kinds[supply] = 1;
            let game = Game::new(problem.clone(), &kinds);
            let mut this = Self {
                problem,
                supply,
                random: Random(seed.wrapping_add(0x8f371d963cbba401)),
                game,
                pending: None,
                known: 0,
                previous: None,
                hint: None,
            };
            let obs = this.game.observation(0);
            let net = Network::new(&this.problem, &obs);
            let n = obs.agents.len();
            let m = this.problem.spots.len();
            let select_lookahead = n <= 5 && limit >= 8000;
            let allotted = if select_lookahead {
                limit.saturating_mul(88) / 100 / n as u64
            } else {
                limit.saturating_sub(300).min(6000) / n as u64
            };
            let mut choice = None;
            for supplier in 0..n {
                if selection_start.elapsed().as_millis() as u64 > limit.saturating_sub(500) {
                    break;
                }
                let start = Instant::now();
                let ctx = Context {
                    p: &this.problem,
                    obs: &obs,
                    net: &net,
                    supply: supplier,
                    known: 0,
                    steps: this.problem.day_steps[0] as i32,
                    future: 1.,
                };
                let sol = Solution {
                    routes: vec![Vec::new(); n],
                    depots: [m + supplier; STAGES],
                    modes: vec![[0; STAGES + 1]; n],
                };
                let e = ctx.evaluate(&sol);
                let (s, e, _, _, _) = search(
                    &ctx,
                    &mut this.random,
                    sol.clone(),
                    sol,
                    e,
                    start,
                    Duration::from_millis(if select_lookahead {
                        allotted * 55 / 100
                    } else {
                        allotted
                    }),
                );
                let mut selection_value = e.value;
                if select_lookahead && this.problem.day_steps.len() > 1 {
                    let mut kinds = vec![0; n];
                    kinds[supplier] = 1;
                    let mut next_game = Game::new(this.problem.clone(), &kinds);
                    if let Ok(day) = next_game.play_day(ctx.emit(&s, &e)) {
                        let mut known = 0u32;
                        for frame in day.frames {
                            for event in frame.events {
                                if let tester::model::Event::Collect { brand, .. } = event {
                                    known |= 1 << brand;
                                }
                            }
                        }
                        let next_obs = next_game.observation(0);
                        let next_net = Network::with_nodes(&this.problem, &next_obs, &net.nodes);
                        let next_ctx = Context {
                            p: &this.problem,
                            obs: &next_obs,
                            net: &next_net,
                            supply: supplier,
                            known,
                            steps: this.problem.day_steps[1] as i32,
                            future: 1.,
                        };
                        let mut future_best = Solution {
                            routes: vec![Vec::new(); n],
                            depots: [m + supplier; STAGES],
                            modes: vec![[0; STAGES + 1]; n],
                        };
                        let mut future_eval = next_ctx.evaluate(&future_best);
                        let forward = RouteSeed::capture(&s, &net).remap(&next_net);
                        let mut reverse = forward.clone();
                        reverse.depots.reverse();
                        for route in &mut reverse.routes {
                            route.reverse();
                            for node in route {
                                if *node >= m {
                                    *node = m + STAGES - 1 - (*node - m);
                                }
                            }
                        }
                        for seed in [forward, reverse] {
                            let ev = next_ctx.evaluate(&seed);
                            if ev.feasible && ev.value > future_eval.value {
                                future_best = seed;
                                future_eval = ev;
                            }
                        }
                        let (_, ev, _, _, _) = search(
                            &next_ctx,
                            &mut this.random,
                            future_best.clone(),
                            future_best,
                            future_eval,
                            start,
                            Duration::from_millis(allotted),
                        );
                        let terminal = ev.value
                            - (ev.score.0 as f64 * 300.
                                + ev.score.1 as f64 * 100.
                                + ev.score.2 as f64);
                        selection_value = (e.score.0 + ev.score.0) as f64 * 5000.
                            + (e.score.1 + ev.score.1) as f64 * 100.
                            + e.score.2 as f64
                            + ev.score.2 as f64 * 0.95
                            + terminal * 0.5;
                        eprintln!(
                            "supply-candidate={} first={:?} second={:?} value={:.2}",
                            supplier, e.score, ev.score, selection_value
                        );
                    }
                }
                if choice.as_ref().is_none_or(
                    |(_, _, _, value): &(usize, Solution, Evaluation, f64)| {
                        selection_value > *value
                    },
                ) {
                    choice = Some((supplier, s, e, selection_value));
                }
            }
            if let Some((supply, solution, e, _)) = choice {
                this.supply = supply;
                this.previous = Some(RouteSeed::capture(&solution, &net));
                this.game = Game::new(this.problem.clone(), &this.kinds());
                eprintln!("selection supply={} projected={:?}", supply, e.score);
            }
            this
        }
        pub fn kinds(&self) -> Vec<u8> {
            let mut k = vec![0; self.problem.agents.len()];
            k[self.supply] = 1;
            k
        }
        pub fn plan(&mut self, obs: Observation, limit: u64, accepted: bool) -> Plan {
            let start = Instant::now();
            if let Some(plan) = self.pending.take() {
                let p = if accepted {
                    plan
                } else {
                    self.game.waiting_plan()
                };
                if let Ok(day) = self.game.play_day(p) {
                    for f in day.frames {
                        for e in f.events {
                            if let tester::model::Event::Collect { brand, .. } = e {
                                self.known |= 1u32 << brand;
                            }
                        }
                    }
                }
            }
            self.game.agents = obs.agents.clone();
            self.game.day = obs.day;
            self.game.set_observed_traffic(obs.traffics.clone());
            let mut extra = Vec::new();
            if let Some(seed) = &self.previous {
                extra.extend(seed.depots);
            }
            if let Some(seed) = &self.hint {
                extra.extend(seed.depots);
            }
            let net = Network::with_nodes(&self.problem, &obs, &extra);
            let steps = self.problem.day_steps[obs.day] as i32;
            let future = if obs.day + 1 == self.problem.day_steps.len() {
                0.
            } else {
                1.
            };
            let ctx = Context {
                p: &self.problem,
                obs: &obs,
                net: &net,
                supply: self.supply,
                known: self.known,
                steps,
                future,
            };
            let n = obs.agents.len();
            let m = self.problem.spots.len();
            let mut sol = Solution {
                routes: vec![Vec::new(); n],
                depots: [m + self.supply; STAGES],
                modes: vec![[0; STAGES + 1]; n],
            };
            let mut best = sol.clone();
            let mut best_eval = ctx.evaluate(&best);
            if let Some(prev) = &self.previous {
                let forward = prev.remap(&net);
                let mut reverse = forward.clone();
                reverse.depots.reverse();
                for route in &mut reverse.routes {
                    route.reverse();
                    for node in route {
                        if *node >= m {
                            *node = m + STAGES - 1 - (*node - m);
                        }
                    }
                }
                for seed in [&forward, &reverse] {
                    let e = ctx.evaluate(seed);
                    if e.feasible && e.value > best_eval.value {
                        best = seed.clone();
                        best_eval = e;
                    }
                }
                sol = best.clone();
            }
            if let Some(hint) = self.hint.take() {
                let seed = hint.remap(&net);
                let eval = ctx.evaluate(&seed);
                if eval.feasible && eval.value > best_eval.value {
                    best = seed.clone();
                    best_eval = eval.clone();
                }
                if eval.value > ctx.evaluate(&sol).value {
                    sol = seed;
                }
            }
            let max_ms = std::env::var("RENDEZVOUS_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or_else(|| limit.saturating_mul(9) / 10);
            let budget = Duration::from_millis(max_ms.min(limit.saturating_sub(250)).max(10));
            let lookahead = future > 0.
                && limit >= 8000
                && (n <= 5 || self.problem.fuel_limits <= self.problem.day_steps[0] * 13 / 10);
            let current_budget = if lookahead {
                budget.mul_f64(0.50)
            } else {
                budget
            };
            let (mut best, mut best_eval, iter, accepted_count, mut archive) = search(
                &ctx,
                &mut self.random,
                sol,
                best,
                best_eval,
                start,
                current_budget,
            );
            if lookahead {
                archive.push((best.clone(), best_eval.clone()));
                archive.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
                let raw = archive
                    .iter()
                    .filter(|(_, e)| {
                        e.score.0 == best_eval.score.0
                            && e.score.1 == best_eval.score.1
                            && e.score.2 > best_eval.score.2
                    })
                    .max_by(|a, b| {
                        a.1.score
                            .cmp(&b.1.score)
                            .then(a.1.value.total_cmp(&b.1.value))
                    })
                    .cloned();
                let lower = if n <= 5 {
                    archive
                        .iter()
                        .filter(|(_, e)| {
                            e.score.0 + 1 >= best_eval.score.0 && e.score.1 + 1 == best_eval.score.1
                        })
                        .max_by(|a, b| a.1.value.total_cmp(&b.1.value))
                        .cloned()
                } else {
                    None
                };
                archive.retain(|(_, e)| {
                    e.score.0 == best_eval.score.0
                        && e.score.1 == best_eval.score.1
                        && e.value >= best_eval.value - 10.
                });
                let mut chosen: Vec<(Solution, Evaluation)> = Vec::new();
                while !archive.is_empty() && chosen.len() < 6 {
                    let next = (0..archive.len())
                        .max_by(|&a, &b| {
                            let value = |i: usize| {
                                let key = endpoint_key(&archive[i].0, &archive[i].1, m);
                                let distance = chosen
                                    .iter()
                                    .map(|(s, e)| {
                                        let old = endpoint_key(s, e, m);
                                        key.iter()
                                            .zip(old.iter())
                                            .map(|(&(a, f), &(b, g))| {
                                                if a == b {
                                                    (f - g).abs() as f64 * 0.05
                                                } else {
                                                    1.
                                                }
                                            })
                                            .sum::<f64>()
                                    })
                                    .min_by(|a, b| a.total_cmp(b))
                                    .unwrap_or(0.);
                                archive[i].1.value + distance * 1.0
                            };
                            value(a).total_cmp(&value(b))
                        })
                        .unwrap();
                    let candidate = archive.swap_remove(next);
                    let key = endpoint_key(&candidate.0, &candidate.1, m);
                    if chosen.iter().all(|(s, e)| endpoint_key(s, e, m) != key) {
                        chosen.push(candidate);
                    }
                }
                if let Some(lower) = lower {
                    if chosen.len() >= 6 {
                        chosen.pop();
                    }
                    chosen.push(lower);
                }
                if let Some(raw) = raw {
                    if chosen.iter().all(|(s, e)| {
                        e.score != raw.1.score
                            || endpoint_key(s, e, m) != endpoint_key(&raw.0, &raw.1, m)
                    }) {
                        chosen.push(raw);
                    }
                }
                let mut horizon_best = f64::NEG_INFINITY;
                let count = chosen.len();
                for (idx, (candidate, eval)) in chosen.into_iter().enumerate() {
                    if start.elapsed() + Duration::from_millis(150) >= budget {
                        break;
                    }
                    let rollout_start = Instant::now();
                    let remaining = budget.saturating_sub(start.elapsed());
                    let slice = remaining / (count - idx) as u32;
                    let plan = ctx.emit(&candidate, &eval);
                    let mut future_game = self.game.clone();
                    let Ok(day) = future_game.play_day(plan) else {
                        continue;
                    };
                    let mut known = self.known;
                    for frame in day.frames {
                        for event in frame.events {
                            if let tester::model::Event::Collect { brand, .. } = event {
                                known |= 1 << brand;
                            }
                        }
                    }
                    let mut next_obs = future_game.observation(0);
                    let past = self.game.last_stays();
                    let present = future_game.last_stays();
                    for t in &mut next_obs.traffics {
                        let visits = present.get(t.pos).copied().unwrap_or(0)
                            + past.get(t.pos).copied().unwrap_or(0);
                        let observed = obs
                            .traffics
                            .iter()
                            .find(|old| old.pos == t.pos)
                            .map(|old| old.status)
                            .unwrap_or(0);
                        let forecast = if visits * 4 >= self.problem.jammed_threshold * 5 {
                            2
                        } else if visits * 4 >= self.problem.busy_threshold * 5 {
                            1
                        } else {
                            0
                        };
                        t.status = forecast.max(observed);
                    }
                    let next_net = Network::with_nodes(&self.problem, &next_obs, &net.nodes);
                    let future_context = Context {
                        p: &self.problem,
                        obs: &next_obs,
                        net: &next_net,
                        supply: self.supply,
                        known,
                        steps: self.problem.day_steps[obs.day + 1] as i32,
                        future: if obs.day + 2 < self.problem.day_steps.len() {
                            1.
                        } else {
                            0.
                        },
                    };
                    let mut next_best = Solution {
                        routes: vec![Vec::new(); n],
                        depots: [m + self.supply; STAGES],
                        modes: vec![[0; STAGES + 1]; n],
                    };
                    let mut next_eval = future_context.evaluate(&next_best);
                    let mut forward = candidate.clone();
                    for depot in &mut forward.depots {
                        *depot = next_net
                            .nodes
                            .iter()
                            .position(|&p| p == net.nodes[*depot])
                            .unwrap();
                    }
                    let mut reverse = forward.clone();
                    reverse.depots.reverse();
                    for route in &mut reverse.routes {
                        route.reverse();
                        for node in route {
                            if *node >= m {
                                *node = m + STAGES - 1 - (*node - m);
                            }
                        }
                    }
                    for seed in [forward, reverse] {
                        let e = future_context.evaluate(&seed);
                        if e.feasible && e.value > next_eval.value {
                            next_best = seed;
                            next_eval = e;
                        }
                    }
                    let (next_solution, next_eval, _, _, _) = search(
                        &future_context,
                        &mut self.random,
                        next_best.clone(),
                        next_best,
                        next_eval,
                        rollout_start,
                        slice,
                    );
                    let terminal = next_eval.value
                        - (next_eval.score.0 as f64 * 300.
                            + next_eval.score.1 as f64 * 100.
                            + next_eval.score.2 as f64);
                    let value = (eval.score.0 + next_eval.score.0) as f64 * 10000.
                        + eval.score.1 as f64 * 100.
                        + eval.score.2 as f64
                        + next_eval.score.1 as f64 * 100.
                        + next_eval.score.2 as f64 * 0.9
                        + terminal * 0.4;
                    eprintln!(
                        "rollout={} today={:?} tomorrow={:?} horizon={:.2}",
                        idx, eval.score, next_eval.score, value
                    );
                    if value > horizon_best {
                        horizon_best = value;
                        best = candidate;
                        best_eval = eval;
                        self.hint = Some(RouteSeed::capture(&next_solution, &next_net));
                    }
                }
            }
            let plan = ctx.emit(&best, &best_eval);
            let mut test = self.game.clone();
            let plan = match test.play_day(plan.clone()) {
                Ok(_) => plan,
                Err(e) => {
                    eprintln!("validation failed: {:?}", e);
                    self.game.waiting_plan()
                }
            };
            eprintln!(
                "day={} iterations={} accepted={} score={:?} fuel={:?} times={:?} supply={} meetings={:?}",
                obs.day,
                iter,
                accepted_count,
                best_eval.score,
                best_eval.fuels,
                best_eval.times,
                self.supply,
                best_eval.meetings
            );
            self.previous = Some(RouteSeed::capture(&best, &net));
            self.pending = Some(plan.clone());
            plan
        }
    }

    fn mutate(
        s: &mut Solution,
        rng: &mut Random,
        m: usize,
        n: usize,
        supply: usize,
        net: &Network,
    ) {
        let mut a = rng.at(n - 1);
        if a >= supply {
            a += 1;
        }
        if rng.at(12) == 0 {
            let k = rng.at(STAGES);
            let l = rng.at(STAGES);
            if k != l
                && !s
                    .routes
                    .iter()
                    .any(|route| route.contains(&(m + k)) && route.contains(&(m + l)))
            {
                let old = s.clone();
                s.depots.swap(k, l);
                for route in &mut s.routes {
                    for node in route {
                        if *node == m + k {
                            *node = m + l;
                        } else if *node == m + l {
                            *node = m + k;
                        }
                    }
                }
                if s.routes.iter().any(|route| !ordered(route, m)) {
                    *s = old;
                }
            }
            return;
        }
        match rng.at(100) {
            0..=26 => {
                let node = rng.at(m);
                let pos = insertion(s, a, node, rng, m, net);
                if !s.routes[a].contains(&node) {
                    s.routes[a].insert(pos, node);
                }
            }
            27..=32 => {
                let mut b = rng.at(n - 1);
                if b >= supply {
                    b += 1;
                }
                if a != b {
                    let q = rng.at(s.routes[a].len() + 1);
                    let z = rng.at(s.routes[b].len() + 1);
                    let ra = s.routes[a].clone();
                    let rb = s.routes[b].clone();
                    s.routes[a].truncate(q);
                    s.routes[a].extend(&rb[z..]);
                    s.routes[b].truncate(z);
                    s.routes[b].extend(&ra[q..]);
                    if !ordered(&s.routes[a], m) || !ordered(&s.routes[b], m) {
                        s.routes[a] = ra;
                        s.routes[b] = rb;
                    }
                }
            }
            33..=43 => {
                if !s.routes[a].is_empty() {
                    let q = rng.at(s.routes[a].len());
                    s.routes[a].remove(q);
                }
            }
            44..=58 => {
                if !s.routes[a].is_empty() {
                    let q = rng.at(s.routes[a].len());
                    let node = s.routes[a].remove(q);
                    let mut b = rng.at(n - 1);
                    if b >= supply {
                        b += 1;
                    }
                    if node >= m || s.routes[b].contains(&node) {
                        b = a;
                    }
                    let pos = if node < m {
                        insertion(s, b, node, rng, m, net)
                    } else {
                        rng.at(s.routes[b].len() + 1)
                    };
                    s.routes[b].insert(pos, node);
                    if !ordered(&s.routes[b], m) {
                        s.routes[b].remove(pos);
                        s.routes[a].insert(q, node);
                    }
                }
            }
            59..=68 => {
                if s.routes[a].len() > 1 {
                    let q = rng.at(s.routes[a].len());
                    let z = rng.at(s.routes[a].len());
                    s.routes[a][q.min(z)..=q.max(z)].reverse();
                    if !ordered(&s.routes[a], m) {
                        s.routes[a][q.min(z)..=q.max(z)].reverse();
                    }
                }
            }
            69..=75 => {
                let pos = rng.at(s.routes[a].len() + 1);
                let segment = s.routes[a][..pos]
                    .iter()
                    .filter(|&&v| v >= m)
                    .last()
                    .map(|&v| v - m + 1)
                    .unwrap_or(0);
                s.modes[a][segment] = rng.at(MODES);
            }
            76..=87 => {
                let k = rng.at(STAGES);
                let node = m + k;
                if let Some(at) = s.routes[a].iter().position(|&q| q == node) {
                    s.routes[a].remove(at);
                } else {
                    let lo = s.routes[a]
                        .iter()
                        .rposition(|&q| q >= m && q < node)
                        .map(|x| x + 1)
                        .unwrap_or(0);
                    let hi = s.routes[a]
                        .iter()
                        .position(|&q| q > node && q >= m)
                        .unwrap_or(s.routes[a].len());
                    let pos = lo + rng.at(hi - lo + 1);
                    s.routes[a].insert(pos, node);
                    if rng.at(2) == 0 {
                        s.depots[k] = if pos > 0 && s.routes[a][pos - 1] < m {
                            s.routes[a][pos - 1]
                        } else {
                            m + a
                        };
                    }
                }
            }
            88..=96 => {
                let k = rng.at(STAGES);
                let old = s.depots[k];
                if rng.at(3) == 0 {
                    s.depots[k] = rng.at(net.size);
                } else {
                    let node = rng.at(net.size);
                    if net.leg(0, old, node).time < 25 {
                        s.depots[k] = node;
                    }
                }
            }
            _ => {
                let mut b = rng.at(n - 1);
                if b >= supply {
                    b += 1;
                }
                if a != b && !s.routes[a].is_empty() && !s.routes[b].is_empty() {
                    let q = rng.at(s.routes[a].len());
                    let z = rng.at(s.routes[b].len());
                    let x = s.routes[a][q];
                    let y = s.routes[b][z];
                    if x < m && y < m && !s.routes[a].contains(&y) && !s.routes[b].contains(&x) {
                        s.routes[a][q] = y;
                        s.routes[b][z] = x;
                    }
                }
            }
        }
    }
    fn ordered(route: &[usize], m: usize) -> bool {
        let mut prev = None;
        for &a in route {
            if a >= m {
                if prev.is_some_and(|b| b >= a) {
                    return false;
                }
                prev = Some(a);
            }
        }
        true
    }

    fn search(
        ctx: &Context<'_>,
        random: &mut Random,
        mut sol: Solution,
        mut best: Solution,
        mut best_eval: Evaluation,
        start: Instant,
        budget: Duration,
    ) -> (Solution, Evaluation, u64, u64, Vec<(Solution, Evaluation)>) {
        let m = ctx.p.spots.len();
        let n = ctx.obs.agents.len();
        let mut current = ctx.evaluate(&sol);
        let mut iter = 0u64;
        let mut accepted_count = 0u64;
        let mut archive = vec![(best.clone(), best_eval.clone())];
        let mut levels: Vec<Option<(Solution, Evaluation)>> = vec![None; 33];
        let mut raw = (best.clone(), best_eval.clone());
        while start.elapsed() < budget {
            for _ in 0..2000 {
                iter += 1;
                let mut candidate = sol.clone();
                mutate(&mut candidate, random, m, n, ctx.supply, ctx.net);
                let eval = ctx.evaluate(&candidate);
                let progress = (start.elapsed().as_secs_f64() / budget.as_secs_f64()).min(1.);
                let phase = ((iter % 50000) as f64) / 50000.;
                let base: f64 = if (iter / 50000) % 7 == 0 { 45.0 } else { 3.0 };
                let temp = base * (0.02 / base).powf(phase) * (1.0 - progress * 0.7);
                let delta = eval.value - current.value;
                if delta >= 0. || random.real() < (delta / temp).exp() {
                    sol = candidate;
                    current = eval;
                    accepted_count += 1;
                    if current.feasible {
                        if current.score > raw.1.score
                            || (current.score == raw.1.score && current.value > raw.1.value)
                        {
                            raw = (sol.clone(), current.clone());
                        }
                        let level = current.score.1 as usize;
                        if levels[level]
                            .as_ref()
                            .is_none_or(|(_, e)| current.value > e.value)
                        {
                            levels[level] = Some((sol.clone(), current.clone()));
                        }
                    }
                    if current.feasible && current.value > best_eval.value {
                        best = sol.clone();
                        best_eval = current.clone();
                        remember(&mut archive, &sol, &current, m);
                    }
                }
                if iter % 50000 == 0 {
                    if current.feasible && current.value > best_eval.value - 10. {
                        remember(&mut archive, &sol, &current, m);
                    }
                    sol = best.clone();
                    current = best_eval.clone();
                }
            }
        }
        if best_eval.score.1 > 0 {
            if let Some(lower) = levels[best_eval.score.1 as usize - 1].take() {
                archive.push(lower);
            }
        }
        archive.push(raw);
        (best, best_eval, iter, accepted_count, archive)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cached_meetings_preserve_cells_when_agents_move() {
            let p = tester::generator::generate(0, None).unwrap().problem;
            let game = Game::new(p.clone(), &[0, 1, 0, 0]);
            let first = game.observation(0);
            let network = Network::new(&p, &first);
            let mut solution = Solution {
                routes: vec![Vec::new(); 4],
                depots: [0; STAGES],
                modes: vec![[0; STAGES + 1]; 4],
            };
            solution.depots = [
                p.spots.len() + 1,
                network.size - 1,
                network.size - 2,
                network.size - 3,
            ];
            let cache = RouteSeed::capture(&solution, &network);
            let mut second = first.clone();
            for (i, agent) in second.agents.iter_mut().enumerate() {
                agent.pos = p.spots[i].pos;
            }
            let changed = Network::with_nodes(&p, &second, &cache.depots);
            let restored = cache.remap(&changed);
            for k in 0..STAGES {
                assert_eq!(
                    network.nodes[solution.depots[k]],
                    changed.nodes[restored.depots[k]]
                );
            }
            assert_ne!(
                network.nodes[p.spots.len() + 1],
                changed.nodes[p.spots.len() + 1]
            );
        }
        #[test]
        fn mutations_keep_refill_stages_ordered() {
            let p = tester::generator::generate(0, None).unwrap().problem;
            let game = Game::new(p.clone(), &[0, 1, 0, 0]);
            let obs = game.observation(0);
            let net = Network::new(&p, &obs);
            let m = p.spots.len();
            let n = p.agents.len();
            let mut s = Solution {
                routes: vec![Vec::new(); n],
                depots: [m + 1; STAGES],
                modes: vec![[0; STAGES + 1]; n],
            };
            let mut random = Random(123456789);
            for _ in 0..100000 {
                mutate(&mut s, &mut random, m, n, 1, &net);
                assert!(s.routes.iter().all(|r| ordered(r, m)));
                assert!(s.routes.iter().flatten().all(|&v| v < m + STAGES));
                assert!(s.depots.iter().all(|&v| v < net.size));
            }
        }
        #[test]
        fn feasible_routes_match_public_engine() {
            for seed in [0, 3, 5] {
                let p = tester::generator::generate(seed, None).unwrap().problem;
                let n = p.agents.len();
                let m = p.spots.len();
                let supply = n - 1;
                let mut kinds = vec![0; n];
                kinds[supply] = 1;
                let game = Game::new(p.clone(), &kinds);
                let obs = game.observation(0);
                let net = Network::new(&p, &obs);
                let ctx = Context {
                    p: &p,
                    obs: &obs,
                    net: &net,
                    supply,
                    known: 0,
                    steps: p.day_steps[0] as i32,
                    future: 1.,
                };
                let mut s = Solution {
                    routes: vec![Vec::new(); n],
                    depots: [m + supply; STAGES],
                    modes: vec![[0; STAGES + 1]; n],
                };
                let mut random = Random(918273645 + seed);
                let mut verified = 0;
                for _ in 0..20000 {
                    let mut next = s.clone();
                    mutate(&mut next, &mut random, m, n, supply, &net);
                    let e = ctx.evaluate(&next);
                    if e.feasible {
                        let mut actual = game.clone();
                        actual.play_day(ctx.emit(&next, &e)).unwrap();
                        assert_eq!(actual.score.unique_brands as u32, e.score.0);
                        assert_eq!(actual.score.daily_brands as u32, e.score.1);
                        assert_eq!(actual.score.udon, e.score.2);
                        s = next;
                        verified += 1;
                    }
                }
                assert!(verified > 100);
            }
        }
    }

    fn insertion(
        s: &Solution,
        a: usize,
        node: usize,
        rng: &mut Random,
        m: usize,
        net: &Network,
    ) -> usize {
        if rng.at(3) == 0 {
            return rng.at(s.routes[a].len() + 1);
        }
        let mut from = m + a;
        let mut segment = 0;
        let mut best = (f64::INFINITY, 0);
        for pos in 0..=s.routes[a].len() {
            let mode = s.modes[a][segment];
            let leg = net.leg(mode, from, node);
            let mut cost = leg.time as f64 + leg.fuel as f64 * 0.15;
            if pos < s.routes[a].len() {
                let target = s.routes[a][pos];
                let dest = if target >= m {
                    s.depots[target - m]
                } else {
                    target
                };
                let next = net.leg(mode, node, dest);
                let old = net.leg(mode, from, dest);
                cost += (next.time - old.time) as f64 + (next.fuel - old.fuel) as f64 * 0.15;
                from = dest;
                if target >= m {
                    segment = target - m + 1;
                }
            }
            cost += rng.real() * 2.;
            if cost < best.0 {
                best = (cost, pos);
            }
        }
        best.1
    }

    fn endpoint_key(s: &Solution, e: &Evaluation, m: usize) -> Vec<(usize, i32)> {
        let mut result: Vec<_> = s
            .routes
            .iter()
            .enumerate()
            .map(|(i, route)| {
                let node = route
                    .last()
                    .map(|&q| if q >= m { s.depots[q - m] } else { q })
                    .unwrap_or(m + i);
                (node, e.fuels[i] / 8)
            })
            .collect();
        if let Some(stage) = e.meetings.iter().rposition(|&t| t >= 0) {
            result.push((s.depots[stage], 0));
        }
        result
    }
    fn remember(archive: &mut Vec<(Solution, Evaluation)>, s: &Solution, e: &Evaluation, m: usize) {
        let key = endpoint_key(s, e, m);
        if let Some(i) = archive
            .iter()
            .position(|(old, ev)| endpoint_key(old, ev, m) == key)
        {
            if e.value > archive[i].1.value {
                archive[i] = (s.clone(), e.clone());
            }
        } else if archive.len() < 24 {
            archive.push((s.clone(), e.clone()));
        } else {
            let low = (0..archive.len())
                .min_by(|&a, &b| archive[a].1.value.total_cmp(&archive[b].1.value))
                .unwrap();
            if e.value > archive[low].1.value {
                archive[low] = (s.clone(), e.clone());
            }
        }
    }
}

use std::io::{self, BufRead, Write};
use tester::model::{Problem, Request};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut solver: Option<rendezvous::Solver> = None;
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if req.phase == "select" {
            let p: Problem = req.problem.unwrap();
            let s = rendezvous::Solver::new(p, seed, req.time_limit_ms);
            let kinds = s.kinds();
            solver = Some(s);
            serde_json::json!(kinds)
        } else if let (Some(s), Some(obs)) = (solver.as_mut(), req.state) {
            serde_json::json!(s.plan(
                obs,
                req.time_limit_ms,
                req.previous_accepted.unwrap_or(true)
            ))
        } else {
            serde_json::json!([])
        };
        writeln!(
            out,
            "{}",
            serde_json::json!({"requestId":req.request_id,"answer":answer})
        )
        .unwrap();
        out.flush().unwrap();
    }
}
