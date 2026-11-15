use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    time::{Duration, Instant},
};
use tester::{
    engine::Game,
    model::{Agent, Observation, Plan, Problem, Traffic},
};
const INF: u16 = 30000;
const MAX_ROUTE: usize = 13;
#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 7;
        self.0 ^= self.0 >> 9;
        self.0 ^= self.0 << 8;
        self.0
    }
    fn usize(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}
#[derive(Clone, Default)]
struct Path {
    time: u16,
    fuel: u16,
    mask: u32,
    brands: u32,
    dirs: Vec<i64>,
}
struct Roads {
    n: usize,
    paths: Vec<Path>,
    departure: Vec<u16>,
    brands: Vec<u32>,
}
impl Roads {
    fn at(&self, a: usize, b: usize, v: usize) -> &Path {
        &self.paths[(v * self.n + a) * self.n + b]
    }
}
#[derive(Clone)]
struct Genome {
    supplier: usize,
    secondary: usize,
    secondary_hubs: Vec<[usize; 3]>,
    secondary_switches: Vec<[u16; 2]>,
    hubs: Vec<usize>,
    late_hubs: Vec<usize>,
    last_hubs: Vec<usize>,
    last_switches: Vec<u16>,
    switches: Vec<u16>,
    routes: Vec<Vec<Vec<usize>>>,
    variant: Vec<Vec<usize>>,
    reserve: Vec<Vec<u16>>,
}
impl Genome {
    fn is_supplier(&self, i: usize) -> bool {
        i == self.supplier || i == self.secondary
    }
    fn refill_count(&self, a: usize) -> usize {
        if self.secondary < a { 6 } else { 3 }
    }
}
fn use_two_supplies(p: &Problem) -> bool {
    let mut steps = p.day_steps.clone();
    steps.sort_unstable();
    p.agents.len() >= 7
        && p.day_steps.len() <= 7
        && p.fuel_limits * 10 <= p.day_steps[0] * 11
        && steps[steps.len() / 2] * 2 <= p.day_steps[0] * 3
}
fn fixed_hubs(p: &Problem) -> Vec<usize> {
    let mut nodes: Vec<_> = p.spots.iter().map(|s| s.pos).collect();
    if p.agents.len() <= 5 {
        let mut extra = std::collections::BTreeSet::new();
        for spot in &p.spots {
            for pos in p.map.neighbors(spot.pos) {
                if p.map.terrain(pos) != 3 && !nodes.contains(&pos) {
                    extra.insert(pos);
                }
            }
        }
        nodes.extend(extra);
    }
    nodes
}
#[derive(Clone)]
struct Evaluation {
    value: f64,
    unique: usize,
    daily: usize,
    udon: u32,
    plans: Vec<Plan>,
    masks: Vec<Vec<u32>>,
    ends: Vec<Vec<(usize, u16)>>,
}
pub struct Solver {
    p: Problem,
    rng: Rng,
    game: Option<Game>,
    genome: Option<Genome>,
    pending: Option<Plan>,
    seen: u32,
}
struct Context<'a> {
    hub_count: usize,
    p: &'a Problem,
    two_supplies: bool,
    mobility_table: Vec<Vec<f64>>,
    roads: Vec<Roads>,
    start: Vec<(usize, u16)>,
    day: usize,
    seen: u32,
    brand_masks: Vec<u32>,
}
impl Solver {
    pub fn new(p: Problem, seed: u64) -> Self {
        Self {
            p,
            rng: Rng(seed.wrapping_add(0x1928efb954db1231).max(1)),
            game: None,
            genome: None,
            pending: None,
            seen: 0,
        }
    }
    fn context(&self, day: usize, agents: &[Agent], traffic: &[Traffic]) -> Context<'_> {
        let mut nodes = fixed_hubs(&self.p);
        let hub_count = nodes.len();
        for a in agents {
            if !nodes.contains(&a.pos) {
                nodes.push(a.pos)
            }
        }
        let start = agents
            .iter()
            .map(|a| {
                (
                    nodes.iter().position(|&p| p == a.pos).unwrap(),
                    a.fuel as u16,
                )
            })
            .collect();
        let today = roads(&self.p, &nodes, traffic, false);
        let rs = vec![today];
        let maxbrand = self.p.spots.iter().map(|s| s.brand).max().unwrap();
        let brand_masks: Vec<u32> = (0..=maxbrand)
            .map(|b| {
                self.p
                    .spots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.brand == b)
                    .fold(0, |m, (s, _)| m | 1 << s)
            })
            .filter(|&m| m != 0)
            .collect();
        let cap = (self.p.fuel_limits as usize).min(240);
        let mut mobility_table = vec![vec![0.; cap + 1]; nodes.len()];
        if day + rs.len() < self.p.day_steps.len() {
            let next_steps = self.p.day_steps[day + rs.len()] as u16;
            let target = (brand_masks.len() as f64
                / (agents.len() - if use_two_supplies(&self.p) { 2 } else { 1 }) as f64)
                .min(5.0);
            for pos in 0..nodes.len() {
                let needs: Vec<_> = brand_masks
                    .iter()
                    .map(|&bm| {
                        let mut need = INF;
                        for t in 0..self.p.spots.len() {
                            if bm & (1 << t) != 0 {
                                for v in [0, 2] {
                                    let path = rs[rs.len() - 1].at(pos, t, v);
                                    if path.time <= next_steps * 4 / 5 {
                                        need = need.min(path.fuel)
                                    }
                                }
                            }
                        }
                        need
                    })
                    .collect();
                let tour = if agents.len() <= 5 {
                    Some(tour_fuel_needs(
                        &rs[rs.len() - 1],
                        self.p.spots.len(),
                        pos,
                        next_steps * 4 / 5,
                        cap as u16,
                        ((target.ceil() as usize) + 1).min(brand_masks.len()),
                    ))
                } else {
                    None
                };
                for f in 0..=cap {
                    let count = needs.iter().filter(|&&n| n <= f as u16).count();
                    let tour_deficit = if let Some(tour) = &tour {
                        let visits = tour
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| **n <= f as u16)
                            .map(|(k, _)| k)
                            .max()
                            .unwrap_or(0);
                        (target - visits as f64).max(0.) * 8.
                    } else {
                        0.
                    };
                    let remaining = self.p.day_steps.len() - day - rs.len();
                    let stranded = if agents.len() <= 5 {
                        (2usize.saturating_sub(count) * remaining.min(3)) as f64 * 12000.
                    } else {
                        0.
                    };
                    mobility_table[pos][f] = f.min(60) as f64 * 0.7
                        - (target - count as f64).max(0.) * 3500.
                        - stranded
                        - tour_deficit;
                }
            }
        }
        Context {
            hub_count,
            p: &self.p,
            two_supplies: use_two_supplies(&self.p),
            mobility_table,
            roads: rs,
            start,
            day,
            seen: self.seen,
            brand_masks,
        }
    }
    pub fn select(&mut self, limit: u64) -> Vec<u8> {
        let agents: Vec<_> = self
            .p
            .agents
            .iter()
            .map(|&pos| Agent {
                kind: 0,
                pos,
                fuel: self.p.fuel_limits,
            })
            .collect();
        let ctx = self.context(0, &agents, &[]);
        let mut rng = self.rng.clone();
        let (g, e) = optimize(&ctx, None, &mut rng, budget(limit, true), true);
        eprintln!(
            "select forecast {}/{}/{} supplier {}",
            e.unique, e.daily, e.udon, g.supplier
        );
        let mut kinds = vec![0; agents.len()];
        if g.supplier < kinds.len() {
            kinds[g.supplier] = 1
        }
        if g.secondary < kinds.len() {
            kinds[g.secondary] = 1;
        }
        self.game = Some(Game::new(self.p.clone(), &kinds));
        self.genome = Some(g);
        self.rng = rng;
        kinds
    }
    pub fn plan(&mut self, state: Observation, limit: u64, accepted: bool) -> Plan {
        if let Some(plan) = self.pending.take() {
            let game = self.game.as_mut().unwrap();
            let applied = if accepted { plan } else { game.waiting_plan() };
            if let Ok(rep) = game.play_day(applied) {
                for f in rep.frames {
                    for e in f.events {
                        if let tester::model::Event::Collect { spot, .. } = e {
                            self.seen |= 1 << spot
                        }
                    }
                }
            }
        }
        let game = self.game.as_mut().unwrap();
        game.agents = state.agents.clone();
        game.day = state.day;
        game.set_observed_traffic(state.traffics.clone());
        let mut seed = self.genome.take();
        if state.day > 0 {
            if let Some(g) = seed.as_mut() {
                if g.hubs.len() > 1 {
                    g.hubs.remove(0);
                    g.secondary_hubs.remove(0);
                    g.secondary_switches.remove(0);
                    g.late_hubs.remove(0);
                    g.last_hubs.remove(0);
                    g.last_switches.remove(0);
                    g.switches.remove(0);
                    g.routes.remove(0);
                    g.variant.remove(0);
                    g.reserve.remove(0);
                }
            }
        }
        // Depot indices refer to fixed spots and adjacent cells; only current-position suffixes vary.
        let ctx = self.context(state.day, &state.agents, &state.traffics);
        let mut rng = self.rng.clone();
        let (g, e) = optimize(&ctx, seed, &mut rng, budget(limit, false), false);
        let mut plan = e.plans[0].clone();
        if let Err(err) = self.game.as_ref().unwrap().clone().play_day(plan.clone()) {
            eprintln!("validation fallback: {}", err.message);
            plan = self.game.as_ref().unwrap().waiting_plan();
        }
        eprintln!(
            "day {} forecast {}/{}/{}",
            state.day, e.unique, e.daily, e.udon
        );
        self.genome = Some(g);
        self.pending = Some(plan.clone());
        self.rng = rng;
        plan
    }
}
fn budget(limit: u64, select: bool) -> Duration {
    let default = if select { 100000 } else { 48000 };
    let cap = std::env::var("HORIZON_SEARCH_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default);
    Duration::from_millis(cap.min(limit.saturating_mul(78) / 100).max(1))
}
fn roads(p: &Problem, nodes: &[usize], traffic: &[Traffic], forecast: bool) -> Roads {
    let n = nodes.len();
    let cells = p.map.len();
    let mut times = vec![2u16; cells];
    let mut fuels = vec![1u16; cells];
    for x in 0..cells {
        match p.map.terrain(x) {
            1 => {
                times[x] = 1;
                fuels[x] = 2
            }
            2 => {
                times[x] = 3;
                fuels[x] = 2
            }
            _ => {}
        }
    }
    for t in traffic {
        times[t.pos] = [1, 2, 4][t.status as usize]
    }
    if forecast {
        for x in 0..cells {
            times[x] = if p.map.terrain(x) == 1 {
                match times[x] {
                    1 => 4,
                    2 => 4,
                    _ => 8,
                }
            } else {
                times[x] * 2
            };
        }
    }
    let unit = if forecast { 2 } else { 1 };
    let mut at = vec![0u32; cells];
    let mut brand_ids: Vec<_> = p.spots.iter().map(|s| s.brand).collect();
    brand_ids.sort_unstable();
    brand_ids.dedup();
    let mut at_brand = vec![0u32; cells];
    for (i, s) in p.spots.iter().enumerate() {
        at[s.pos] = 1 << i;
        at_brand[s.pos] = 1 << brand_ids.binary_search(&s.brand).unwrap();
    }
    let mut paths = vec![Path::default(); n * n * 3];
    for v in 0..3 {
        let fp = [0u64, 10000, (cells as u64 * 4 + 1) * 10000][v] * unit as u64;
        for (a, &source) in nodes.iter().enumerate() {
            let mut dist = vec![u64::MAX; cells];
            let mut prev = vec![(0usize, 0i64); cells];
            let mut heap = BinaryHeap::new();
            dist[source] = 0;
            heap.push(Reverse((0u64, source)));
            while let Some(Reverse((d, x))) = heap.pop() {
                if d != dist[x] {
                    continue;
                }
                for dir in 0..6 {
                    if let Some(y) = p.map.neighbor(x, dir) {
                        if p.map.terrain(y) == 3 {
                            continue;
                        }
                        let nd = d + times[x] as u64 * 10000 + fuels[x] as u64 * fp + 1;
                        if nd < dist[y] {
                            dist[y] = nd;
                            prev[y] = (x, dir as i64);
                            heap.push(Reverse((nd, y)))
                        }
                    }
                }
            }
            for (b, &dest) in nodes.iter().enumerate() {
                let mut route = Path::default();
                let mut x = dest;
                if dist[x] == u64::MAX {
                    route.time = INF;
                    route.fuel = INF;
                } else {
                    while x != source {
                        route.mask |= at[x];
                        route.brands |= at_brand[x];
                        let (y, dir) = prev[x];
                        route.time += times[y];
                        route.fuel += fuels[y];
                        route.dirs.push(dir);
                        x = y
                    }
                    route.dirs.reverse();
                    route.time = route.time.div_ceil(unit as u16);
                }
                paths[(v * n + a) * n + b] = route;
            }
        }
    }
    Roads {
        n,
        paths,
        departure: nodes
            .iter()
            .map(|&p| times[p].div_ceil(unit as u16))
            .collect(),
        brands: nodes.iter().map(|&p| at_brand[p]).collect(),
    }
}
fn tour_fuel_needs(
    r: &Roads,
    spots: usize,
    source: usize,
    steps: u16,
    cap: u16,
    max_count: usize,
) -> Vec<u16> {
    #[derive(Clone, Copy)]
    struct Label {
        pos: usize,
        time: u16,
        fuel: u16,
        mask: u32,
    }
    let mut buckets = vec![Vec::<Label>::new(); max_count + 1];
    let first = r.brands[source];
    buckets[(first.count_ones() as usize).min(max_count)].push(Label {
        pos: source,
        time: 0,
        fuel: 0,
        mask: first,
    });
    let mut needs = vec![INF; max_count + 1];
    needs[0] = 0;
    for count in 0..=max_count {
        let mut labels = std::mem::take(&mut buckets[count]);
        if labels.is_empty() {
            continue;
        }
        needs[count] = labels.iter().map(|l| l.fuel).min().unwrap();
        if count == max_count {
            continue;
        }
        labels.sort_unstable_by_key(|l| (l.fuel as u32 * 4 + l.time as u32, l.time));
        let mut seen = std::collections::HashMap::<(usize, u32), Vec<(u16, u16)>>::new();
        let mut used = 0;
        for label in labels {
            let prev = seen.entry((label.pos, label.mask)).or_default();
            if prev
                .iter()
                .any(|&(f, t)| f <= label.fuel && t <= label.time)
            {
                continue;
            }
            prev.push((label.fuel, label.time));
            for target in 0..spots {
                if label.mask & r.brands[target] != 0 {
                    continue;
                }
                for v in [0, 2] {
                    let path = r.at(label.pos, target, v);
                    let fuel = label.fuel + path.fuel;
                    let time = label.time + path.time;
                    if fuel > cap || time > steps {
                        continue;
                    }
                    let mask = label.mask | path.brands;
                    let k = (mask.count_ones() as usize).min(max_count);
                    buckets[k].push(Label {
                        pos: target,
                        time,
                        fuel,
                        mask,
                    });
                }
            }
            used += 1;
            if used == 64 {
                break;
            }
        }
    }
    needs
}
fn support_intervals(
    c: &Context,
    g: &Genome,
    d: usize,
    source: usize,
    secondary: bool,
) -> Vec<(usize, u16, u16)> {
    let r = &c.roads[d];
    let steps = c.p.day_steps[c.day + d] as u16;
    let hubs = if secondary {
        g.secondary_hubs[d]
    } else {
        [g.hubs[d], g.late_hubs[d], g.last_hubs[d]]
    };
    let switches = if secondary {
        g.secondary_switches[d]
    } else {
        [g.switches[d], g.last_switches[d]]
    };
    let mut intervals = vec![];
    let mut from = source;
    let mut time = 0;
    for k in 0..3 {
        let arrival = time + r.at(from, hubs[k], 0).time;
        if arrival > steps {
            if let Some((_, _, end)) = intervals.last_mut() {
                *end = steps;
            }
            break;
        }
        let depart = if k < 2 {
            arrival.max(steps * switches[k] / 100)
        } else {
            steps
        };
        intervals.push((hubs[k], arrival, depart));
        time = depart;
        from = hubs[k];
    }
    intervals
}
fn refill_intervals(
    schedule: &[(usize, u16, u16)],
    r: &Roads,
    steps: u16,
) -> Vec<(usize, u16, u16)> {
    schedule
        .iter()
        .enumerate()
        .map(|(k, &(hub, begin, end))| {
            // A reserved move leaves the car at its source until reflection.
            let end = if k + 1 < schedule.len() && hub != schedule[k + 1].0 {
                (end + r.departure[hub] - 1).min(steps)
            } else {
                end
            };
            (hub, begin, end)
        })
        .collect()
}
fn evaluate(c: &Context, g: &Genome, output: bool) -> Evaluation {
    let a = c.start.len();
    let days = c.roads.len();
    let mut pos: Vec<_> = c.start.iter().map(|v| v.0).collect();
    let mut fuel: Vec<_> = c.start.iter().map(|v| v.1).collect();
    let mut ever = c.seen;
    let mut daily = 0;
    let mut udon = 0;
    let mut plans = Vec::new();
    let mut masks = Vec::new();
    let mut ends = Vec::new();
    let mut mobility = 0.;
    let mut weighted_daily = 0.;
    let mut weighted_udon = 0.;
    for d in 0..days {
        let r = &c.roads[d];
        let steps = c.p.day_steps[c.day + d] as u16;
        let next_hub = g.hubs[(d + 1).min(days - 1)];
        let supply = g.supplier < a;
        let mut intervals = vec![];
        let mut day_plan = vec![vec![]; a];
        let mut mask = vec![0u32; a];
        for (si, secondary) in [(g.supplier, false), (g.secondary, true)] {
            if si >= a {
                continue;
            }
            let schedule = support_intervals(c, g, d, pos[si], secondary);
            if schedule.is_empty() {
                if output {
                    day_plan[si].push(-i64::from(steps));
                }
            } else {
                for &(h, begin, end) in &schedule {
                    if output {
                        day_plan[si].extend_from_slice(&r.at(pos[si], h, 0).dirs);
                        if end > begin {
                            day_plan[si].push(-i64::from(end - begin));
                        }
                    }
                    pos[si] = h;
                }
            }
            intervals.extend(refill_intervals(&schedule, r, steps));
        }
        let hub_active = !intervals.is_empty();
        for i in 0..a {
            if g.is_supplier(i) {
                continue;
            }
            let base_variant = g.variant[d][i];
            let mut time = 0u16;
            let mut got = if pos[i] < c.p.spots.len() {
                1u32 << pos[i]
            } else {
                0
            };
            for &gene in &g.routes[d][i] {
                let stride = c.p.spots.len() + g.refill_count(a);
                let target = gene % stride;
                let v = if gene / stride == 0 {
                    base_variant
                } else {
                    gene / stride - 1
                };
                if target >= c.p.spots.len() {
                    let idx = target - c.p.spots.len();
                    if let Some(&(h, begin, end)) = intervals.get(idx) {
                        let home = r.at(pos[i], h, v);
                        let meet = (time + home.time).max(begin).max(1);
                        if home.fuel <= fuel[i] && meet <= end && meet <= steps {
                            if output {
                                day_plan[i].extend_from_slice(&home.dirs);
                                if meet > time + home.time {
                                    day_plan[i].push(-i64::from(meet - time - home.time))
                                }
                            }
                            got |= home.mask;
                            if h < c.p.spots.len() {
                                got |= 1 << h
                            }
                            time = meet;
                            pos[i] = h;
                            fuel[i] = c.p.fuel_limits as u16;
                        }
                    }
                    continue;
                }
                if got & (1 << target) != 0 {
                    continue;
                }
                let path = r.at(pos[i], target, v);
                let reserve = if c.day + d + 1 < c.p.day_steps.len() && supply {
                    r.at(target, next_hub, 2)
                        .fuel
                        .min(if g.secondary < a {
                            r.at(target, g.secondary_hubs[(d + 1).min(days - 1)][0], 2)
                                .fuel
                        } else {
                            INF
                        })
                        .min((c.p.fuel_limits / 2) as u16)
                        * g.reserve[d][i]
                        / 4
                } else {
                    0
                };
                let want_refill = hub_active && (path.fuel + reserve > fuel[i]);
                if want_refill {
                    let mut best_refill = None;
                    let mut best_time = INF;
                    for &(h, begin, end) in &intervals {
                        for rv in 0..3 {
                            let home = r.at(pos[i], h, rv);
                            let outward = r.at(h, target, v);
                            let meet = (time + home.time).max(begin).max(1);
                            if home.fuel <= fuel[i]
                                && meet <= end
                                && meet + outward.time <= steps
                                && outward.fuel <= c.p.fuel_limits as u16
                                && meet + outward.time < best_time
                            {
                                best_time = meet + outward.time;
                                best_refill = Some((h, rv, meet));
                            }
                        }
                    }
                    if let Some((h, rv, meet)) = best_refill {
                        let home = r.at(pos[i], h, rv);
                        if output {
                            day_plan[i].extend_from_slice(&home.dirs);
                            if meet > time + home.time {
                                day_plan[i].push(-i64::from(meet - time - home.time))
                            }
                        }
                        got |= home.mask;
                        if h < c.p.spots.len() {
                            got |= 1 << h
                        }
                        time = meet;
                        pos[i] = h;
                        fuel[i] = c.p.fuel_limits as u16;
                    }
                }
                let path = r.at(pos[i], target, v);
                if time + path.time > steps || path.fuel > fuel[i] {
                    continue;
                }
                time += path.time;
                fuel[i] -= path.fuel;
                got |= path.mask;
                pos[i] = target;
                if output {
                    day_plan[i].extend_from_slice(&path.dirs)
                }
                if intervals
                    .iter()
                    .any(|&(h, b, e)| pos[i] == h && time >= b && time <= e && time > 0)
                {
                    fuel[i] = c.p.fuel_limits as u16
                }
            }
            // Explicit end-of-day refills are optional genes, represented by the hub as the last target.
            if intervals
                .iter()
                .any(|&(h, _, e)| pos[i] == h && e >= time && e > 0)
            {
                fuel[i] = c.p.fuel_limits as u16
            }
            if output && time < steps {
                day_plan[i].push(-i64::from(steps - time))
            }
            mask[i] = got;
            if d + 1 == days && c.day + days < c.p.day_steps.len() {
                let gap = if supply {
                    r.at(pos[i], pos[g.supplier], 2)
                        .fuel
                        .min(if g.secondary < a {
                            r.at(pos[i], pos[g.secondary], 2).fuel
                        } else {
                            INF
                        })
                } else {
                    0
                };
                let deficit = (gap as f64 - fuel[i] as f64).max(0.);
                mobility += c.mobility_table[pos[i]]
                    [(fuel[i] as usize).min(c.mobility_table[pos[i]].len() - 1)]
                    - deficit * deficit * 0.20;
            }
        }
        let daymask = mask.iter().fold(0, |x, &y| x | y);
        ever |= daymask;
        let db = c.brand_masks.iter().filter(|&&b| b & daymask != 0).count();
        daily += db;
        weighted_daily += db as f64 * 0.8_f64.powi(d as i32);
        let prev_udon = udon;
        for (s, spot) in c.p.spots.iter().enumerate() {
            udon += (mask.iter().filter(|&&m| m & (1 << s) != 0).count() as u32).min(spot.stocks)
        }
        weighted_udon += (udon - prev_udon) as f64 * 0.8_f64.powi(d as i32);
        if output {
            plans.push(day_plan);
            masks.push(mask);
            ends.push(pos.iter().zip(&fuel).map(|(&p, &f)| (p, f)).collect());
        }
    }
    let unique = c.brand_masks.iter().filter(|&&b| b & ever != 0).count();
    if c.day + days < c.p.day_steps.len() {
        for &bm in &c.brand_masks {
            let mut dist = INF;
            for target in 0..c.p.spots.len() {
                if bm & (1 << target) == 0 {
                    continue;
                }
                for i in 0..a {
                    if g.is_supplier(i) {
                        continue;
                    }
                    let path = c.roads[days - 1].at(pos[i], target, 0);
                    let adjusted = path.time + path.fuel.saturating_sub(fuel[i]) / 2;
                    dist = dist.min(adjusted);
                }
            }
            mobility -= dist as f64 * 0.10;
        }
    }
    let unique_weight = if c.day + 1 == c.p.day_steps.len() {
        1000000.
    } else {
        5000.
    };
    let value =
        unique as f64 * unique_weight + weighted_daily * 30000. + weighted_udon * 10. + mobility;
    Evaluation {
        value,
        unique,
        daily,
        udon,
        plans,
        masks,
        ends,
    }
}
fn blank(c: &Context, supplier: usize, hub: usize) -> Genome {
    let days = c.roads.len();
    let a = c.start.len();
    Genome {
        supplier,
        secondary: a,
        secondary_hubs: vec![[hub; 3]; days],
        secondary_switches: vec![[35, 70]; days],
        hubs: vec![hub; days],
        late_hubs: vec![hub; days],
        last_hubs: vec![hub; days],
        last_switches: vec![75; days],
        switches: vec![50; days],
        routes: vec![vec![vec![]; a]; days],
        variant: vec![vec![0; a]; days],
        reserve: vec![vec![4; a]; days],
    }
}
fn greedy(c: &Context, g: &mut Genome, rng: &mut Rng) {
    let a = c.start.len();
    let s = c.p.spots.len();
    for d in 0..c.roads.len() {
        for _ in 0..s * 2 {
            let base = evaluate(c, g, false).value;
            let mut best = base;
            let mut choice = None;
            for _ in 0..(s * a).min(110) {
                let i = rng.usize(a);
                if g.is_supplier(i) || g.routes[d][i].len() >= MAX_ROUTE {
                    continue;
                }
                let target = rng.usize(s);
                if g.routes[d][i].contains(&target) {
                    continue;
                }
                let at = rng.usize(g.routes[d][i].len() + 1);
                g.routes[d][i].insert(at, target);
                let value = evaluate(c, g, false).value + (rng.f64() - 0.5) * 3.;
                g.routes[d][i].remove(at);
                if value > best {
                    best = value;
                    choice = Some((i, at, target))
                }
            }
            if let Some((i, at, t)) = choice {
                g.routes[d][i].insert(at, t)
            } else {
                break;
            }
        }
    }
}
fn repair_neighborhood(c: &Context, g: &Genome, rng: &mut Rng) -> Genome {
    let mut next = g.clone();
    let a = c.start.len();
    let d = if rng.usize(2) == 0 {
        0
    } else {
        rng.usize(c.roads.len())
    };
    let mut removed = vec![];
    for _ in 0..(2 + rng.usize(6)) {
        let i = rng.usize(a);
        if g.is_supplier(i) || next.routes[d][i].is_empty() {
            continue;
        }
        let at = rng.usize(next.routes[d][i].len());
        removed.push((d, next.routes[d][i].remove(at)));
        if d + 1 < c.roads.len() && rng.usize(2) == 0 && !next.routes[d + 1][i].is_empty() {
            let at = rng.usize(next.routes[d + 1][i].len());
            removed.push((d + 1, next.routes[d + 1][i].remove(at)));
        }
    }
    removed.push((d, rng.usize(c.p.spots.len() + g.refill_count(a))));
    removed.sort_by_key(|x| x.0);
    for (d, target) in removed {
        let mut choice = None;
        let mut score = evaluate(c, &next, false).value;
        for i in 0..a {
            if g.is_supplier(i)
                || next.routes[d][i].len() >= MAX_ROUTE
                || next.routes[d][i].contains(&target)
            {
                continue;
            }
            for at in 0..=next.routes[d][i].len() {
                next.routes[d][i].insert(at, target);
                let value = evaluate(c, &next, false).value;
                next.routes[d][i].remove(at);
                if value > score {
                    score = value;
                    choice = Some((i, at));
                }
            }
        }
        if let Some((i, at)) = choice {
            next.routes[d][i].insert(at, target)
        }
    }
    next
}
#[derive(Clone)]
struct BeamState {
    pos: usize,
    time: u16,
    fuel: u16,
    mask: u32,
    route: Vec<usize>,
    priority: f64,
}
fn beam_rebuild(c: &Context, g: &Genome, rng: &mut Rng) -> Genome {
    let a = c.start.len();
    let i = loop {
        let i = rng.usize(a);
        if !g.is_supplier(i) {
            break i;
        }
    };
    let d = rng.usize(c.roads.len());
    let v = rng.usize(3);
    let r = &c.roads[d];
    let steps = c.p.day_steps[c.day + d] as u16;
    let spots = c.p.spots.len();
    let eval = evaluate(c, g, true);
    let initial_state = if d == 0 { &c.start } else { &eval.ends[d - 1] };
    let others = eval.masks[d]
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .fold(0, |m, (_, x)| m | x);
    let others_counts: Vec<_> = (0..spots)
        .map(|s| {
            eval.masks[d]
                .iter()
                .enumerate()
                .filter(|(j, m)| *j != i && **m & (1 << s) != 0)
                .count() as u32
        })
        .collect();
    let mut intervals = refill_intervals(
        &support_intervals(c, g, d, initial_state[g.supplier].0, false),
        r,
        steps,
    );
    if g.secondary < a {
        intervals.extend(refill_intervals(
            &support_intervals(c, g, d, initial_state[g.secondary].0, true),
            r,
            steps,
        ));
    }
    let value = |mask: u32, time: u16, fuel: u16| -> f64 {
        let brands = c
            .brand_masks
            .iter()
            .filter(|&&b| b & others == 0 && b & mask != 0)
            .count();
        let udon = (0..spots)
            .filter(|&s| mask & (1 << s) != 0 && others_counts[s] < c.p.spots[s].stocks)
            .count();
        brands as f64 * 1000.
            + udon as f64 * 10.
            + (steps - time) as f64 * 0.002
            + fuel as f64 * 0.001
    };
    let initial = BeamState {
        pos: initial_state[i].0,
        time: 0,
        fuel: initial_state[i].1,
        mask: if initial_state[i].0 < spots {
            1 << initial_state[i].0
        } else {
            0
        },
        route: vec![],
        priority: 0.,
    };
    let mut beam = vec![initial];
    let mut best = g.clone();
    let mut bestval = eval.value;
    let mut candidate = g.clone();
    candidate.variant[d][i] = v;
    candidate.reserve[d][i] = 0;
    for _ in 0..MAX_ROUTE {
        let mut children = Vec::new();
        for st in &beam {
            for target in 0..spots + intervals.len() {
                for v in 0..3 {
                    let (to, meet, refill) = if target < spots {
                        if st.mask & (1 << target) != 0 {
                            continue;
                        }
                        let path = r.at(st.pos, target, v);
                        if path.fuel > st.fuel || st.time + path.time > steps {
                            continue;
                        }
                        (target, st.time + path.time, false)
                    } else {
                        let (to, begin, end) = intervals[target - spots];
                        let path = r.at(st.pos, to, v);
                        let meet = (st.time + path.time).max(begin).max(1);
                        if path.fuel > st.fuel
                            || meet > end
                            || meet > steps
                            || st.fuel - path.fuel + 3 >= c.p.fuel_limits as u16
                        {
                            continue;
                        }
                        (to, meet, true)
                    };
                    let path = r.at(st.pos, to, v);
                    let mask = st.mask | path.mask;
                    let mut fuel = if refill {
                        c.p.fuel_limits as u16
                    } else {
                        st.fuel - path.fuel
                    };
                    if intervals
                        .iter()
                        .any(|&(h, b, e)| to == h && meet >= b && meet <= e)
                    {
                        fuel = c.p.fuel_limits as u16
                    }
                    let mut route = st.route.clone();
                    route.push(target + (spots + g.refill_count(a)) * (v + 1));
                    children.push(BeamState {
                        pos: to,
                        time: meet,
                        fuel,
                        mask,
                        route,
                        priority: value(mask, meet, fuel),
                    });
                }
            }
        }
        if children.is_empty() {
            break;
        }
        children.sort_unstable_by(|x, y| y.priority.total_cmp(&x.priority));
        let mut kept = Vec::new();
        let mut seen = std::collections::HashMap::<(u32, usize), Vec<(u16, u16)>>::new();
        for st in children {
            let prev = seen.entry((st.mask, st.pos)).or_default();
            if prev.iter().any(|&(t, f)| t <= st.time && f >= st.fuel) {
                continue;
            }
            prev.push((st.time, st.fuel));
            kept.push(st);
            if kept.len() >= 192 {
                break;
            }
        }
        for st in kept.iter().take(40) {
            candidate.routes[d][i] = st.route.clone();
            let val = evaluate(c, &candidate, false).value;
            if val > bestval {
                bestval = val;
                best = candidate.clone();
            }
        }
        beam = kept;
    }
    best
}
fn optimize(
    c: &Context,
    seed: Option<Genome>,
    rng: &mut Rng,
    limit: Duration,
    selection: bool,
) -> (Genome, Evaluation) {
    let start = Instant::now();
    let a = c.start.len();
    let s = c.p.spots.len();
    let days = c.roads.len();
    let mut current = if let Some(mut g) = seed {
        let grow = g.hubs.len() < days;
        while g.hubs.len() < days {
            let last = g.hubs.len() - 1;
            g.hubs.push(g.last_hubs[last]);
            g.secondary_hubs.push([g.secondary_hubs[last][2]; 3]);
            g.secondary_switches.push([35, 70]);
            g.late_hubs.push(g.last_hubs[last]);
            g.last_hubs.push(g.last_hubs[last]);
            g.switches.push(35);
            g.last_switches.push(70);
            g.routes.push(vec![vec![]; a]);
            g.variant.push(vec![0; a]);
            g.reserve.push(vec![4; a]);
        }
        if grow {
            greedy(c, &mut g, rng);
        }
        g
    } else {
        let mut best = blank(c, a, 0);
        let mut score = -1.;
        for supplier in 0..a {
            let seconds: Vec<_> = if c.two_supplies {
                (supplier + 1..a).collect()
            } else {
                vec![a]
            };
            for secondary in seconds {
                let hub = (0..c.hub_count)
                    .min_by_key(|&h| {
                        c.start
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != supplier && *i != secondary)
                            .map(|(_, v)| c.roads[0].at(v.0, h, 0).time as u32)
                            .sum::<u32>()
                    })
                    .unwrap();
                let mut g = blank(c, supplier, hub);
                g.secondary = secondary;
                if secondary < a {
                    let other = (0..c.hub_count)
                        .min_by_key(|&h| c.roads[0].at(c.start[secondary].0, h, 0).time)
                        .unwrap();
                    for d in 0..days {
                        g.secondary_hubs[d] = [other; 3];
                    }
                }
                greedy(c, &mut g, rng);
                let ev = evaluate(c, &g, false);
                if ev.value > score {
                    score = ev.value;
                    best = g;
                }
            }
        }
        best
    };
    let mut cur = evaluate(c, &current, false).value;
    let mut best = current.clone();
    let mut bestval = cur;
    let mut iter = 0usize;
    while start.elapsed().as_secs_f64() < limit.as_secs_f64() * 0.78 {
        for _ in 0..128 {
            iter += 1;
            let phase = start.elapsed().as_secs_f64() / (limit.as_secs_f64() * 0.78);
            let temp = 48000. * (0.000008_f64).powf(phase.min(1.));
            let guided = iter % 160 == 0;
            let mut next = if guided {
                repair_neighborhood(c, &current, rng)
            } else {
                current.clone()
            };
            let d = rng.usize(days);
            let i = rng.usize(a);
            if next.is_supplier(i) {
                continue;
            }
            let kind = if guided { 101 } else { rng.usize(100) };
            if kind < 24 {
                if next.routes[d][i].len() < MAX_ROUTE {
                    let t = rng.usize(s + next.refill_count(a));
                    if !next.routes[d][i].contains(&t) {
                        let at = rng.usize(next.routes[d][i].len() + 1);
                        next.routes[d][i].insert(at, t)
                    }
                }
            } else if kind < 39 {
                let len = next.routes[d][i].len();
                if len > 0 {
                    let at = rng.usize(len);
                    next.routes[d][i].remove(at);
                }
            } else if kind < 55 {
                let len = next.routes[d][i].len();
                if len > 0 {
                    let at = rng.usize(len);
                    let t = rng.usize(s + next.refill_count(a));
                    if !next.routes[d][i].contains(&t) {
                        next.routes[d][i][at] = t
                    }
                }
            } else if kind < 71 {
                let len = next.routes[d][i].len();
                if len > 1 {
                    let from = rng.usize(len);
                    let to = rng.usize(len);
                    let t = next.routes[d][i].remove(from);
                    next.routes[d][i].insert(to, t);
                }
            } else if kind < 75 {
                let j = rng.usize(a);
                if !next.is_supplier(j)
                    && i != j
                    && !next.routes[d][i].is_empty()
                    && next.routes[d][j].len() < MAX_ROUTE
                {
                    let at = rng.usize(next.routes[d][i].len());
                    let t = next.routes[d][i].remove(at);
                    let to = rng.usize(next.routes[d][j].len() + 1);
                    if !next.routes[d][j].contains(&t) {
                        next.routes[d][j].insert(to, t)
                    }
                }
            } else if kind < 79 {
                let j = rng.usize(a);
                if !next.is_supplier(j) && j != i {
                    let ci = rng.usize(next.routes[d][i].len() + 1);
                    let cj = rng.usize(next.routes[d][j].len() + 1);
                    let ti = next.routes[d][i][ci..].to_vec();
                    let tj = next.routes[d][j][cj..].to_vec();
                    if ci + tj.len() <= MAX_ROUTE && cj + ti.len() <= MAX_ROUTE {
                        next.routes[d][i].truncate(ci);
                        next.routes[d][j].truncate(cj);
                        next.routes[d][i].extend(tj);
                        next.routes[d][j].extend(ti);
                    }
                }
            } else if kind < 87 {
                let hub = rng.usize(c.hub_count);
                if next.secondary < a && rng.usize(2) == 0 {
                    let k = rng.usize(3);
                    next.secondary_hubs[d][k] = hub;
                    if rng.usize(3) == 0 {
                        next.secondary_switches[d][rng.usize(2)] = rng.usize(101) as u16;
                    }
                    if d + 1 < days && k == 2 && rng.usize(2) == 0 {
                        next.secondary_hubs[d + 1][0] = hub;
                    }
                } else {
                    match rng.usize(3) {
                        0 => next.hubs[d] = hub,
                        1 => next.late_hubs[d] = hub,
                        _ => {
                            next.last_hubs[d] = hub;
                            if d + 1 < days && rng.usize(2) == 0 {
                                next.hubs[d + 1] = hub
                            }
                        }
                    }
                    if rng.usize(3) == 0 {
                        if rng.usize(2) == 0 {
                            next.switches[d] = rng.usize(101) as u16
                        } else {
                            next.last_switches[d] = rng.usize(101) as u16
                        }
                    }
                    if rng.usize(5) == 0 {
                        next.last_hubs[d] = next.late_hubs[d];
                        if rng.usize(2) == 0 {
                            next.late_hubs[d] = next.hubs[d]
                        }
                    }
                }
            } else if kind < 92 {
                if rng.usize(3) == 0 {
                    next.variant[d][i] = rng.usize(3)
                } else if rng.usize(4) == 0 {
                    next.reserve[d][i] = rng.usize(5) as u16
                } else if !next.routes[d][i].is_empty() {
                    let stride = s + next.refill_count(a);
                    let at = rng.usize(next.routes[d][i].len());
                    next.routes[d][i][at] = next.routes[d][i][at] % stride + stride * rng.usize(4);
                }
            } else if kind < 95 {
                let len = next.routes[d][i].len();
                if len > 1 {
                    let mut x = rng.usize(len);
                    let mut y = rng.usize(len);
                    if x > y {
                        std::mem::swap(&mut x, &mut y)
                    }
                    next.routes[d][i][x..=y].reverse();
                }
            } else if kind < 97 && days > 1 {
                let dd = rng.usize(days);
                next.routes[d][i] = next.routes[dd][i].clone();
            } else if kind == 101 {
            } else if selection {
                let sup = rng.usize(a);
                if !next.is_supplier(sup) {
                    let change_secondary = next.secondary < a && rng.usize(2) == 0;
                    let old = if change_secondary {
                        next.secondary
                    } else {
                        next.supplier
                    };
                    for dd in 0..days {
                        next.routes[dd].swap(sup, old);
                        next.variant[dd].swap(sup, old);
                        next.reserve[dd].swap(sup, old);
                    }
                    if change_secondary {
                        next.secondary = sup
                    } else {
                        next.supplier = sup
                    }
                }
            } else {
                let len = next.routes[d][i].len();
                if len > 2 {
                    let from = rng.usize(len);
                    let count = 1 + rng.usize(3);
                    for _ in 0..count {
                        if next.routes[d][i].len() > from {
                            next.routes[d][i].remove(from);
                        }
                    }
                }
            }
            if d + 1 < days && rng.usize(7) == 0 {
                let len = next.routes[d + 1][i].len();
                if len > 1 {
                    next.routes[d + 1][i].rotate_left(rng.usize(len));
                }
            }
            let value = evaluate(c, &next, false).value;
            let diff = value - cur;
            if diff >= 0. || rng.f64() < (diff / temp).exp() {
                current = next;
                cur = value;
                if value > bestval {
                    bestval = value;
                    best = current.clone()
                }
            }
            if iter % 50000 == 0 && rng.usize(2) == 0 {
                current = best.clone();
                cur = bestval;
            }
        }
    }
    while start.elapsed() < limit {
        let candidate = beam_rebuild(c, &best, rng);
        let value = evaluate(c, &candidate, false).value;
        if value > bestval {
            bestval = value;
            best = candidate;
        }
    }
    eprintln!(
        "horizon iterations {} elapsed {}",
        iter,
        start.elapsed().as_millis()
    );
    let e = evaluate(c, &best, true);
    (best, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn future_tour_needs_fuel_to_visit_both_sides() {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![20],
            map: tester::model::Map {
                height: 1,
                width: 5,
                cells: vec![vec![0; 5]],
            },
            spots: (0..3)
                .map(|i| tester::model::Spot {
                    brand: i,
                    pos: i * 2,
                    stocks: 2,
                })
                .collect(),
            agents: vec![1, 3],
            fuel_limits: 6,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        };
        let r = roads(&p, &[0, 2, 4], &[], false);
        let needs = tour_fuel_needs(&r, 3, 1, 20, 6, 3);
        assert_eq!(r.at(1, 0, 2).fuel, 2);
        assert_eq!(r.at(1, 2, 2).fuel, 2);
        assert_eq!(needs[2], 2);
        assert_eq!(needs[3], 6);
    }
    #[test]
    fn refill_is_available_until_a_departing_supplier_leaves() {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![8],
            map: tester::model::Map {
                height: 1,
                width: 6,
                cells: vec![vec![0, 1, 0, 0, 0, 0]],
            },
            spots: vec![
                tester::model::Spot {
                    brand: 0,
                    pos: 2,
                    stocks: 2,
                },
                tester::model::Spot {
                    brand: 1,
                    pos: 4,
                    stocks: 2,
                },
            ],
            agents: vec![5, 0],
            fuel_limits: 4,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        };
        let agents = vec![
            Agent {
                kind: 1,
                pos: 2,
                fuel: 4,
            },
            Agent {
                kind: 0,
                pos: 0,
                fuel: 3,
            },
        ];
        let solver = Solver::new(p.clone(), 0);
        let c = solver.context(0, &agents, &[]);
        let mut g = blank(&c, 0, 0);
        g.late_hubs[0] = 1;
        g.last_hubs[0] = 1;
        g.switches[0] = 25;
        g.routes[0][1] = vec![p.spots.len(), 1];
        let e = evaluate(&c, &g, true);
        let mut game = Game::new(p, &[1, 0]);
        game.agents = agents;
        game.play_day(e.plans[0].clone()).unwrap();
        assert_eq!(e.daily, 2);
        assert_eq!(game.score.daily_brands, e.daily);
        assert_eq!(game.score.udon, e.udon);
    }
    #[test]
    fn late_refill_occurs_at_the_final_step() {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![8],
            map: tester::model::Map {
                height: 1,
                width: 5,
                cells: vec![vec![0; 5]],
            },
            spots: vec![
                tester::model::Spot {
                    brand: 0,
                    pos: 1,
                    stocks: 2,
                },
                tester::model::Spot {
                    brand: 1,
                    pos: 3,
                    stocks: 2,
                },
            ],
            agents: vec![0, 4],
            fuel_limits: 4,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        };
        let solver = Solver::new(p.clone(), 0);
        let agents = vec![
            Agent {
                kind: 1,
                pos: 0,
                fuel: 4,
            },
            Agent {
                kind: 0,
                pos: 4,
                fuel: 1,
            },
        ];
        let c = solver.context(0, &agents, &[]);
        let mut g = blank(&c, 0, 0);
        g.late_hubs[0] = 1;
        g.routes[0][1] = vec![3];
        let e = evaluate(&c, &g, true);
        let mut game = Game::new(p, &[1, 0]);
        game.agents = agents;
        game.play_day(e.plans[0].clone()).unwrap();
        assert_eq!(game.agents[1].fuel, 4);
        assert_eq!(game.score.udon, e.udon);
        assert_eq!(game.score.daily_brands, e.daily);
    }
    #[test]
    fn two_day_routes_carry_fuel_and_collect_each_day() {
        let p = Problem {
            starts_at: 0,
            day_seconds: vec![60, 60],
            day_steps: vec![8, 8],
            map: tester::model::Map {
                height: 1,
                width: 5,
                cells: vec![vec![0; 5]],
            },
            spots: vec![
                tester::model::Spot {
                    brand: 0,
                    pos: 1,
                    stocks: 2,
                },
                tester::model::Spot {
                    brand: 1,
                    pos: 3,
                    stocks: 2,
                },
            ],
            agents: vec![0, 4],
            fuel_limits: 4,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        };
        let solver = Solver::new(p.clone(), 0);
        let agents = vec![
            Agent {
                kind: 1,
                pos: 0,
                fuel: 4,
            },
            Agent {
                kind: 0,
                pos: 4,
                fuel: 1,
            },
        ];
        let mut c = solver.context(0, &agents, &[]);
        let mut nodes = fixed_hubs(&p);
        for agent in &agents {
            if !nodes.contains(&agent.pos) {
                nodes.push(agent.pos);
            }
        }
        c.roads.push(roads(&p, &nodes, &[], false));
        assert_eq!(c.roads.len(), 2);
        let mut g = blank(&c, 0, 0);
        g.late_hubs[0] = 1;
        g.last_hubs[0] = 1;
        g.routes[0][1] = vec![3];
        g.routes[1][1] = vec![0, 1];
        let e = evaluate(&c, &g, true);
        let mut game = Game::new(p, &[1, 0]);
        game.agents = agents;
        for plan in e.plans {
            game.play_day(plan).unwrap();
        }
        assert_eq!(game.score.udon, e.udon);
        assert_eq!(game.score.daily_brands, e.daily);
        assert_eq!(game.score.unique_brands, e.unique);
    }
    #[test]
    fn rolling_horizon_survives_every_protocol_day() {
        let p: Problem =
            serde_json::from_str(include_str!("../official/sample_maps/sample_16.json")).unwrap();
        let mut solver = Solver::new(p.clone(), 901);
        let kinds = solver.select(100);
        let mut game = Game::new(p, &kinds);
        for _ in 0..game.problem.day_steps.len() {
            let plan = solver.plan(game.observation(0), 100, true);
            game.play_day(plan).unwrap();
        }
        assert_eq!(game.day, 5);
    }
    #[test]
    fn compact_decoder_matches_engine_collection_on_random_routes() {
        let mut p: Problem =
            serde_json::from_str(include_str!("../official/sample_maps/sample_16.json")).unwrap();
        p.day_steps.truncate(1);
        p.day_seconds.truncate(1);
        let solver = Solver::new(p.clone(), 0);
        let agents: Vec<_> = p
            .agents
            .iter()
            .enumerate()
            .map(|(i, &pos)| Agent {
                kind: if i == 0 { 1 } else { 0 },
                pos,
                fuel: p.fuel_limits,
            })
            .collect();
        let traffic: Vec<_> = (0..p.map.len())
            .filter(|&pos| p.map.terrain(pos) == 1)
            .map(|pos| Traffic {
                pos,
                status: (pos % 3) as u8,
            })
            .collect();
        let c = solver.context(0, &agents, &traffic);
        let mut rng = Rng(777);
        for trial in 0..300 {
            let mut g = blank(&c, 0, rng.usize(c.hub_count));
            if trial % 2 == 0 {
                g.secondary = 1;
                g.secondary_hubs[0] = [
                    rng.usize(p.spots.len()),
                    rng.usize(p.spots.len()),
                    rng.usize(p.spots.len()),
                ];
                g.secondary_switches[0] = [rng.usize(101) as u16, rng.usize(101) as u16];
            }
            g.late_hubs[0] = rng.usize(c.hub_count);
            g.last_hubs[0] = rng.usize(c.hub_count);
            g.last_switches[0] = rng.usize(101) as u16;
            g.switches[0] = rng.usize(101) as u16;
            for i in 1..agents.len() {
                g.variant[0][i] = rng.usize(3);
                g.routes[0][i] = (0..8)
                    .map(|_| rng.usize(4 * (p.spots.len() + g.refill_count(agents.len()))))
                    .collect()
            }
            let e = evaluate(&c, &g, true);
            let mut game = Game::new(
                p.clone(),
                if g.secondary < agents.len() {
                    &[1, 1, 0, 0]
                } else {
                    &[1, 0, 0, 0]
                },
            );
            game.set_observed_traffic(traffic.clone());
            game.play_day(e.plans[0].clone()).unwrap();
            assert_eq!(game.score.udon, e.udon);
            assert_eq!(game.score.daily_brands, e.daily);
            assert_eq!(game.score.unique_brands, e.unique);
        }
    }
}
