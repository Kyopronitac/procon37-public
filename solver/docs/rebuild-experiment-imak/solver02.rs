// solver02: 再構築1。出典コミット: cbeaafdf756898ba6f1a0e48a17c51669b580741
// アルゴリズム・設定・対応表: solver/docs/rebuild-experiment-imak/README.md
// 詳細: solver/docs/rebuild-experiment-imak/solver02/algorithm.md
// 記録用の単一ファイル。開発用の main.rs・solver/*.rs は別途維持する。
// 通信・探索・テストを収録する。tester と serde_json は Cargo 依存。
// 元の外部モジュールをインライン化し、探索処理・定数・テストを保持している。

// 元ファイル: solver/mod.rs
mod solver {
    // 元ファイル: solver/paths.rs
    mod paths {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use tester::model::{Observation, Problem};

        pub const MODES: usize = 3;

        #[derive(Clone, Default)]
        pub struct Edge {
            pub time: i32,
            pub fuel: i32,
            pub spots: u32,
            pub moves: Vec<i64>,
        }

        pub struct Paths {
            pub nodes: Vec<usize>,
            pub edges: Vec<Edge>,
            pub spots: usize,
            pub brands: Vec<u64>,
            pub initial: Vec<usize>,
        }
        impl Paths {
            pub fn new(p: &Problem, state: &Observation) -> Self {
                let mut nodes: Vec<_> = p.spots.iter().map(|s| s.pos).collect();
                let mut initial = Vec::new();
                for a in &state.agents {
                    let idx = nodes.iter().position(|&x| x == a.pos).unwrap_or_else(|| {
                        nodes.push(a.pos);
                        nodes.len() - 1
                    });
                    initial.push(idx);
                }
                let size = p.map.len();
                let count = nodes.len();
                let mut traffic = vec![0; size];
                for t in &state.traffics {
                    traffic[t.pos] = t.status;
                }
                let mut durations = vec![0; size];
                let mut fuels = vec![0; size];
                let mut spot_at = vec![0u32; size];
                for (i, s) in p.spots.iter().enumerate() {
                    spot_at[s.pos] = 1u32 << i;
                }
                let mut neighbors = vec![Vec::new(); size];
                for pos in 0..size {
                    if p.map.terrain(pos) == 3 {
                        continue;
                    }
                    let (t, f) = match p.map.terrain(pos) {
                        0 => (2, 1),
                        1 => ([1, 2, 4][traffic[pos] as usize], 2),
                        _ => (3, 2),
                    };
                    durations[pos] = t;
                    fuels[pos] = f;
                    for dir in 0..6 {
                        if let Some(to) = p.map.neighbor(pos, dir)
                            && p.map.terrain(to) != 3
                        {
                            neighbors[pos].push((to, dir as i64));
                        }
                    }
                }
                let mut edges = vec![Edge::default(); MODES * count * count];
                for mode in 0..MODES {
                    for (source, &pos) in nodes.iter().enumerate() {
                        let mut dist = vec![i32::MAX; size];
                        let mut prev = vec![(usize::MAX, 0); size];
                        let mut heap = BinaryHeap::new();
                        dist[pos] = 0;
                        heap.push(Reverse((0, pos)));
                        while let Some(Reverse((d, x))) = heap.pop() {
                            if d != dist[x] {
                                continue;
                            }
                            // Fuel resolves equal travel times and provides alternate low-fuel routes.
                            let weight =
                                durations[x] * [1024, 16, 4][mode] + fuels[x] * [1, 8, 8][mode];
                            for &(y, dir) in &neighbors[x] {
                                let nd = d + weight;
                                if nd < dist[y] {
                                    dist[y] = nd;
                                    prev[y] = (x, dir);
                                    heap.push(Reverse((nd, y)));
                                }
                            }
                        }
                        for (target, &end) in nodes.iter().enumerate() {
                            let mut edge = Edge::default();
                            let mut x = end;
                            edge.spots |= spot_at[x];
                            while x != pos {
                                let (from, dir) = prev[x];
                                assert!(from != usize::MAX, "unreachable node");
                                edge.moves.push(dir);
                                edge.time += durations[from];
                                edge.fuel += fuels[from];
                                edge.spots |= spot_at[from];
                                x = from;
                            }
                            edge.moves.reverse();
                            edges[(mode * count + source) * count + target] = edge;
                        }
                    }
                }
                Self {
                    nodes,
                    edges,
                    spots: p.spots.len(),
                    brands: p.spots.iter().map(|s| 1u64 << s.brand).collect(),
                    initial,
                }
            }
            pub fn edge(&self, mode: usize, from: usize, to: usize) -> &Edge {
                &self.edges[(mode * self.nodes.len() + from) * self.nodes.len() + to]
            }
            pub fn brand_mask(&self, mut spots: u32) -> u64 {
                let mut mask = 0;
                while spots != 0 {
                    let i = spots.trailing_zeros() as usize;
                    spots &= spots - 1;
                    mask |= self.brands[i];
                }
                mask
            }
        }
    }

    use paths::{MODES, Paths};
    use std::time::{Duration, Instant};
    use tester::{
        engine::Game,
        model::{Observation, Plan, Problem},
    };

    const MAX_AGENTS: usize = 8;
    const MAX_MEETINGS: usize = 16;
    const MEETING: u8 = 64;

    pub struct Random(u64);
    impl Random {
        pub fn new(seed: u64) -> Self {
            Self(seed.wrapping_add(0x9e3779b97f4a7c15))
        }
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn index(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
        }
    }
    #[derive(Clone, Copy, Default)]
    struct Meeting {
        car: usize,
        supply: usize,
        node: usize,
        active: bool,
    }
    #[derive(Clone)]
    struct Candidate {
        routes: [Vec<u8>; MAX_AGENTS],
        meetings: [Meeting; MAX_MEETINGS],
        modes: [usize; MAX_AGENTS],
    }
    impl Candidate {
        fn empty() -> Self {
            Self {
                routes: std::array::from_fn(|_| Vec::new()),
                meetings: [Meeting::default(); MAX_MEETINGS],
                modes: [0; MAX_AGENTS],
            }
        }
        fn node(&self, stop: u8) -> usize {
            if stop < MEETING {
                stop as usize
            } else {
                self.meetings[(stop - MEETING) as usize].node
            }
        }
        fn remove_meeting(&mut self, id: usize) {
            let m = self.meetings[id];
            self.meetings[id].active = false;
            self.routes[m.car].retain(|&x| x != MEETING + id as u8);
            self.routes[m.supply].retain(|&x| x != MEETING + id as u8);
        }
    }
    struct Context<'a> {
        p: &'a Problem,
        state: &'a Observation,
        paths: Paths,
        cars: Vec<usize>,
        supplies: Vec<usize>,
        horizon: i32,
        collected: u64,
    }
    #[derive(Clone)]
    struct Evaluation {
        merit: f64,
        quality: f64,
        feasible: bool,
        brands: u64,
        times: [i32; MAX_AGENTS],
        fuel: [i32; MAX_AGENTS],
        end: [usize; MAX_AGENTS],
        visits: [u32; MAX_AGENTS],
        waits: [[i32; 2]; MAX_MEETINGS],
    }
    impl Context<'_> {
        fn evaluate(&self, c: &Candidate) -> Evaluation {
            let n = self.state.agents.len();
            let mut out = Evaluation {
                merit: -1e12,
                quality: -1e12,
                feasible: false,
                brands: 0,
                times: [0; MAX_AGENTS],
                fuel: [0; MAX_AGENTS],
                end: [0; MAX_AGENTS],
                visits: [0; MAX_AGENTS],
                waits: [[0; 2]; MAX_MEETINGS],
            };
            for i in 0..n {
                out.fuel[i] = self.state.agents[i].fuel as i32;
                out.end[i] = self.paths.initial[i];
                if self.state.agents[i].kind == 0 && out.end[i] < self.paths.spots {
                    out.visits[i] |= 1 << out.end[i];
                }
            }
            let mut cursors = [0; MAX_AGENTS];
            let mut processed = 0usize;
            let total: usize = c.routes[..n].iter().map(Vec::len).sum();
            let mut fuel_debt = 0;
            while processed < total {
                let mut progress = false;
                for i in 0..n {
                    if cursors[i] >= c.routes[i].len() {
                        continue;
                    }
                    let stop = c.routes[i][cursors[i]];
                    if stop < MEETING {
                        self.travel(c, &mut out, i, stop as usize);
                        cursors[i] += 1;
                        processed += 1;
                        progress = true;
                    } else {
                        let id = (stop - MEETING) as usize;
                        let m = c.meetings[id];
                        let other = if i == m.car { m.supply } else { m.car };
                        if cursors[other] >= c.routes[other].len()
                            || c.routes[other][cursors[other]] != stop
                        {
                            continue;
                        }
                        self.travel(c, &mut out, m.car, m.node);
                        self.travel(c, &mut out, m.supply, m.node);
                        let time = out.times[m.car].max(out.times[m.supply]).max(1);
                        out.waits[id] = [time - out.times[m.car], time - out.times[m.supply]];
                        out.times[m.car] = time;
                        out.times[m.supply] = time;
                        fuel_debt += (-out.fuel[m.car]).max(0);
                        out.fuel[m.car] = self.p.fuel_limits as i32;
                        cursors[i] += 1;
                        cursors[other] += 1;
                        processed += 2;
                        progress = true;
                    }
                }
                if !progress {
                    return out;
                }
            }
            let mut overtime = 0;
            for i in 0..n {
                overtime += (out.times[i] - self.horizon).max(0);
            }
            for &i in &self.cars {
                fuel_debt += (-out.fuel[i]).max(0);
            }
            let mut stocks = [0u32; 32];
            for &i in &self.cars {
                let mut mask = out.visits[i];
                out.brands |= self.paths.brand_mask(mask);
                while mask != 0 {
                    let s = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    stocks[s] += 1;
                }
            }
            let balls: u32 = self
                .p
                .spots
                .iter()
                .enumerate()
                .map(|(s, x)| stocks[s].min(x.stocks))
                .sum();
            let new = (out.brands & !self.collected).count_ones();
            let score =
                new as f64 * 2000.0 + out.brands.count_ones() as f64 * 1000.0 + balls as f64 * 4.0;
            let future = if self.state.day + 1 < self.p.day_steps.len() {
                self.cars
                    .iter()
                    .map(|&i| {
                        let f = out.fuel[i].max(0) as f64;
                        let near = self
                            .supplies
                            .iter()
                            .map(|&s| self.paths.edge(0, out.end[s], out.end[i]).time)
                            .min()
                            .unwrap_or(1000);
                        let reach = self
                            .supplies
                            .iter()
                            .map(|&s| self.paths.edge(1, out.end[i], out.end[s]).fuel)
                            .min()
                            .unwrap_or(1000);
                        60.0 * f / self.p.fuel_limits as f64
                            - 1.2 * near as f64
                            - ((reach - out.fuel[i]).max(0) as f64) * 2.0
                    })
                    .sum::<f64>()
            } else {
                0.0
            };
            out.quality = score + future - out.times[..n].iter().sum::<i32>() as f64 * 0.015;
            out.merit = out.quality - 180.0 * overtime as f64 - 180.0 * fuel_debt as f64;
            out.feasible = overtime == 0 && fuel_debt == 0;
            out
        }
        fn travel(&self, c: &Candidate, out: &mut Evaluation, i: usize, node: usize) {
            let edge = self.paths.edge(c.modes[i], out.end[i], node);
            out.times[i] += edge.time;
            if self.state.agents[i].kind == 0 {
                out.fuel[i] -= edge.fuel;
                out.visits[i] |= edge.spots;
            }
            out.end[i] = node;
        }
        fn materialize(&self, c: &Candidate, e: &Evaluation) -> Plan {
            (0..self.state.agents.len())
                .map(|i| {
                    let mut plan = Vec::new();
                    let mut node = self.paths.initial[i];
                    for &stop in &c.routes[i] {
                        let to = c.node(stop);
                        plan.extend_from_slice(&self.paths.edge(c.modes[i], node, to).moves);
                        node = to;
                        if stop >= MEETING {
                            let id = (stop - MEETING) as usize;
                            let side = usize::from(c.meetings[id].car != i);
                            if e.waits[id][side] > 0 {
                                plan.push(-i64::from(e.waits[id][side]));
                            }
                        }
                    }
                    if e.times[i] < self.horizon {
                        plan.push(-i64::from(self.horizon - e.times[i]));
                    }
                    plan
                })
                .collect()
        }
        fn mutate(&self, c: &mut Candidate, rng: &mut Random) {
            let car = self.cars[rng.index(self.cars.len())];
            let len = c.routes[car].len();
            match rng.index(16) {
                0..=3 => {
                    // Insert a store, or replace an existing one.
                    let node = rng.index(self.paths.spots) as u8;
                    if len < self.paths.spots + 8
                        && !c.routes[car].iter().any(|&s| c.node(s) == node as usize)
                    {
                        let at = rng.index(len + 1);
                        c.routes[car].insert(at, node);
                    }
                }
                4 | 5 => {
                    if len > 0 {
                        let at = rng.index(len);
                        let stop = c.routes[car][at];
                        if stop < MEETING {
                            c.routes[car].remove(at);
                        } else if rng.index(4) == 0 {
                            c.remove_meeting((stop - MEETING) as usize);
                        }
                    }
                }
                6 => {
                    if len > 1 {
                        let a = rng.index(len);
                        let b = rng.index(len);
                        c.routes[car].swap(a, b);
                    }
                }
                7 => {
                    if len > 1 {
                        let a = rng.index(len);
                        let b = rng.index(len);
                        c.routes[car][a.min(b)..=a.max(b)].reverse();
                    }
                }
                8 => {
                    if len > 0 {
                        let at = rng.index(len);
                        let stop = c.routes[car][at];
                        let other = self.cars[rng.index(self.cars.len())];
                        if stop < MEETING && c.routes[other].len() < self.paths.spots + 8 {
                            c.routes[car].remove(at);
                            let to = rng.index(c.routes[other].len() + 1);
                            c.routes[other].insert(to, stop);
                        }
                    }
                }
                9 => {
                    // Insert a rendezvous into both routes.
                    if self.supplies.is_empty() {
                        return;
                    }
                    if let Some(id) = c.meetings.iter().position(|m| !m.active) {
                        let supply = self.supplies[rng.index(self.supplies.len())];
                        let at = rng.index(len + 1);
                        let node = if at > 0 && rng.index(3) > 0 {
                            c.node(c.routes[car][at - 1])
                        } else {
                            rng.index(self.paths.nodes.len())
                        };
                        c.meetings[id] = Meeting {
                            car,
                            supply,
                            node,
                            active: true,
                        };
                        c.routes[car].insert(at, MEETING + id as u8);
                        let j = rng.index(c.routes[supply].len() + 1);
                        c.routes[supply].insert(j, MEETING + id as u8);
                    }
                }
                10 | 11 => {
                    let id = rng.index(MAX_MEETINGS);
                    let m = c.meetings[id];
                    if m.active {
                        let route = &c.routes[m.car];
                        let k = rng.index(route.len());
                        c.meetings[id].node = if rng.index(3) > 0 {
                            c.node(route[k])
                        } else {
                            rng.index(self.paths.nodes.len())
                        };
                    }
                }
                12 => {
                    let id = rng.index(MAX_MEETINGS);
                    let m = c.meetings[id];
                    if m.active {
                        let agent = if rng.index(2) == 0 { m.car } else { m.supply };
                        let at = c.routes[agent]
                            .iter()
                            .position(|&s| s == MEETING + id as u8)
                            .unwrap();
                        c.routes[agent].remove(at);
                        let j = rng.index(c.routes[agent].len() + 1);
                        c.routes[agent].insert(j, MEETING + id as u8);
                    }
                }
                13 => {
                    if len > 0 {
                        let at = rng.index(len);
                        if c.routes[car][at] < MEETING {
                            c.routes[car][at] = rng.index(self.paths.spots) as u8;
                        }
                    }
                }
                14 => {
                    c.modes[car] = rng.index(MODES);
                }
                _ => {
                    let id = rng.index(MAX_MEETINGS);
                    let m = c.meetings[id];
                    if m.active && !self.supplies.is_empty() {
                        let s = self.supplies[rng.index(self.supplies.len())];
                        c.routes[m.supply].retain(|&x| x != MEETING + id as u8);
                        c.meetings[id].supply = s;
                        let at = rng.index(c.routes[s].len() + 1);
                        c.routes[s].insert(at, MEETING + id as u8);
                    }
                }
            }
        }
    }
    fn context<'a>(p: &'a Problem, state: &'a Observation, collected: u64) -> Context<'a> {
        Context {
            p,
            state,
            paths: Paths::new(p, state),
            cars: state
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, a)| (a.kind == 0).then_some(i))
                .collect(),
            supplies: state
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, a)| (a.kind == 1).then_some(i))
                .collect(),
            horizon: p.day_steps[state.day] as i32,
            collected,
        }
    }
    #[derive(Default)]
    pub struct SearchMemory {
        warm: Option<Candidate>,
    }

    type Pool = Vec<(Candidate, Evaluation)>;
    fn add_pool(pool: &mut Pool, c: &Candidate, e: &Evaluation) {
        if !e.feasible {
            return;
        }
        if let Some(item) = pool
            .iter_mut()
            .find(|(_, x)| x.end == e.end && x.brands == e.brands)
        {
            if e.quality > item.1.quality {
                *item = (c.clone(), e.clone());
            }
            return;
        }
        pool.push((c.clone(), e.clone()));
        pool.sort_by(|a, b| b.1.quality.total_cmp(&a.1.quality));
        pool.truncate(20);
    }
    fn search(
        ctx: &Context<'_>,
        deadline: Instant,
        rng: &mut Random,
    ) -> (Candidate, Evaluation, u64) {
        let (c, e, it, _) = search_with(ctx, deadline, rng, None, false);
        (c, e, it)
    }
    fn search_with(
        ctx: &Context<'_>,
        deadline: Instant,
        rng: &mut Random,
        warm: Option<&Candidate>,
        archive: bool,
    ) -> (Candidate, Evaluation, u64, Pool) {
        let mut best = Candidate::empty();
        let mut best_eval = ctx.evaluate(&best);
        if let Some(c) = warm {
            let e = ctx.evaluate(c);
            if e.feasible && e.quality > best_eval.quality {
                best = c.clone();
                best_eval = e;
            }
        }
        let mut pool = Vec::new();
        let mut current = best.clone();
        let mut eval = best_eval.clone();
        let mut iterations = 0u64;
        let mut stale = 0;
        while Instant::now() < deadline {
            for _ in 0..128 {
                let mut next = current.clone();
                ctx.mutate(&mut next, rng);
                let ne = ctx.evaluate(&next);
                if ne.feasible && ne.quality > best_eval.quality {
                    best = next.clone();
                    best_eval = ne.clone();
                    stale = 0;
                    if archive {
                        add_pool(&mut pool, &best, &best_eval);
                    }
                }
                let phase = (iterations % 20000) as f64 / 20000.0;
                let temperature = 400.0 * (0.005f64).powf(phase);
                if ne.merit >= eval.merit
                    || rng.unit() < ((ne.merit - eval.merit) / temperature).exp()
                {
                    current = next;
                    eval = ne;
                }
                iterations += 1;
                stale += 1;
                if archive && iterations.is_multiple_of(2048) {
                    add_pool(&mut pool, &current, &eval);
                }
                if stale >= 40000 || iterations.is_multiple_of(20000) {
                    current = best.clone();
                    eval = best_eval.clone();
                    stale = 0;
                }
            }
        }
        if archive {
            add_pool(&mut pool, &best, &best_eval);
        }
        (best, best_eval, iterations, pool)
    }

    pub fn plan(
        p: &Problem,
        state: &Observation,
        game: &Game,
        collected: u64,
        deadline: Instant,
        rng: &mut Random,
        memory: &mut SearchMemory,
    ) -> (Plan, u64) {
        let ctx = context(p, state, collected);
        if ctx.cars.is_empty() {
            return (
                vec![vec![-i64::from(p.day_steps[state.day])]; state.agents.len()],
                0,
            );
        }
        let lookahead = state.day + 1 < p.day_steps.len()
            && std::env::var("SOLVER_LOOKAHEAD").is_ok_and(|x| x == "1");
        let start = Instant::now();
        let duration = deadline.saturating_duration_since(start);
        let explore_end = if lookahead {
            start + duration.mul_f64(0.4)
        } else {
            deadline
        };
        let (mut best, mut e, mut it, pool) =
            search_with(&ctx, explore_end, rng, memory.warm.as_ref(), lookahead);
        memory.warm = None;
        let mut best_rank = f64::NEG_INFINITY;
        let mut trials = 0;
        if lookahead {
            let depth = (p.day_steps.len() - state.day - 1).min(2);
            let slice = duration
                .mul_f64(0.6 / (pool.len().max(1) * depth) as f64)
                .min(Duration::from_millis(500));
            while Instant::now() < deadline {
                for (c, ce) in &pool {
                    if Instant::now() >= deadline {
                        break;
                    }
                    let mut sim = game.clone();
                    if sim.play_day(ctx.materialize(c, ce)).is_err() {
                        continue;
                    }
                    let mut all = collected | ce.brands;
                    let mut next_warm = None;
                    let mut last_quality = ce.quality;
                    let mut daily_value = (sim.score.daily_brands - game.score.daily_brands) as f64;
                    let mut ball_value = (sim.score.udon - game.score.udon) as f64;
                    let mut complete = true;
                    for d in 0..depth {
                        if Instant::now() >= deadline {
                            complete = false;
                            break;
                        }
                        let obs = sim.observation(0);
                        let next_ctx = context(p, &obs, all);
                        let (nc, ne, ni) =
                            search(&next_ctx, (Instant::now() + slice).min(deadline), rng);
                        it += ni;
                        let prior_daily = sim.score.daily_brands;
                        let prior_balls = sim.score.udon;
                        if sim.play_day(next_ctx.materialize(&nc, &ne)).is_err() {
                            complete = false;
                            break;
                        }
                        daily_value +=
                            (sim.score.daily_brands - prior_daily) as f64 * [0.9, 0.7][d];
                        ball_value += (sim.score.udon - prior_balls) as f64 * [0.2, 0.1][d];
                        all |= ne.brands;
                        last_quality = ne.quality;
                        if d == 0 {
                            next_warm = Some(nc);
                        }
                    }
                    if complete {
                        trials += 1;
                        let rank = sim.score.unique_brands as f64 * 1_000_000.0
                            + daily_value * 10000.0
                            + ball_value * 20.0
                            + (last_quality % 1000.0) * 0.1;
                        if rank > best_rank {
                            best_rank = rank;
                            best = c.clone();
                            e = ce.clone();
                            memory.warm = next_warm;
                        }
                    }
                }
            }
        }
        eprintln!(
            "day={} kinds={:?} brands={} merit={:.1} iterations={} lookahead={} times={:?} fuel={:?}",
            state.day,
            state.agents.iter().map(|a| a.kind).collect::<Vec<_>>(),
            e.brands.count_ones(),
            e.quality,
            it,
            trials,
            &e.times[..state.agents.len()],
            &e.fuel[..state.agents.len()]
        );
        (ctx.materialize(&best, &e), e.brands)
    }

    pub fn select(p: &Problem, deadline: Instant, rng: &mut Random) -> Vec<u8> {
        let n = p.agents.len();
        let mut masks: Vec<_> = (1u32..(1u32 << n))
            .filter(|m| m.count_ones() as usize <= n / 2)
            .collect();
        if masks.is_empty() {
            return vec![0; n];
        }
        // Randomize ties so equivalent initial positions do not select a fixed role.
        for i in (1..masks.len()).rev() {
            let j = rng.index(i + 1);
            masks.swap(i, j);
        }
        let mut results = Vec::new();
        let mut round = 0;
        let mut best_score = 0u64;
        let mut best_kinds = vec![0; n];
        best_kinds[n - 1] = 1;
        while Instant::now() < deadline {
            let list: Vec<u32> = if round == 0 {
                masks.clone()
            } else {
                results.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
                results.iter().take(6).map(|&(_, m)| m).collect()
            };
            if list.is_empty() {
                break;
            }
            for mask in list {
                if Instant::now() >= deadline {
                    break;
                }
                let kinds: Vec<u8> = (0..n).map(|i| ((mask >> i) & 1) as u8).collect();
                let mut game = Game::new(p.clone(), &kinds);
                let mut all = 0;
                let mut finished = true;
                for _ in 0..p.day_steps.len() {
                    if Instant::now() >= deadline {
                        finished = false;
                        break;
                    }
                    let state = game.observation(0);
                    let ctx = context(p, &state, all);
                    let slice = Duration::from_millis(if round == 0 { 25 } else { 100 });
                    let (c, e, _) = search(&ctx, (Instant::now() + slice).min(deadline), rng);
                    if let Err(error) = game.play_day(ctx.materialize(&c, &e)) {
                        eprintln!("selection simulation failed: {}", error.message);
                        finished = false;
                        break;
                    }
                    all |= e.brands;
                }
                if finished {
                    let score = game.score.unique_brands as u64 * 1_000_000
                        + game.score.daily_brands as u64 * 1000
                        + game.score.udon as u64;
                    results.push((score, mask));
                    if score > best_score {
                        best_score = score;
                        best_kinds = kinds;
                        eprintln!(
                            "select score={} kinds={:?} round={}",
                            score, best_kinds, round
                        );
                    }
                }
            }
            results.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
            let mut seen = std::collections::BTreeSet::new();
            results.retain(|&(_, mask)| seen.insert(mask));
            round += 1;
        }
        best_kinds
    }

    #[cfg(test)]
    // 元ファイル: solver/tests.rs
    mod tests {
        use super::*;
        use tester::generator::generate;

        #[test]
        fn synchronized_routes_are_accepted_by_independent_simulator() {
            for seed in 0..9 {
                let p = generate(seed, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                *kinds.last_mut().unwrap() = 1;
                let mut game = Game::new(p.clone(), &kinds);
                let mut rng = Random::new(seed);
                let mut collected = 0;
                for _ in 0..p.day_steps.len() {
                    let state = game.observation(0);
                    let ctx = context(&p, &state, collected);
                    let (c, e, _) =
                        search(&ctx, Instant::now() + Duration::from_millis(15), &mut rng);
                    let old_brands = game.score.daily_brands;
                    let replay = game
                        .play_day(ctx.materialize(&c, &e))
                        .unwrap_or_else(|err| {
                            panic!("seed={seed} day={} {}", state.day, err.message)
                        });
                    assert_eq!(
                        game.score.daily_brands - old_brands,
                        e.brands.count_ones() as usize,
                        "seed={seed}, day={}",
                        state.day
                    );
                    for (i, a) in replay.frames.last().unwrap().agents.iter().enumerate() {
                        assert_eq!(a.pos, ctx.paths.nodes[e.end[i]]);
                        if a.kind == 0 {
                            assert!(a.fuel as i32 >= e.fuel[i]);
                        }
                    }
                    collected |= e.brands;
                }
            }
        }
    }
}

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::{
    engine::Game,
    model::{Plan, Problem, Request, Response},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = std::env::args()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    let mut rng = solver::Random::new(seed);
    let mut problem: Option<Problem> = None;
    let mut game: Option<Game> = None;
    let mut pending: Option<Plan> = None;
    let mut memory = solver::SearchMemory::default();
    let mut collected = 0u64;
    let mut expected_brands = 0u64;
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let start = Instant::now();
        let req: Request = serde_json::from_str(&line)?;
        let override_ms = std::env::var("SOLVER_BUDGET_MS")
            .ok()
            .and_then(|x| x.parse::<u64>().ok());
        let budget = override_ms
            .unwrap_or(req.time_limit_ms.saturating_sub(1500))
            .min(req.time_limit_ms.saturating_sub(50))
            .max(1);
        let deadline = start + Duration::from_millis(budget);
        let answer = if req.phase == "select" {
            let p = req.problem.ok_or("missing problem")?;
            let kinds = solver::select(&p, deadline, &mut rng);
            game = Some(Game::new(p.clone(), &kinds));
            problem = Some(p);
            serde_json::to_value(kinds)?
        } else {
            if req.previous_accepted != Some(false) {
                collected |= expected_brands;
            }
            let p = problem.as_ref().ok_or("select must precede plan")?;
            let state = req.state.ok_or("missing state")?;
            let game = game.as_mut().ok_or("missing game")?;
            if let Some(previous) = pending.take() {
                let accepted = if req.previous_accepted == Some(false) {
                    game.waiting_plan()
                } else {
                    previous
                };
                game.play_day(accepted).map_err(|e| e.message)?;
            }
            if game.agents != state.agents || game.day != state.day {
                return Err("state differs from local simulation".into());
            }
            let (plan, brands) =
                solver::plan(p, &state, game, collected, deadline, &mut rng, &mut memory);
            pending = Some(plan.clone());
            expected_brands = brands;
            serde_json::to_value(plan)?
        };
        serde_json::to_writer(
            &mut stdout,
            &Response {
                request_id: req.request_id,
                answer,
            },
        )?;
        writeln!(stdout)?;
        stdout.flush()?;
        eprintln!(
            "phase={} request={} elapsed_ms={}",
            req.phase,
            req.request_id,
            start.elapsed().as_millis()
        );
    }
    Ok(())
}
