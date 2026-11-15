use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    time::{Duration, Instant},
};
use tester::{engine::Game, model::*};

const INF: i32 = 100000;
const MODES: usize = 3;
#[derive(Clone, Default)]
struct Path {
    time: i32,
    fuel: i32,
    mask: u32,
    actions: Vec<i64>,
}
struct Network {
    paths: Vec<Path>,
    size: usize,
    starts: Vec<usize>,
    nearby: Vec<Vec<usize>>,
}
impl Network {
    fn new(game: &Game) -> Self {
        let p = &game.problem;
        let mut position: Vec<_> = p.spots.iter().map(|s| s.pos).collect();
        for &pos in &p.agents {
            if !position.contains(&pos) {
                position.push(pos);
            }
        }
        let spacing = if p.map.width <= 16 { 1 } else { 4 };
        for y in (0..p.map.height).step_by(spacing) {
            for x in (0..p.map.width).step_by(spacing) {
                let center = y * p.map.width + x;
                let cell = if p.map.terrain(center) != 3 {
                    Some(center)
                } else {
                    p.map.neighbors(center).find(|&q| p.map.terrain(q) != 3)
                };
                if let Some(cell) = cell {
                    if !position.contains(&cell) {
                        position.push(cell);
                    }
                }
            }
        }
        let starts: Vec<_> = game
            .agents
            .iter()
            .map(|a| {
                if let Some(i) = position.iter().position(|&x| x == a.pos) {
                    i
                } else {
                    position.push(a.pos);
                    position.len() - 1
                }
            })
            .collect();
        let size = position.len();
        let mut spot_at = vec![None; p.map.len()];
        for (i, s) in p.spots.iter().enumerate() {
            spot_at[s.pos] = Some(i);
        }
        let mut duration = vec![0; p.map.len()];
        let mut fuel = vec![0; p.map.len()];
        let mut statuses = vec![0; p.map.len()];
        for tr in game.traffic() {
            statuses[tr.pos] = tr.status as usize;
        }
        for cell in 0..p.map.len() {
            (duration[cell], fuel[cell]) = match p.map.terrain(cell) {
                0 => (2, 1),
                1 => ([1, 2, 4][statuses[cell]], 2),
                2 => (3, 2),
                _ => (INF, INF),
            };
        }
        let mut paths = vec![Path::default(); size * size * MODES];
        for (source, &start) in position.iter().enumerate() {
            for mode in 0..MODES {
                let mut distance = vec![INF; p.map.len()];
                let mut parent = vec![(usize::MAX, 0); p.map.len()];
                distance[start] = 0;
                let mut heap = BinaryHeap::from([(Reverse(0), start)]);
                while let Some((Reverse(d), pos)) = heap.pop() {
                    if distance[pos] != d {
                        continue;
                    }
                    let cost = duration[pos] * [12, 8, 3][mode] + fuel[pos] * [1, 5, 9][mode];
                    for dir in 0..6 {
                        if let Some(to) = p.map.neighbor(pos, dir) {
                            if p.map.terrain(to) == 3 {
                                continue;
                            }
                            if d + cost < distance[to] {
                                distance[to] = d + cost;
                                parent[to] = (pos, dir as i64);
                                heap.push((Reverse(d + cost), to));
                            }
                        }
                    }
                }
                for (target, &end) in position.iter().enumerate() {
                    let mut path = Path::default();
                    let mut pos = end;
                    if let Some(s) = spot_at[pos] {
                        path.mask |= 1u32 << s;
                    }
                    while pos != start {
                        let (from, dir) = parent[pos];
                        if from == usize::MAX {
                            path.time = INF;
                            path.fuel = INF;
                            break;
                        }
                        path.time += duration[from];
                        path.fuel += fuel[from];
                        path.actions.push(dir);
                        pos = from;
                        if let Some(s) = spot_at[pos] {
                            path.mask |= 1u32 << s;
                        }
                    }
                    path.actions.reverse();
                    paths[(source * size + target) * MODES + mode] = path;
                }
            }
        }
        let nearby = (0..size)
            .map(|from| {
                let mut nodes: Vec<_> = (0..size).filter(|&to| to != from).collect();
                nodes.sort_by_key(|&to| paths[(from * size + to) * MODES].time);
                nodes.truncate(12);
                nodes
            })
            .collect();
        Self {
            paths,
            size,
            starts,
            nearby,
        }
    }
    fn path(&self, a: usize, b: usize, mode: usize) -> &Path {
        &self.paths[(a * self.size + b) * MODES + mode]
    }
}
#[derive(Clone)]
struct Route {
    nodes: Vec<usize>,
    modes: Vec<usize>,
    refill: Option<usize>,
    refill_node: Option<usize>,
    refill_mode: usize,
}
#[derive(Clone)]
struct Solution {
    routes: Vec<Route>,
    orders: Vec<Vec<usize>>,
}
#[derive(Clone, Default)]
struct RouteStats {
    prefix_time: i32,
    suffix_time: i32,
    prefix_fuel: i32,
    suffix_fuel: i32,
    meeting: usize,
    end: usize,
    mask: u32,
}
#[derive(Clone)]
struct Evaluation {
    value: f64,
    valid: bool,
    delays: Vec<i32>,
    stats: Vec<RouteStats>,
    score: i64,
    end_positions: Vec<usize>,
    end_fuels: Vec<i32>,
    brands: u64,
    excess: i32,
}
struct Context<'a> {
    game: &'a Game,
    net: &'a Network,
    patrols: Vec<usize>,
    supplies: Vec<usize>,
    seen: u64,
    remaining: usize,
}
impl Context<'_> {
    fn stats(&self, index: usize, route: &Route, starts: &[usize]) -> RouteStats {
        let agent = self.patrols[index];
        let start = starts[agent];
        let mut st = RouteStats {
            meeting: start,
            end: start,
            ..RouteStats::default()
        };
        if start < self.game.problem.spots.len() {
            st.mask |= 1u32 << start;
        }
        let mut pos = start;
        for i in 0..=route.nodes.len() {
            if route.refill == Some(i) {
                if let Some(to) = route.refill_node {
                    let path = self.net.path(pos, to, route.refill_mode);
                    st.prefix_time += path.time;
                    st.prefix_fuel += path.fuel;
                    st.mask |= path.mask;
                    pos = to;
                }
                st.meeting = pos;
            }
            if i == route.nodes.len() {
                break;
            }
            let to = route.nodes[i];
            let path = self.net.path(pos, to, route.modes[i]);
            st.mask |= path.mask;
            if route.refill.is_some_and(|r| i >= r) {
                st.suffix_time += path.time;
                st.suffix_fuel += path.fuel;
            } else {
                st.prefix_time += path.time;
                st.prefix_fuel += path.fuel;
            }
            pos = to;
        }
        st.end = pos;
        st
    }
    fn evaluate(&self, sol: &Solution, starts: &[usize], fuels: &[i32], day: usize) -> Evaluation {
        let stats: Vec<_> = sol
            .routes
            .iter()
            .enumerate()
            .map(|(a, r)| self.stats(a, r, starts))
            .collect();
        let mut end_positions = starts.to_vec();
        let mut end_fuels = fuels.to_vec();
        let mut delays = vec![0; stats.len()];
        let mut excess = 0;
        for (s, order) in sol.orders.iter().enumerate() {
            let mut pos = starts[self.supplies[s]];
            let mut time = 0;
            for &a in order {
                if sol.routes[a].refill.is_none() {
                    continue;
                }
                let st = &stats[a];
                time += self.net.path(pos, st.meeting, 0).time;
                time = time.max(st.prefix_time).max(1);
                delays[a] = time - st.prefix_time;
                pos = st.meeting;
            }
            end_positions[self.supplies[s]] = pos;
            excess += (time - self.game.problem.day_steps[day] as i32).max(0);
        }
        let limit = self.game.problem.day_steps[day] as i32;
        let capacity = self.game.problem.fuel_limits as i32;
        let mut counts = [0usize; 32];
        let mut fuelbonus = 0.0;
        let mut unused = 0;
        let mut endpoint_brands = 0u64;
        for (a, st) in stats.iter().enumerate() {
            let fuel = fuels[self.patrols[a]];
            let mut rem = fuel - st.prefix_fuel;
            excess += (-rem).max(0);
            if sol.routes[a].refill.is_some() {
                rem = capacity - st.suffix_fuel;
                excess += (-rem).max(0);
            }
            end_positions[self.patrols[a]] = st.end;
            end_fuels[self.patrols[a]] = rem.max(0);
            let time = st.prefix_time + st.suffix_time + delays[a];
            excess += (time - limit).max(0);
            unused += (limit - time).max(0);
            if day + 1 < self.game.problem.day_steps.len() {
                fuelbonus += 0.015 * rem.max(0) as f64;
                if !self.supplies.is_empty() {
                    let reach = self
                        .supplies
                        .iter()
                        .map(|&s| self.net.path(st.end, end_positions[s], 2).fuel)
                        .min()
                        .unwrap();
                    let reserve = (reach as f64 * 0.60 + 2.0).min(capacity as f64 * 0.45);
                    fuelbonus -= (reserve - rem.max(0) as f64).max(0.0) * 22.0;
                }
                if st.end < self.game.problem.spots.len() {
                    endpoint_brands |= 1u64 << self.game.problem.spots[st.end].brand;
                }
            }
            let mut mask = st.mask;
            while mask != 0 {
                let s = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                counts[s] += 1;
            }
        }
        let mut brands = 0u64;
        let mut udon = 0;
        for (s, &count) in counts
            .iter()
            .enumerate()
            .take(self.game.problem.spots.len())
        {
            if count > 0 {
                brands |= 1u64 << self.game.problem.spots[s].brand;
            }
            udon += count.min(self.game.problem.spots[s].stocks as usize);
        }
        let new = (brands & !self.seen).count_ones() as i64;
        let daily = brands.count_ones() as i64;
        let score = new * 100000 + daily * 1000 + udon as i64 * 10;
        let objective = new as f64 * 6000.0 + daily as f64 * 1000.0 + udon as f64 * 10.0;
        Evaluation {
            value: objective - excess as f64 * 1200.0
                + fuelbonus
                + endpoint_brands.count_ones() as f64 * 0.02
                + unused as f64 * 0.001,
            valid: excess == 0,
            delays,
            stats,
            score,
            end_positions,
            end_fuels,
            brands,
            excess,
        }
    }
    fn compile(&self, sol: &Solution, ev: &Evaluation) -> Plan {
        let steps = self.game.problem.day_steps[self.game.day] as i32;
        let mut plan = vec![Vec::new(); self.game.agents.len()];
        for (a, route) in sol.routes.iter().enumerate() {
            let agent = self.patrols[a];
            let mut pos = self.net.starts[agent];
            let mut time = 0;
            for i in 0..=route.nodes.len() {
                if route.refill == Some(i) {
                    if let Some(to) = route.refill_node {
                        let path = self.net.path(pos, to, route.refill_mode);
                        plan[agent].extend_from_slice(&path.actions);
                        time += path.time;
                        pos = to;
                    }
                    if ev.delays[a] > 0 {
                        plan[agent].push(-(ev.delays[a] as i64));
                        time += ev.delays[a];
                    }
                }
                if i == route.nodes.len() {
                    break;
                }
                let to = route.nodes[i];
                let path = self.net.path(pos, to, route.modes[i]);
                plan[agent].extend_from_slice(&path.actions);
                time += path.time;
                pos = to;
            }
            if time < steps {
                plan[agent].push(-((steps - time) as i64));
            }
        }
        for (s, order) in sol.orders.iter().enumerate() {
            let agent = self.supplies[s];
            let mut pos = self.net.starts[agent];
            let mut time = 0;
            for &a in order {
                if sol.routes[a].refill.is_none() {
                    continue;
                }
                let to = ev.stats[a].meeting;
                let path = self.net.path(pos, to, 0);
                plan[agent].extend_from_slice(&path.actions);
                time += path.time;
                let meet = ev.stats[a].prefix_time + ev.delays[a];
                if meet > time {
                    plan[agent].push(-((meet - time) as i64));
                    time = meet;
                }
                pos = to;
            }
            if time < steps {
                plan[agent].push(-((steps - time) as i64));
            }
        }
        plan
    }
}
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}
pub struct Optimizer {
    rng: Rng,
    carry: Option<Vec<Solution>>,
    initial: Option<Vec<Solution>>,
    planning_depth: usize,
}
impl Optimizer {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed.wrapping_add(913481414387)),
            carry: None,
            initial: None,
            planning_depth: 1,
        }
    }
    fn solve(&mut self, game: &Game, seen: u64, seconds: f64) -> Plan {
        let begun = Instant::now();
        let net = Network::new(game);
        let patrols: Vec<_> = game
            .agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind == 0)
            .map(|(i, _)| i)
            .collect();
        let supplies: Vec<_> = game
            .agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind == 1)
            .map(|(i, _)| i)
            .collect();
        let ctx = Context {
            game,
            net: &net,
            patrols,
            supplies,
            seen,
            remaining: game.problem.day_steps.len() - game.day,
        };
        let n = ctx.patrols.len();
        let mut current = Solution {
            routes: vec![
                Route {
                    nodes: vec![],
                    modes: vec![],
                    refill: None,
                    refill_node: None,
                    refill_mode: 0,
                };
                n
            ],
            orders: vec![Vec::new(); ctx.supplies.len()],
        };
        if !ctx.supplies.is_empty() {
            for a in 0..n {
                current.orders[a % ctx.supplies.len()].push(a);
            }
        }
        let horizon = ctx.remaining.min(self.planning_depth);
        let mut horizon_current = vec![current.clone(); horizon];
        let empty = horizon_current.clone();
        if let Some(mut previous) = self.carry.take() {
            if previous.len() > 1 {
                previous.remove(0);
                for (day, solution) in previous.into_iter().enumerate().take(horizon) {
                    horizon_current[day] = solution;
                }
            } else if let Some(mut solution) = previous.pop() {
                for route in &mut solution.routes {
                    route.nodes.reverse();
                    route.modes.reverse();
                    if let Some(r) = route.refill {
                        route.refill = Some(route.nodes.len() - r);
                    }
                }
                for order in &mut solution.orders {
                    order.reverse();
                }
                horizon_current[0] = solution;
            }
        }
        if let Some(initial) = self.initial.take() {
            for (day, solution) in initial.into_iter().enumerate().take(horizon) {
                horizon_current[day] = solution;
            }
        }
        let starts = net.starts.clone();
        let fuels: Vec<_> = game.agents.iter().map(|a| a.fuel as i32).collect();
        let assess = |solutions: &[Solution]| -> (f64, bool, Vec<Evaluation>) {
            let mut positions = starts.clone();
            let mut fuel = fuels.clone();
            let mut evaluations = Vec::new();
            let mut value = 0.0;
            let mut all_brands = seen;
            let mut valid = true;
            for (day, solution) in solutions.iter().enumerate() {
                let ev = ctx.evaluate(solution, &positions, &fuel, game.day + day);
                let newly = (ev.brands & !all_brands).count_ones();
                all_brands |= ev.brands;
                let instant = ev.brands.count_ones() as f64 * 1000.0
                    + (ev.score
                        - ((ev.brands & !seen).count_ones() as i64) * 100000
                        - (ev.brands.count_ones() as i64) * 1000) as f64;
                value += instant * (if day == 0 { 1.0 } else { 0.97 }) + newly as f64 * 100000.0
                    - ev.excess as f64 * 2000.0;
                if day + 1 == solutions.len() {
                    value += (ev.value
                        - (ev.brands & !seen).count_ones() as f64 * 6000.0
                        - ev.brands.count_ones() as f64 * 1000.0
                        - (ev.score
                            - ((ev.brands & !seen).count_ones() as i64) * 100000
                            - (ev.brands.count_ones() as i64) * 1000)
                            as f64
                        + ev.excess as f64 * 1200.0)
                        * 0.8;
                }
                positions = ev.end_positions.clone();
                fuel = ev.end_fuels.clone();
                valid &= ev.valid;
                evaluations.push(ev);
            }
            (value, valid, evaluations)
        };
        let exploration = |value: f64, evaluations: &[Evaluation]| {
            let brands = evaluations.iter().fold(0u64, |mask, e| mask | e.brands);
            value - (brands & !seen).count_ones() as f64 * 97000.0
        };
        let (initial_value, _, initial_evaluations) = assess(&horizon_current);
        let mut current_value = exploration(initial_value, &initial_evaluations);
        let mut best = empty.clone();
        let (mut best_value, _, mut best_evaluations) = assess(&best);
        let (warm_value, warm_valid, warm_evaluations) = assess(&horizon_current);
        if warm_valid && warm_value > best_value {
            best = horizon_current.clone();
            best_value = warm_value;
            best_evaluations = warm_evaluations;
        }
        let mut iter = 0usize;
        let mut temperature = 180.0;
        let mut restarts = 0;
        let mut epoch = Instant::now();
        let cycle = (seconds * 0.8).clamp(0.015, 2.5);
        let limit = Duration::from_secs_f64(seconds.max(0.01));
        while begun.elapsed() < limit {
            iter += 1;
            if iter % 256 == 0 {
                let phase = (epoch.elapsed().as_secs_f64() / cycle).min(1.0);
                temperature = 450.0 * (0.001f64).powf(phase);
                if phase >= 1.0 {
                    horizon_current = if restarts % 2 == 0 {
                        empty.clone()
                    } else {
                        best.clone()
                    };
                    let (value, _, evaluations) = assess(&horizon_current);
                    current_value = exploration(value, &evaluations);
                    epoch = Instant::now();
                    restarts += 1;
                }
            }
            let mut next = horizon_current.clone();
            let day = self.rng.below(horizon);
            self.mutate(
                &mut next[day],
                game.problem.spots.len(),
                ctx.supplies.len(),
                &net,
            );
            let (value, valid, evaluations) = assess(&next);
            let search_value = exploration(value, &evaluations);
            if valid && value > best_value {
                best = next.clone();
                best_value = value;
                best_evaluations = evaluations;
            }
            if search_value >= current_value
                || self.rng.unit() < ((search_value - current_value) / temperature).exp()
            {
                horizon_current = next;
                current_value = search_value;
            }
        }
        eprintln!(
            "day={} horizon={} iterations={} cycles={} estimated={} objective={:.2}",
            game.day, horizon, iter, restarts, best_evaluations[0].score, best_value
        );
        let plan = ctx.compile(&best[0], &best_evaluations[0]);
        self.carry = Some(best);
        plan
    }
    fn mutate(&mut self, sol: &mut Solution, spots: usize, suppliers: usize, network: &Network) {
        let nodes = network.size;
        let n = sol.routes.len();
        if n == 0 {
            return;
        }
        let a = self.rng.below(n);
        let len = sol.routes[a].nodes.len();
        match self.rng.below(19) {
            0 | 1 | 2 => {
                if len >= spots + 3 {
                    return;
                }
                let i = self.rng.below(len + 1);
                let to = self.rng.below(spots);
                if sol.routes[a].nodes.contains(&to) {
                    return;
                }
                sol.routes[a].nodes.insert(i, to);
                sol.routes[a].modes.insert(i, 0);
                if let Some(r) = sol.routes[a].refill {
                    if i < r {
                        sol.routes[a].refill = Some(r + 1);
                    }
                }
            }
            3 => {
                if len == 0 {
                    return;
                }
                let i = self.rng.below(len);
                sol.routes[a].nodes.remove(i);
                sol.routes[a].modes.remove(i);
                if let Some(r) = sol.routes[a].refill {
                    if i < r {
                        sol.routes[a].refill = Some(r - 1);
                    }
                }
            }
            4 => {
                if len == 0 {
                    return;
                }
                let i = self.rng.below(len);
                let to = self.rng.below(spots);
                if sol.routes[a].nodes.contains(&to) {
                    return;
                }
                sol.routes[a].nodes[i] = to;
            }
            5 => {
                if len < 2 {
                    return;
                }
                let i = self.rng.below(len);
                let j = self.rng.below(len);
                sol.routes[a].nodes.swap(i, j);
                sol.routes[a].modes.swap(i, j);
            }
            6 => {
                if len < 2 {
                    return;
                }
                let i = self.rng.below(len);
                let j = self.rng.below(len);
                sol.routes[a].nodes[i.min(j)..=i.max(j)].reverse();
            }
            7 => {
                if suppliers == 0 {
                    return;
                }
                let r = self.rng.below(len + 2);
                sol.routes[a].refill = if r <= len { Some(r) } else { None };
            }
            8 => {
                if suppliers > 0 && self.rng.below(3) == 0 {
                    sol.routes[a].refill_mode = self.rng.below(MODES);
                    return;
                }
                if len == 0 {
                    return;
                }
                let i = self.rng.below(len);
                sol.routes[a].modes[i] = self.rng.below(MODES);
            }
            9 | 10 => {
                let b = self.rng.below(n);
                if a == b || len == 0 {
                    return;
                }
                let i = self.rng.below(len);
                let to = sol.routes[a].nodes[i];
                if sol.routes[b].nodes.contains(&to) {
                    return;
                }
                let j = self.rng.below(sol.routes[b].nodes.len() + 1);
                sol.routes[a].nodes.remove(i);
                sol.routes[a].modes.remove(i);
                if let Some(r) = sol.routes[a].refill {
                    if i < r {
                        sol.routes[a].refill = Some(r - 1);
                    }
                }
                sol.routes[b].nodes.insert(j, to);
                sol.routes[b].modes.insert(j, 0);
                if let Some(r) = sol.routes[b].refill {
                    if j < r {
                        sol.routes[b].refill = Some(r + 1);
                    }
                }
            }
            11 | 12 => {
                let b = self.rng.below(n);
                let blen = sol.routes[b].nodes.len();
                if a == b || len == 0 || blen == 0 {
                    return;
                }
                let i = self.rng.below(len);
                let j = self.rng.below(blen);
                let x = sol.routes[a].nodes[i];
                let y = sol.routes[b].nodes[j];
                if sol.routes[a].nodes.contains(&y) || sol.routes[b].nodes.contains(&x) {
                    return;
                }
                sol.routes[a].nodes[i] = y;
                sol.routes[b].nodes[j] = x;
            }
            14 | 15 => {
                if suppliers == 0 {
                    return;
                }
                let x = self.rng.below(nodes + nodes / 2);
                if self.rng.below(3) != 0 {
                    if let Some(old) = sol.routes[a].refill_node {
                        let near = &network.nearby[old];
                        if !near.is_empty() {
                            sol.routes[a].refill_node = Some(near[self.rng.below(near.len())]);
                        }
                    } else {
                        sol.routes[a].refill_node = if x < nodes { Some(x) } else { None };
                    }
                } else {
                    sol.routes[a].refill_node = if x < nodes { Some(x) } else { None };
                }
            }
            16 => {
                if suppliers == 0 {
                    return;
                }
                let b = self.rng.below(n);
                if let Some(r) = sol.routes[b].refill {
                    let target = sol.routes[b].refill_node.or_else(|| {
                        if r > 0 {
                            Some(sol.routes[b].nodes[r - 1])
                        } else {
                            None
                        }
                    });
                    sol.routes[a].refill_node = target;
                    if sol.routes[a].refill.is_none() {
                        sol.routes[a].refill = Some(self.rng.below(len + 1));
                    }
                }
            }
            17 | 18 => {
                let b = self.rng.below(n);
                if a == b {
                    return;
                }
                let i = self.rng.below(len + 1);
                let j = self.rng.below(sol.routes[b].nodes.len() + 1);
                let an = sol.routes[a].nodes.split_off(i);
                let am = sol.routes[a].modes.split_off(i);
                let bn = sol.routes[b].nodes.split_off(j);
                let bm = sol.routes[b].modes.split_off(j);
                sol.routes[a].nodes.extend(bn);
                sol.routes[a].modes.extend(bm);
                sol.routes[b].nodes.extend(an);
                sol.routes[b].modes.extend(am);
                if let Some(r) = sol.routes[a].refill {
                    sol.routes[a].refill = Some(r.min(sol.routes[a].nodes.len()));
                }
                if let Some(r) = sol.routes[b].refill {
                    sol.routes[b].refill = Some(r.min(sol.routes[b].nodes.len()));
                }
            }
            _ => {
                if suppliers == 0 {
                    return;
                }
                let s = self.rng.below(suppliers);
                if sol.orders[s].is_empty() {
                    return;
                }
                let i = self.rng.below(sol.orders[s].len());
                let car = sol.orders[s].remove(i);
                let t = self.rng.below(suppliers);
                let j = self.rng.below(sol.orders[t].len() + 1);
                sol.orders[t].insert(j, car);
            }
        }
    }
    pub fn plan(&mut self, game: &Game, seen: u64, millis: u64) -> Plan {
        let seconds = (millis as f64 * 0.001 * 0.90).min(52.0);
        self.solve(game, seen, seconds)
    }
    pub fn select(&mut self, p: &Problem, millis: u64) -> Vec<u8> {
        let begun = Instant::now();
        let n = p.agents.len();
        let mut candidates = Vec::new();
        for s in 0..n {
            let mut kinds = vec![0; n];
            kinds[s] = 1;
            candidates.push((kinds.clone(), 1));
            if n <= 4 {
                candidates.push((kinds, 2));
            }
        }
        if n >= 6 && p.fuel_limits < p.day_steps[0] {
            for s in 0..n {
                let mut kinds = vec![0; n];
                kinds[s] = 1;
                kinds[(s + n / 2) % n] = 1;
                candidates.push((kinds, 1));
            }
        }
        if n == 1 || p.fuel_limits * 2 > p.day_steps.iter().sum::<u32>() {
            candidates.push((vec![0; n], 1));
        }
        let totaldays = p.day_steps.len() * candidates.len();
        let allotted = (millis as f64 * 0.001 * 0.80 / totaldays as f64).max(0.02);
        let mut best_score = (0, 0, 0);
        let mut best_initial = None;
        let mut best = candidates[0].0.clone();
        let mut best_depth = 1;
        for (kinds, depth) in candidates {
            self.planning_depth = depth;
            self.carry = None;
            self.initial = None;
            let mut first_initial = None;
            if begun.elapsed().as_millis() as u64 > millis * 9 / 10 {
                break;
            }
            let mut g = Game::new(p.clone(), &kinds);
            let mut seen = 0u64;
            for _ in 0..p.day_steps.len() {
                let plan = self.solve(&g, seen, allotted);
                if g.day == 0 {
                    first_initial = self.carry.clone();
                }
                if let Ok(day) = g.play_day(plan) {
                    for frame in day.frames {
                        for event in frame.events {
                            if let Event::Collect { brand, .. } = event {
                                seen |= 1u64 << brand;
                            }
                        }
                    }
                } else {
                    let _ = g.play_day(g.waiting_plan());
                }
            }
            let score = (g.score.unique_brands, g.score.daily_brands, g.score.udon);
            eprintln!("selection {:?} depth {} score {:?}", kinds, depth, score);
            if score > best_score {
                best_score = score;
                best = kinds;
                best_initial = first_initial;
                best_depth = depth;
            }
        }
        self.carry = None;
        self.initial = best_initial;
        self.planning_depth = best_depth;
        eprintln!(
            "selected {:?} depth {} score {:?}",
            best, best_depth, best_score
        );
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_game() -> Game {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![8],
            map: Map {
                width: 5,
                height: 1,
                cells: vec![vec![0; 5]],
            },
            spots: vec![Spot {
                brand: 0,
                pos: 4,
                stocks: 1,
            }],
            agents: vec![0, 2],
            fuel_limits: 3,
            players: 1,
            busy_threshold: 2,
            jammed_threshold: 4,
        };
        Game::new(p, &[0, 1])
    }

    #[test]
    fn meeting_at_arrival_refills_before_the_next_move() {
        let mut game = line_game();
        game.agents[0].fuel = 1;
        let network = Network::new(&game);
        let mut at_hub = game.clone();
        at_hub.agents[0].pos = 1;
        let hub = Network::new(&at_hub).starts[0];
        let context = Context {
            game: &game,
            net: &network,
            patrols: vec![0],
            supplies: vec![1],
            seen: 0,
            remaining: 1,
        };
        let solution = Solution {
            routes: vec![Route {
                nodes: vec![0],
                modes: vec![0],
                refill: Some(0),
                refill_node: Some(hub),
                refill_mode: 0,
            }],
            orders: vec![vec![0]],
        };
        let evaluation = context.evaluate(&solution, &network.starts, &[1, 3], 0);
        assert!(evaluation.valid);
        let plan = context.compile(&solution, &evaluation);
        game.play_day(plan).unwrap();
        assert_eq!(game.agents[0].pos, 4);
        assert_eq!(game.agents[0].fuel, 0);
        assert_eq!(game.score.udon, 1);
    }

    #[test]
    fn arrival_refill_cannot_pay_for_the_move_into_the_meeting_cell() {
        let game = line_game();
        let network = Network::new(&game);
        let mut at_hub = game.clone();
        at_hub.agents[0].pos = 1;
        let hub = Network::new(&at_hub).starts[0];
        let context = Context {
            game: &game,
            net: &network,
            patrols: vec![0],
            supplies: vec![1],
            seen: 0,
            remaining: 1,
        };
        let solution = Solution {
            routes: vec![Route {
                nodes: vec![0],
                modes: vec![0],
                refill: Some(0),
                refill_node: Some(hub),
                refill_mode: 0,
            }],
            orders: vec![vec![0]],
        };
        assert!(
            !context
                .evaluate(&solution, &network.starts, &[0, 3], 0)
                .valid
        );
    }

    #[test]
    fn intermediate_cell_ids_survive_a_changed_observation() {
        let mut original = line_game();
        let before = Network::new(&original);
        let mut observed = original.clone();
        observed.agents[0].pos = 3;
        let after = Network::new(&observed);
        let target = after.starts[0];
        let path = before.path(before.starts[1], target, 0);
        let mut supply = path.actions.clone();
        if path.time < 8 {
            supply.push(-i64::from(8 - path.time));
        }
        original.play_day(vec![vec![-8], supply]).unwrap();
        assert_eq!(original.agents[1].pos, 3);
    }
}
