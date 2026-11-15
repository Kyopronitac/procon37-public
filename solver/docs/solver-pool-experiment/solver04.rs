// 第4世代の保存用単一ファイル。元の分割ソースは solver04/ に保存。
// 保存元: solver04/itinerary.rs
mod itinerary {
    // 保存元: solver04/circuit.rs
    mod circuit {
        use super::{Network, Rng};
        use tester::model::Problem;

        #[derive(Clone)]
        struct Circuit {
            groups: Vec<Vec<usize>>,
        }
        #[derive(Clone)]
        struct Ready {
            node: usize,
            time: u16,
            masks: Vec<u32>,
        }
        #[derive(Clone)]
        struct Tour {
            order: Vec<Vec<usize>>,
            time: Vec<u16>,
            value: f64,
        }
        struct Metric {
            n: usize,
            time: Vec<u16>,
            mask: Vec<u32>,
            brands: Vec<u64>,
        }
        impl Metric {
            fn new(p: &Problem, net: &Network) -> Self {
                let mut time = Vec::new();
                let mut mask = Vec::new();
                for a in 0..net.n {
                    for b in 0..net.n {
                        let path = net.path(a, b, 0);
                        time.push(path.time);
                        let mut m = 0;
                        let start = net.spot_at[net.pos[a]];
                        if start >= 0 {
                            m |= 1u32 << start;
                        }
                        for &(to, _, _, _) in &path.moves {
                            let s = net.spot_at[to as usize];
                            if s >= 0 {
                                m |= 1u32 << s;
                            }
                        }
                        mask.push(m);
                    }
                }
                Self {
                    n: net.n,
                    time,
                    mask,
                    brands: p.spots.iter().map(|s| 1u64 << s.brand).collect(),
                }
            }
        }
        impl Circuit {
            fn evaluate(
                &self,
                p: &Problem,
                metric: &Metric,
                ready: &[Ready],
                order: Vec<Vec<usize>>,
                steps: u16,
                seen: u64,
                last: bool,
            ) -> Option<Tour> {
                let mut masks = vec![0u32; p.agents.len()];
                let mut ends = Vec::new();
                let mut times = Vec::new();
                for (g, group) in self.groups.iter().enumerate() {
                    let mut at = ready[g].node;
                    let mut time = ready[g].time;
                    let mut mask = metric.mask[at * metric.n + at];
                    for &dest in &order[g] {
                        time += metric.time[at * metric.n + dest];
                        if time > steps {
                            return None;
                        }
                        mask |= metric.mask[at * metric.n + dest];
                        at = dest;
                    }
                    for &a in group.iter().skip(1) {
                        masks[a] = ready[g].masks[a] | mask;
                    }
                    ends.push(at);
                    times.push(time);
                }
                let mut brands = 0u64;
                let mut udon = 0;
                for (s, spot) in p.spots.iter().enumerate() {
                    let count = masks.iter().filter(|&&m| m >> s & 1 != 0).count() as u32;
                    if count > 0 {
                        brands |= metric.brands[s];
                    }
                    udon += count.min(spot.stocks);
                }
                let mut value = brands.count_ones() as f64 * 100000.0
                    + (brands & !seen).count_ones() as f64 * 1000000.0
                    + udon as f64 * 100.0;
                if !last {
                    for s in 0..p.spots.len() {
                        value -= ends
                            .iter()
                            .map(|&a| metric.time[a * metric.n + s])
                            .min()
                            .unwrap() as f64
                            * 0.5;
                    }
                }
                Some(Tour {
                    order,
                    time: times,
                    value,
                })
            }
            fn search(
                &self,
                p: &Problem,
                metric: &Metric,
                ready: &[Ready],
                steps: u16,
                seen: u64,
                last: bool,
                rng: &mut Rng,
                iterations: usize,
                warm: Option<&[Vec<usize>]>,
            ) -> Tour {
                let empty = vec![Vec::new(); self.groups.len()];
                let mut best = self
                    .evaluate(p, metric, ready, empty, steps, seen, last)
                    .unwrap();
                if let Some(warm) = warm {
                    let mut order = warm.to_vec();
                    for (g, oo) in order.iter_mut().enumerate() {
                        oo.retain(|&s| s != ready[g].node);
                    }
                    if let Some(c) = self.evaluate(p, metric, ready, order, steps, seen, last) {
                        if c.value > best.value {
                            best = c;
                        }
                    }
                }
                // Greedy insertion starts annealing from a feasible set of diverse tours.
                for _ in 0..p.spots.len() * 2 {
                    let mut choice = None;
                    let mut ratio = 0.0;
                    for g in 0..self.groups.len() {
                        for s in 0..p.spots.len() {
                            if best.order[g].contains(&s) {
                                continue;
                            }
                            for k in 0..=best.order[g].len() {
                                let mut order = best.order.clone();
                                order[g].insert(k, s);
                                if let Some(c) =
                                    self.evaluate(p, metric, ready, order, steps, seen, last)
                                {
                                    let cost = c.time[g].saturating_sub(best.time[g]) as f64;
                                    let r = (c.value - best.value) / (cost + 1.0);
                                    if r > ratio {
                                        ratio = r;
                                        choice = Some(c);
                                    }
                                }
                            }
                        }
                    }
                    let Some(c) = choice else {
                        break;
                    };
                    best = c;
                }
                if let Some(warm) = warm {
                    if let Some(c) =
                        self.evaluate(p, metric, ready, warm.to_vec(), steps, seen, last)
                    {
                        if c.value > best.value {
                            best = c;
                        }
                    }
                }
                let mut current = best.clone();
                for it in 0..iterations {
                    let g = rng.n(self.groups.len());
                    let h = rng.n(self.groups.len());
                    let mut order = current.order.clone();
                    let op = rng.n(8);
                    match op {
                        0 | 1 => {
                            let s = rng.n(p.spots.len());
                            if order[g].contains(&s) {
                                continue;
                            }
                            let k = rng.n(order[g].len() + 1);
                            order[g].insert(k, s);
                        }
                        2 => {
                            if order[g].is_empty() {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            order[g].remove(k);
                        }
                        3 => {
                            if order[g].len() < 2 {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            let j = rng.n(order[g].len());
                            order[g].swap(k, j);
                        }
                        4 => {
                            if order[g].len() < 2 {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            let j = rng.n(order[g].len());
                            order[g][k.min(j)..=k.max(j)].reverse();
                        }
                        5 => {
                            if g == h || order[g].is_empty() {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            let s = order[g].remove(k);
                            if order[h].contains(&s) {
                                continue;
                            }
                            let j = rng.n(order[h].len() + 1);
                            order[h].insert(j, s);
                        }
                        6 => {
                            if g == h || order[g].is_empty() || order[h].is_empty() {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            let j = rng.n(order[h].len());
                            let s = order[g][k];
                            order[g][k] = order[h][j];
                            order[h][j] = s;
                        }
                        _ => {
                            if order[g].is_empty() {
                                continue;
                            }
                            let k = rng.n(order[g].len());
                            order[g][k] = rng.n(p.spots.len());
                        }
                    }
                    let Some(c) = self.evaluate(p, metric, ready, order, steps, seen, last) else {
                        continue;
                    };
                    let temp = 45000.0 * 0.00001f64.powf((it % 12000) as f64 / 12000.0) + 1.0;
                    if c.value >= current.value
                        || rng.f() < ((c.value - current.value) / temp).exp()
                    {
                        current = c;
                        if current.value > best.value {
                            best = current.clone();
                        }
                    }
                }
                best
            }
        }

        pub(super) fn supplier_circuit(
            p: &Problem,
            net: &Network,
            start: usize,
            steps: u16,
            rng: &mut Rng,
        ) -> Vec<usize> {
            if p.agents.len() < 2 {
                return Vec::new();
            }
            let solver = Circuit {
                groups: vec![vec![0, 1]],
            };
            let metric = Metric::new(p, net);
            let ready = vec![Ready {
                node: start,
                time: 0,
                masks: vec![0; p.agents.len()],
            }];
            solver
                .search(p, &metric, &ready, steps, 0, true, rng, 10000, None)
                .order
                .remove(0)
        }
    }
    // 保存元: solver04/escort.rs
    mod escort {
        //! Route the supplier through the space-time traces of the patrol fleet.
        use super::{Candidate, Network, Rng, SupportRoute};
        use std::cmp::Reverse;
        use std::collections::{BinaryHeap, HashSet};
        use tester::model::{Agent, Problem};

        #[derive(Clone)]
        struct State {
            pos: usize,
            fuel: [i16; 8],
            debt: u16,
            early: f64,
            rank: f64,
            parent: usize,
            action: i64,
        }

        pub(super) fn optimize(
            p: &Problem,
            net: &Network,
            agents: &[Agent],
            c: &Candidate,
            steps: u16,
            last: bool,
            width: usize,
            rng: &mut Rng,
        ) -> Vec<SupportRoute> {
            let suppliers = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(a, _)| a)
                .collect::<Vec<_>>();
            if suppliers.len() != 1 {
                return c.support.clone();
            }
            let supplier = suppliers[0];
            let cars = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 0)
                .map(|(a, _)| a)
                .collect::<Vec<_>>();
            let n = agents.len();
            let mut positions = vec![[0usize; 8]; steps as usize + 1];
            let mut costs = vec![[0i16; 8]; steps as usize + 1];
            for &a in &cars {
                let mut pos = agents[a].pos;
                let mut t = 0u16;
                positions[0][a] = pos;
                for &act in &c.plan[a] {
                    let dt = if act < 0 {
                        (-act) as u16
                    } else {
                        net.time[pos] as u16
                    };
                    let dest = if act < 0 {
                        pos
                    } else {
                        p.map.neighbor(pos, act as usize).unwrap()
                    };
                    let cost = if act < 0 { 0 } else { net.cost[pos] as i16 };
                    for k in 1..=dt {
                        t += 1;
                        if t > steps {
                            break;
                        }
                        if k == dt {
                            pos = dest;
                            costs[t as usize][a] = cost;
                        }
                        positions[t as usize][a] = pos;
                    }
                    if t >= steps {
                        break;
                    }
                }
                for k in t + 1..=steps {
                    positions[k as usize][a] = pos;
                }
            }
            let mut consumed = vec![[0i16; 8]; steps as usize + 1];
            for t in 1..=steps as usize {
                consumed[t] = consumed[t - 1];
                for &a in &cars {
                    consumed[t][a] += costs[t][a];
                }
            }
            let limit = p.fuel_limits as i16;
            let align_end = !last && rng.n(3) == 0;
            let mut end_time = vec![Vec::new(); n];
            let mut end_fuel = vec![Vec::new(); n];
            if align_end {
                for &a in &cars {
                    let end = positions[steps as usize][a];
                    for mode in 0..2 {
                        let cost = if mode == 0 { &net.time } else { &net.cost };
                        let row = if let Some(node) = net.pos.iter().position(|&pos| pos == end) {
                            let known = if mode == 0 {
                                &net.dist_t[node]
                            } else {
                                &net.dist_f[node]
                            };
                            // Costs belong to the departure cell. Reversing a path
                            // changes only the two endpoint costs.
                            known
                                .iter()
                                .enumerate()
                                .map(|(pos, &d)| {
                                    (d as i32 + cost[end] as i32 - cost[pos] as i32)
                                        .clamp(0, u16::MAX as i32)
                                        as u16
                                })
                                .collect()
                        } else {
                            let mut row = vec![u16::MAX; p.map.len()];
                            let mut queue = BinaryHeap::new();
                            row[end] = 0;
                            queue.push(Reverse((0u16, end)));
                            while let Some(Reverse((d, pos))) = queue.pop() {
                                if d != row[pos] {
                                    continue;
                                }
                                for to in p.map.neighbors(pos).filter(|&to| p.map.terrain(to) != 3)
                                {
                                    let next = d + cost[pos] as u16;
                                    if next < row[to] {
                                        row[to] = next;
                                        queue.push(Reverse((next, to)));
                                    }
                                }
                            }
                            row
                        };
                        if mode == 0 {
                            end_time[a] = row;
                        } else {
                            end_fuel[a] = row;
                        }
                    }
                }
            }
            let endpoint = |pos: usize, fuel: &[i16; 8]| -> f64 {
                cars.iter()
                    .map(|&a| {
                        let f = fuel[a].max(0) as f64;
                        let end = positions[steps as usize][a];
                        let needed = end_fuel[a][pos] as f64;
                        let near = end_time[a][pos] as f64;
                        let urgency = (1.0 - f / (limit as f64 * 0.70)).clamp(0.0, 1.0);
                        let mobility = (f / (needed + 1.0)).min(0.5);
                        let escape = net.escape_fuel[end] as f64;
                        let stranded = if escape > 0.0 {
                            (1.0 - f / escape).max(0.0) * 60.0
                        } else {
                            0.0
                        };
                        -stranded
                            - near * (1.0 - mobility) * urgency * 1.25
                            - if n >= 6 {
                                ((needed - f).max(0.0) * 4.0).min(120.0) * urgency
                            } else {
                                0.0
                            }
                            + f.min((limit as f64 * 0.5).min(30.0)) * 0.5
                            + 0.5 * f / limit as f64
                    })
                    .sum()
            };
            let mut initial = State {
                pos: agents[supplier].pos,
                fuel: [0; 8],
                debt: 0,
                early: 0.0,
                rank: 0.,
                parent: usize::MAX,
                action: 0,
            };
            for &a in &cars {
                initial.fuel[a] = agents[a].fuel as i16;
            }
            let mut buckets = vec![Vec::new(); steps as usize + 1];
            buckets[0].push(initial);
            let mut arena = Vec::<State>::new();
            let mut best = 0;
            let mut best_value = f64::NEG_INFINITY;
            let weights = (0..n).map(|_| 0.75 + rng.f() * 0.5).collect::<Vec<_>>();
            for t in 0..=steps {
                let mut states = std::mem::take(&mut buckets[t as usize]);
                states.sort_unstable_by(|a, b| b.rank.total_cmp(&a.rank));
                let mut seen = HashSet::new();
                let mut count = 0;
                for b in states {
                    let key = (b.pos, b.debt, b.fuel.map(|f| f / 6));
                    if !seen.insert(key) {
                        continue;
                    }
                    count += 1;
                    if count > width {
                        break;
                    }
                    let id = arena.len();
                    arena.push(b.clone());
                    if t == steps {
                        let value = -(b.debt as f64) * 10000.0
                            + b.early * if align_end { 0.15 } else { 0.5 }
                            + if align_end {
                                endpoint(b.pos, &b.fuel)
                            } else {
                                (if last { 0.05 } else { 1.0 })
                                    * cars
                                        .iter()
                                        .map(|&a| weights[a] * (b.fuel[a] as f64).min(limit as f64))
                                        .sum::<f64>()
                            };
                        if value > best_value {
                            best_value = value;
                            best = id;
                        }
                        continue;
                    }
                    for action in -1i64..6 {
                        let (dest, dt) = if action < 0 {
                            (b.pos, 1)
                        } else {
                            let Some(dest) = p.map.neighbor(b.pos, action as usize) else {
                                continue;
                            };
                            if p.map.terrain(dest) == 3 {
                                continue;
                            }
                            (dest, net.time[b.pos] as u16)
                        };
                        let tt = t + dt;
                        if tt > steps {
                            continue;
                        }
                        let mut q = b.clone();
                        q.pos = dest;
                        q.parent = id;
                        q.action = action;
                        for k in t + 1..=tt {
                            let at = if k == tt { dest } else { b.pos };
                            for &a in &cars {
                                q.fuel[a] -= costs[k as usize][a];
                                if q.fuel[a] < 0 {
                                    q.debt += (-q.fuel[a]) as u16;
                                    q.fuel[a] = 0;
                                }
                                if at == positions[k as usize][a] {
                                    q.early += (limit - q.fuel[a]) as f64 * (steps - k) as f64
                                        / steps as f64;
                                    q.fuel[a] = limit;
                                }
                            }
                        }
                        let mut potential = 0.0f64;
                        let mut reserves = 0.0;
                        for &a in &cars {
                            reserves += weights[a] * (q.fuel[a] as f64).min(limit as f64);
                            for lead in [4usize, 10, 20, 35] {
                                let future = (tt as usize + lead).min(steps as usize);
                                let target = positions[future][a];
                                let dx = (dest % p.map.width).abs_diff(target % p.map.width);
                                let dy = (dest / p.map.width).abs_diff(target / p.map.width);
                                let distance = dx.max(dy) as f64;
                                if distance > (future - tt as usize) as f64 + 2.0 {
                                    continue;
                                }
                                let expected =
                                    q.fuel[a] - (consumed[future][a] - consumed[tt as usize][a]);
                                let need = (limit as f64 * 0.75 - expected as f64).max(0.0);
                                potential =
                                    potential.max(weights[a] * need * 4.0 / (distance + 4.0));
                            }
                        }
                        let resource = if align_end {
                            let blend = (tt as f64 / steps as f64).powi(2);
                            let mut future_fuel = q.fuel;
                            for &a in &cars {
                                future_fuel[a] -=
                                    consumed[steps as usize][a] - consumed[tt as usize][a];
                            }
                            reserves * (1.0 - blend) + endpoint(dest, &future_fuel) * blend
                        } else {
                            reserves * if last { 0.05 } else { 1.0 }
                        };
                        q.rank = -(q.debt as f64) * 10000.0
                            + q.early * if align_end { 0.15 } else { 0.5 }
                            + resource
                            + potential;
                        buckets[tt as usize].push(q);
                    }
                }
            }
            if !best_value.is_finite() {
                return c.support.clone();
            }
            let mut actions = Vec::new();
            let mut id = best;
            while arena[id].parent != usize::MAX {
                actions.push(arena[id].action);
                id = arena[id].parent;
            }
            actions.reverse();
            let mut compact = Vec::new();
            for action in actions {
                if action < 0 && compact.last().is_some_and(|x| *x < 0) {
                    *compact.last_mut().unwrap() -= 1;
                } else {
                    compact.push(action);
                }
            }
            let mut routes = c.support.clone();
            routes[0].direct = Some(compact);
            routes
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn endpoint_routing_refills_an_idle_low_fuel_patrol() {
                let p = Problem {
                    starts_at: 0,
                    day_seconds: vec![60, 60],
                    day_steps: vec![10, 10],
                    map: tester::model::Map {
                        width: 7,
                        height: 3,
                        cells: vec![vec![0; 7]; 3],
                    },
                    spots: vec![
                        tester::model::Spot {
                            brand: 0,
                            pos: 0,
                            stocks: 2,
                        },
                        tester::model::Spot {
                            brand: 1,
                            pos: 6,
                            stocks: 2,
                        },
                    ],
                    agents: vec![0, 6, 10],
                    fuel_limits: 6,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let mut game = tester::engine::Game::new(p.clone(), &[0, 0, 1]);
                game.agents[0].fuel = 1;
                let agents = game.agents.clone();
                let net = Network::new(&p, &agents, &[]);
                let source = Candidate {
                    plan: game.waiting_plan(),
                    masks: vec![1, 2, 0],
                    end: agents.clone(),
                    support: vec![SupportRoute {
                        stops: vec![],
                        direct: Some(vec![-10]),
                    }],
                    score: 0.0,
                };
                let seed = (0..100).find(|&seed| Rng(seed).n(3) == 0).unwrap();
                let support = optimize(&p, &net, &agents, &source, 10, false, 200, &mut Rng(seed));
                let supply = super::super::Supply::build(&p, &agents, &net, &support, 10);
                let mut plan = source.plan;
                plan[2] = supply.plans[2].clone();
                game.play_day(plan).unwrap();
                assert_eq!(game.agents[0].fuel, 6);
                assert_eq!(game.score.daily_brands, 2);
                assert_eq!(game.score.udon, 2);
                assert_eq!(game.agents[2].pos, supply.end[2]);
            }
        }
    }
    // 保存元: solver04/joint.rs
    mod joint {
        //! Joint search through temporarily fuel-infeasible patrol and supplier tours.
        use super::tour::{Visit, extract};
        use super::{Candidate, Network, Rng, Solver, Supply};
        use std::time::{Duration, Instant};
        use tester::model::{Agent, Traffic};

        struct Evaluation {
            candidate: Candidate,
            shortage: u32,
        }

        fn evaluate(
            solver: &Solver,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            tours: &[Vec<Visit>],
            template: &Candidate,
            day: usize,
            steps: u16,
        ) -> Evaluation {
            let p = &solver.p;
            let mut c = template.clone();
            c.plan = supply.plans.clone();
            c.support = agents
                .iter()
                .enumerate()
                .filter(|(_, ag)| ag.kind == 1)
                .map(|(a, _)| super::SupportRoute {
                    stops: Vec::new(),
                    direct: Some(supply.plans[a].clone()),
                })
                .collect();
            c.masks.fill(0);
            c.end = agents.to_vec();
            let mut shortage = 0;
            for (a, ag) in agents.iter().enumerate() {
                if ag.kind == 1 {
                    c.end[a].pos = supply.end[a];
                    continue;
                }
                let mut node = p.spots.len() + a;
                let mut pos = ag.pos;
                let mut t = 0u16;
                let mut fuel = ag.fuel as u16;
                let mut mask = 0u32;
                let mut plan = Vec::new();
                let collect = |pos: usize, mask: &mut u32| {
                    let spot = net.spot_at[pos];
                    if spot >= 0 {
                        *mask |= 1 << spot;
                    }
                };
                'visits: for visit in &tours[a] {
                    let path = net.path(node, visit.node, visit.mode);
                    for &(to, dir, dt, cost) in &path.moves {
                        let dt = dt as u16;
                        let cost = cost as u16;
                        if t + dt > steps {
                            break 'visits;
                        }
                        if dt > 1 {
                            fuel = supply.wait_fuel(pos, t, t + dt - 1, fuel, p.fuel_limits as u16);
                            collect(pos, &mut mask);
                        }
                        shortage += cost.saturating_sub(fuel) as u32;
                        fuel = fuel.saturating_sub(cost);
                        t += dt;
                        pos = to as usize;
                        if supply.at[t as usize][pos] {
                            fuel = p.fuel_limits as u16;
                        }
                        collect(pos, &mut mask);
                        plan.push(dir as i64);
                    }
                    let until = visit.earliest.min(steps);
                    if t < until {
                        fuel = supply.wait_fuel(pos, t, until, fuel, p.fuel_limits as u16);
                        collect(pos, &mut mask);
                        plan.push(-((until - t) as i64));
                        t = until;
                    }
                    node = visit.node;
                }
                if t < steps {
                    fuel = supply.wait_fuel(pos, t, steps, fuel, p.fuel_limits as u16);
                    collect(pos, &mut mask);
                    plan.push(-((steps - t) as i64));
                }
                c.plan[a] = plan;
                c.masks[a] = mask;
                c.end[a].pos = pos;
                c.end[a].fuel = fuel as u32;
            }
            c.score = solver.local_value(&c.masks, &c.end, net, supply, day, steps);
            Evaluation {
                candidate: c,
                shortage,
            }
        }

        fn insertion(
            tour: &[Visit],
            node: usize,
            start: usize,
            net: &Network,
            rng: &mut Rng,
        ) -> usize {
            if rng.n(4) == 0 {
                return rng.n(tour.len() + 1);
            }
            (0..=tour.len())
                .min_by_key(|&i| {
                    let before = if i == 0 { start } else { tour[i - 1].node };
                    net.path(before, node, 0).time as i32
                        + if i < tour.len() {
                            net.path(node, tour[i].node, 0).time as i32
                                - net.path(before, tour[i].node, 0).time as i32
                        } else {
                            0
                        }
                })
                .unwrap()
        }

        pub(super) fn improve(
            solver: &mut Solver,
            agents: &[Agent],
            original_net: &Network,
            original_supply: &Supply,
            c: &mut Candidate,
            day: usize,
            steps: u16,
            iterations: usize,
        ) {
            solver.tour_stats[5] += 1;
            let started = Instant::now();
            let initial_score = c.score;
            let (nodes, mut tours) =
                extract(&solver.p, original_net, original_supply, agents, c, steps);
            let traffic = (0..solver.p.map.len())
                .filter(|&v| solver.p.map.terrain(v) == 1)
                .map(|pos| Traffic {
                    pos,
                    status: match original_net.time[pos] {
                        1 => 0,
                        2 => 1,
                        _ => 2,
                    },
                })
                .collect::<Vec<_>>();
            let net = Network::from_positions(&solver.p, nodes, &traffic);
            let cars = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 0)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            let mut support = c.support.clone();
            let mut supply = Supply::build(&solver.p, agents, &net, &support, steps);
            let mut current = evaluate(solver, agents, &net, &supply, &tours, c, day, steps);
            let mut saved_tours = tours.clone();
            let mut saved_support = support.clone();
            if current.shortage == 0 && current.candidate.score > c.score {
                *c = current.candidate.clone();
            }
            for it in 0..iterations {
                if it % 32 == 0 && started.elapsed() >= Duration::from_millis(120) {
                    break;
                }
                let phase = (it % 700) as f64 / 700.0;
                if it % 700 == 699 {
                    tours = saved_tours.clone();
                    support = saved_support.clone();
                    supply = Supply::build(&solver.p, agents, &net, &support, steps);
                    current = evaluate(solver, agents, &net, &supply, &tours, c, day, steps);
                }
                let op = solver.rng.n(12);
                let mut proposed = tours.clone();
                let mut routes = support.clone();
                let replacement_supply;
                if op < 2 {
                    solver.mutate_support(&mut routes, agents, &net, steps, it);
                    replacement_supply =
                        Some(Supply::build(&solver.p, agents, &net, &routes, steps));
                } else {
                    replacement_supply = None;
                    let a = cars[solver.rng.n(cars.len())];
                    let tour = &mut proposed[a];
                    match op {
                        2 | 3 => {
                            if tour.len() > 42 {
                                continue;
                            }
                            let (node, earliest) = if op == 3 && !supply.windows.is_empty() {
                                let &(node, arrive, _) =
                                    &supply.windows[solver.rng.n(supply.windows.len())];
                                (node, arrive)
                            } else {
                                (solver.rng.n(solver.p.spots.len()), 0)
                            };
                            if tour.iter().any(|v| v.node == node) && solver.rng.n(3) != 0 {
                                continue;
                            }
                            let at = insertion(
                                tour,
                                node,
                                solver.p.spots.len() + a,
                                &net,
                                &mut solver.rng,
                            );
                            tour.insert(
                                at,
                                Visit {
                                    node,
                                    earliest,
                                    mode: solver.rng.n(3),
                                },
                            );
                        }
                        4 => {
                            if tour.is_empty() {
                                continue;
                            }
                            tour.remove(solver.rng.n(tour.len()));
                        }
                        5 | 7 => {
                            if tour.len() < 2 {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            let j = solver.rng.n(tour.len());
                            if op == 5 {
                                tour.swap(i, j);
                            } else {
                                tour[i.min(j)..=i.max(j)].reverse();
                            }
                        }
                        6 => {
                            if tour.len() < 2 {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            let visit = tour.remove(i);
                            let j = insertion(
                                tour,
                                visit.node,
                                solver.p.spots.len() + a,
                                &net,
                                &mut solver.rng,
                            );
                            tour.insert(j, visit);
                        }
                        8 => {
                            if tour.is_empty() {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            let old = tour[i].node;
                            let targets = (0..solver.p.spots.len())
                                .filter(|&s| {
                                    old >= solver.p.spots.len()
                                        || solver.p.spots[s].brand == solver.p.spots[old].brand
                                })
                                .collect::<Vec<_>>();
                            tour[i].node = targets[solver.rng.n(targets.len())];
                            tour[i].earliest = 0;
                        }
                        9 => {
                            if tour.is_empty() {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            let options = supply
                                .windows
                                .iter()
                                .filter(|&&(node, _, _)| node == tour[i].node)
                                .collect::<Vec<_>>();
                            tour[i].earliest = if solver.rng.n(3) == 0 {
                                0
                            } else if !options.is_empty() {
                                options[solver.rng.n(options.len())].1
                            } else {
                                tour[i]
                                    .earliest
                                    .saturating_add_signed(solver.rng.n(17) as i16 - 8)
                                    .min(steps)
                            };
                        }
                        10 => {
                            if tour.is_empty() {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            tour[i].mode = solver.rng.n(3);
                        }
                        _ => {
                            if tour.is_empty() {
                                continue;
                            }
                            let b = cars[solver.rng.n(cars.len())];
                            if a == b {
                                continue;
                            }
                            let i = solver.rng.n(tour.len());
                            let mut visit = tour.remove(i);
                            visit.earliest = 0;
                            let j = insertion(
                                &proposed[b],
                                visit.node,
                                solver.p.spots.len() + b,
                                &net,
                                &mut solver.rng,
                            );
                            proposed[b].insert(j, visit);
                        }
                    }
                }
                let next_supply = replacement_supply.as_ref().unwrap_or(&supply);
                let trial = evaluate(solver, agents, &net, next_supply, &proposed, c, day, steps);
                let penalty = 20.0 + phase * 130.0;
                let delta = trial.candidate.score
                    - current.candidate.score
                    - (trial.shortage as f64 - current.shortage as f64) * penalty;
                let temperature = 4.0 + 75.0 * (1.0 - phase).powi(2);
                if delta >= 0.0 || solver.rng.f() < (delta / temperature).exp() {
                    current = trial;
                    tours = proposed;
                    support = routes;
                    if let Some(next) = replacement_supply {
                        supply = next;
                    }
                    if current.shortage == 0 && current.candidate.score > c.score {
                        *c = current.candidate.clone();
                        saved_tours = tours.clone();
                        saved_support = support.clone();
                    }
                }
            }
            if c.score > initial_score {
                solver.tour_stats[6] += 1;
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn shortage_is_charged_before_same_tick_destination_refill() {
                let p = tester::model::Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![2],
                    map: tester::model::Map {
                        width: 2,
                        height: 1,
                        cells: vec![vec![0, 0]],
                    },
                    spots: vec![tester::model::Spot {
                        brand: 0,
                        pos: 1,
                        stocks: 1,
                    }],
                    agents: vec![0, 1],
                    fuel_limits: 4,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let solver = Solver::new(p.clone(), 1);
                let agents = vec![
                    Agent {
                        kind: 0,
                        pos: 0,
                        fuel: 0,
                    },
                    Agent {
                        kind: 1,
                        pos: 1,
                        fuel: 4,
                    },
                ];
                let net = Network::new(&p, &agents, &[]);
                let support = vec![super::super::SupportRoute {
                    stops: vec![],
                    direct: Some(vec![-2]),
                }];
                let supply = Supply::build(&p, &agents, &net, &support, 2);
                let template = Candidate {
                    plan: vec![],
                    score: 0.,
                    masks: vec![0; 2],
                    end: agents.clone(),
                    support,
                };
                let tours = vec![
                    vec![Visit {
                        node: 0,
                        earliest: 0,
                        mode: 0,
                    }],
                    vec![],
                ];
                let invalid = evaluate(&solver, &agents, &net, &supply, &tours, &template, 0, 2);
                assert_eq!(invalid.shortage, 1);
                let mut fueled = agents.clone();
                fueled[0].fuel = 1;
                let valid = evaluate(&solver, &fueled, &net, &supply, &tours, &template, 0, 2);
                assert_eq!(valid.shortage, 0);
                let mut game = tester::engine::Game::new(p, &[0, 1]);
                game.agents = fueled;
                game.play_day(valid.candidate.plan).unwrap();
                assert_eq!(game.score.udon, 1);
                assert_eq!(game.agents, valid.candidate.end);
            }

            #[test]
            fn extra_refill_nodes_are_exported_as_executable_supplier_actions() {
                let p = tester::model::Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![12],
                    map: tester::model::Map {
                        width: 4,
                        height: 1,
                        cells: vec![vec![0, 1, 0, 0]],
                    },
                    spots: vec![tester::model::Spot {
                        brand: 0,
                        pos: 3,
                        stocks: 1,
                    }],
                    agents: vec![0, 2],
                    fuel_limits: 8,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let agents = tester::engine::Game::new(p.clone(), &[0, 1]).agents;
                let original = Network::new(&p, &agents, &[]);
                let mut positions = original.pos.clone();
                let extra = positions.len();
                positions.push(1);
                let net = Network::from_positions(&p, positions, &[]);
                let support = vec![super::super::SupportRoute {
                    stops: vec![(extra, 4), (0, 12)],
                    direct: None,
                }];
                let supply = Supply::build(&p, &agents, &net, &support, 12);
                let template = Candidate {
                    plan: vec![],
                    score: 0.,
                    masks: vec![0; 2],
                    end: agents.clone(),
                    support,
                };
                let tours = vec![
                    vec![
                        Visit {
                            node: extra,
                            earliest: 4,
                            mode: 0,
                        },
                        Visit {
                            node: 0,
                            earliest: 0,
                            mode: 0,
                        },
                    ],
                    vec![],
                ];
                let solver = Solver::new(p.clone(), 1);
                let evaluated = evaluate(&solver, &agents, &net, &supply, &tours, &template, 0, 12);
                assert_eq!(evaluated.shortage, 0);
                assert!(evaluated.candidate.support[0].direct.is_some());
                let rebuilt =
                    Supply::build(&p, &agents, &original, &evaluated.candidate.support, 12);
                assert_eq!(rebuilt.at, supply.at);
                let mut game = tester::engine::Game::new(p, &[0, 1]);
                game.play_day(evaluated.candidate.plan).unwrap();
                assert_eq!(game.agents, evaluated.candidate.end);
            }
        }
    }
    // 保存元: solver04/patrol_first.rs
    mod patrol_first {
        //! Generate broad patrol tours first, then fit a real supplier to them.
        use super::{Candidate, Network, Solver, Supply, escort};
        use tester::model::Agent;

        pub(super) fn build(
            solver: &mut Solver,
            agents: &[Agent],
            net: &Network,
            source: &Candidate,
            day: usize,
            steps: u16,
        ) -> Candidate {
            // On small teams, keeping part of the incumbent avoids making every
            // patrol depend on an entirely different supplier schedule at once.
            let partial = agents.len() <= 5 && solver.rng.n(2) == 0;
            build_mode(solver, agents, net, source, day, steps, partial)
        }

        fn build_mode(
            solver: &mut Solver,
            agents: &[Agent],
            net: &Network,
            source: &Candidate,
            day: usize,
            steps: u16,
            partial: bool,
        ) -> Candidate {
            solver.tour_stats[9] += 1;
            let mut relaxed = Supply::build(&solver.p, agents, net, &source.support, steps);
            // These refills exist only while generating a proposal. The submitted
            // patrols are retimed and repaired against the actual supplier below.
            for time in 0..=steps {
                if time > 0 {
                    relaxed.at[time as usize].fill(true);
                }
                relaxed.next[time as usize].fill(if time < steps { time + 1 } else { u16::MAX });
            }
            let mut charged = agents.to_vec();
            for agent in &mut charged {
                agent.fuel = solver.p.fuel_limits;
            }
            let tag = solver.rng.n(1_000_000);
            let proposal = if partial {
                let mut proposal = source.clone();
                solver.rebuild_subset(&mut proposal, &charged, net, &relaxed, day, steps, tag, 65);
                proposal
            } else {
                solver.build_candidate(
                    &charged,
                    net,
                    &relaxed,
                    source.support.clone(),
                    day,
                    steps,
                    tag,
                    45,
                )
            };
            let support = escort::optimize(
                &solver.p,
                net,
                agents,
                &proposal,
                steps,
                day + 1 == solver.p.day_steps.len(),
                240,
                &mut solver.rng,
            );
            let supply = Supply::build(&solver.p, agents, net, &support, steps);
            let candidate = solver
                .preserve_patrols(&proposal, agents, net, &supply, support, day, steps)
                .unwrap();
            if candidate.score > source.score {
                solver.tour_stats[10] += 1;
            }
            candidate
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            #[ignore = "optional diagnostic over an explicitly selected own replay"]
            fn broad_individual_route_probe() {
                let path = std::env::var("PROCON37_PATROL_REPLAY").unwrap();
                let replay: tester::model::Replay =
                    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                let day: usize = std::env::var("PROCON37_PROBE_DAY")
                    .unwrap()
                    .parse()
                    .unwrap();
                let record = &replay.days[day];
                let agents = &record.frames[0].agents;
                let net = Network::new(&replay.case.problem, agents, &record.traffics);
                let mut solver = Solver::new(replay.case.problem.clone(), 91);
                for d in &replay.days[..day] {
                    for f in &d.frames {
                        for e in &f.events {
                            if let tester::model::Event::Collect { brand, .. } = e {
                                solver.seen |= 1 << brand;
                            }
                        }
                    }
                }
                let mut source = Candidate {
                    plan: record.plan.clone(),
                    masks: vec![0; agents.len()],
                    end: record.frames.last().unwrap().agents.clone(),
                    score: 0.0,
                    support: agents
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.kind == 1)
                        .map(|(a, _)| super::super::SupportRoute {
                            stops: vec![],
                            direct: Some(record.plan[a].clone()),
                        })
                        .collect(),
                };
                for frame in record.frames.iter().skip(1) {
                    for (a, agent) in frame.agents.iter().enumerate() {
                        let s = net.spot_at[agent.pos];
                        if agent.kind == 0 && s >= 0 {
                            source.masks[a] |= 1 << s;
                        }
                    }
                }
                let steps = record.steps as u16;
                let supply = Supply::build(&solver.p, agents, &net, &source.support, steps);
                source.score =
                    solver.local_value(&source.masks, &source.end, &net, &supply, day, steps);
                for (a, agent) in agents.iter().enumerate() {
                    if agent.kind != 0 {
                        continue;
                    }
                    let mut used = vec![0u8; solver.p.spots.len()];
                    let mut brands = 0;
                    for (b, &mask) in source.masks.iter().enumerate() {
                        if b == a {
                            continue;
                        }
                        for (s, spot) in solver.p.spots.iter().enumerate() {
                            if mask >> s & 1 != 0 {
                                used[s] += 1;
                                brands |= 1 << spot.brand;
                            }
                        }
                    }
                    let started = std::time::Instant::now();
                    let route = solver.cell_route(
                        a, agents, &net, &supply, day, steps, &used, brands, 2400, 0, 0,
                    );
                    let mut candidate = source.clone();
                    candidate.plan[a] = route.plan;
                    candidate.masks[a] = route.mask;
                    candidate.end[a].pos = route.pos;
                    candidate.end[a].fuel = route.fuel as u32;
                    candidate.score = solver.local_value(
                        &candidate.masks,
                        &candidate.end,
                        &net,
                        &supply,
                        day,
                        steps,
                    );
                    let mut game = tester::engine::Game::new(solver.p.clone(), &replay.kinds);
                    game.day = day;
                    game.agents = agents.clone();
                    game.set_observed_traffic(record.traffics.clone());
                    game.play_day(candidate.plan.clone()).unwrap();
                    let (bb, food) = solver.statistics(&candidate);
                    assert_eq!(game.agents, candidate.end);
                    assert_eq!(game.score.daily_brands, bb.count_ones() as usize);
                    assert_eq!(game.score.udon, food);
                    eprintln!(
                        "route_probe day={} car={} delta={:.1} food={}->{} elapsed_ms={}",
                        day,
                        a,
                        candidate.score - source.score,
                        solver.statistics(&source).1,
                        food,
                        started.elapsed().as_millis()
                    );
                }
            }
            #[test]
            #[ignore = "optional diagnostic over an explicitly selected own replay"]
            fn compare_repair_variants_on_saved_observations() {
                let path = std::env::var("PROCON37_PATROL_REPLAY").expect("path to an own replay");
                let replay: tester::model::Replay =
                    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                let mut seen = 0u64;
                for (day, record) in replay.days.iter().enumerate() {
                    let agents = &record.frames[0].agents;
                    let net = Network::new(&replay.case.problem, agents, &record.traffics);
                    let mut masks = vec![0u32; agents.len()];
                    for frame in record.frames.iter().skip(1) {
                        for (a, agent) in frame.agents.iter().enumerate() {
                            let s = net.spot_at[agent.pos];
                            if agent.kind == 0 && s >= 0 {
                                masks[a] |= 1 << s;
                            }
                        }
                    }
                    let support = agents
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.kind == 1)
                        .map(|(a, _)| super::super::SupportRoute {
                            stops: vec![],
                            direct: Some(record.plan[a].clone()),
                        })
                        .collect::<Vec<_>>();
                    let supply = Supply::build(
                        &replay.case.problem,
                        agents,
                        &net,
                        &support,
                        record.steps as u16,
                    );
                    for partial in [false, true] {
                        let mut solver = Solver::new(replay.case.problem.clone(), 91 + day as u64);
                        solver.seen = seen;
                        let mut source = Candidate {
                            plan: record.plan.clone(),
                            score: 0.,
                            masks: masks.clone(),
                            end: record.frames.last().unwrap().agents.clone(),
                            support: support.clone(),
                        };
                        source.score = solver.local_value(
                            &source.masks,
                            &source.end,
                            &net,
                            &supply,
                            day,
                            record.steps as u16,
                        );
                        let initial = source.score;
                        let before_food = solver.statistics(&source).1;
                        let started = std::time::Instant::now();
                        let mut improved = 0;
                        for _ in 0..10 {
                            let candidate = build_mode(
                                &mut solver,
                                agents,
                                &net,
                                &source,
                                day,
                                record.steps as u16,
                                partial,
                            );
                            let mut check = tester::engine::Game::new(
                                replay.case.problem.clone(),
                                &replay.kinds,
                            );
                            check.day = day;
                            check.agents = agents.clone();
                            check.set_observed_traffic(record.traffics.clone());
                            check.play_day(candidate.plan.clone()).unwrap();
                            let (brands, food) = solver.statistics(&candidate);
                            assert_eq!(check.agents, candidate.end);
                            assert_eq!(check.score.daily_brands, brands.count_ones() as usize);
                            assert_eq!(check.score.udon, food);
                            if candidate.score > source.score {
                                source = candidate;
                                improved += 1;
                            }
                        }
                        eprintln!(
                            "fixed_day={} partial={} improvements={} delta={:.1} food={}->{} elapsed_ms={}",
                            day,
                            partial,
                            improved,
                            source.score - initial,
                            before_food,
                            solver.statistics(&source).1,
                            started.elapsed().as_millis()
                        );
                    }
                    for frame in &record.frames {
                        for event in &frame.events {
                            if let tester::model::Event::Collect { brand, .. } = event {
                                seen |= 1 << brand;
                            }
                        }
                    }
                }
            }
            #[test]
            fn relaxed_proposals_are_repaired_with_real_fuel() {
                let p = tester::model::Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![14],
                    map: tester::model::Map {
                        width: 5,
                        height: 3,
                        cells: vec![vec![0; 5]; 3],
                    },
                    spots: [(0, 1, 2), (1, 4, 1), (2, 7, 2), (3, 13, 1)]
                        .into_iter()
                        .map(|(brand, pos, stocks)| tester::model::Spot { brand, pos, stocks })
                        .collect(),
                    agents: vec![0, 3, 10],
                    fuel_limits: 4,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let mut game = tester::engine::Game::new(p.clone(), &[0, 0, 1]);
                game.agents[0].fuel = 1;
                game.agents[1].fuel = 0;
                let agents = game.agents.clone();
                let net = Network::new(&p, &agents, &[]);
                let mut solver = Solver::new(p, 19);
                let source = Candidate {
                    plan: game.waiting_plan(),
                    score: 0.0,
                    masks: vec![0; 3],
                    end: agents.clone(),
                    support: vec![super::super::SupportRoute {
                        stops: vec![],
                        direct: Some(vec![-14]),
                    }],
                };
                let candidate = build(&mut solver, &agents, &net, &source, 0, 14);
                let (brands, food) = solver.statistics(&candidate);
                game.play_day(candidate.plan).unwrap();
                assert_eq!(game.agents, candidate.end);
                assert_eq!(game.score.daily_brands, brands.count_ones() as usize);
                assert_eq!(game.score.udon, food);
            }
        }
    }
    // 保存元: solver04/route_set.rs
    mod route_set {
        //! Choose a compatible team from independently feasible route alternatives.
        use super::{Candidate, CarRoute, Network, Solver, Supply};
        use tester::model::Agent;

        struct OptionRoute {
            route: CarRoute,
            endpoint: f64,
        }

        fn value(solver: &Solver, pools: &[Vec<OptionRoute>], choice: &[usize], day: usize) -> f64 {
            let mut masks = [0u32; 8];
            let mut score = 0.0;
            for (i, (pool, &index)) in pools.iter().zip(choice).enumerate() {
                let route = &pool[index];
                masks[i] = route.route.mask;
                score += route.endpoint;
            }
            let (brands, food) = solver.mask_statistics(&masks[..choice.len()]);
            score
                + brands.count_ones() as f64 * 1000.0
                + (brands & !solver.seen).count_ones() as f64 * solver.fresh_bonus(day)
                + food as f64 * 10.0
        }

        pub(super) fn improve(
            solver: &mut Solver,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            c: &mut Candidate,
            day: usize,
            steps: u16,
        ) {
            solver.tour_stats[7] += 1;
            let original = c.score;
            let cars = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 0)
                .map(|(a, _)| a)
                .collect::<Vec<_>>();
            let mut brand_spots = [0u32; 64];
            for (s, spot) in solver.p.spots.iter().enumerate() {
                brand_spots[spot.brand] |= 1 << s;
            }
            let mut transfers = Vec::new();
            for (brand, &mask) in brand_spots.iter().enumerate() {
                if mask == 0 {
                    continue;
                }
                let owners = cars
                    .iter()
                    .enumerate()
                    .filter(|&(_, &a)| c.masks[a] & mask != 0)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if owners.len() != 1 {
                    continue;
                }
                let owner = owners[0];
                for (i, &a) in cars.iter().enumerate() {
                    if i == owner {
                        continue;
                    }
                    let distance = solver
                        .p
                        .spots
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| s.brand == brand)
                        .map(|(s, _)| net.dist_t[s][agents[a].pos])
                        .min()
                        .unwrap();
                    if distance < steps {
                        transfers.push((brand, owner, i));
                    }
                }
            }
            let focus = if cars.len() <= 4 && !transfers.is_empty() && solver.rng.n(4) != 0 {
                Some(transfers[solver.rng.n(transfers.len())])
            } else {
                None
            };
            let mut pools = Vec::new();
            let used = vec![0u8; solver.p.spots.len()];
            for &a in &cars {
                solver.captured_routes = Some(Vec::new());
                let tag = solver.rng.n(1_000_000);
                let partners = cars
                    .iter()
                    .filter(|&&b| b != a)
                    .fold(0u16, |m, &b| m | (1 << b));
                let route = solver.cell_route(
                    a, agents, net, supply, day, steps, &used, 0, 220, tag, partners,
                );
                let mut options = solver.captured_routes.take().unwrap();
                options.push(route);
                // A route that repeats brands can still be valuable once the other cars
                // cover them. Keep food-oriented alternatives alongside coverage routes.
                solver.captured_routes = Some(Vec::new());
                let all_brands = solver.p.spots.iter().fold(0u64, |m, s| m | (1 << s.brand));
                let food_route = solver.cell_route(
                    a, agents, net, supply, day, steps, &used, all_brands, 140, tag, 0,
                );
                options.extend(solver.captured_routes.take().unwrap());
                options.push(food_route);
                let food_tour = solver.car_route(
                    a, agents, net, supply, day, steps, &used, all_brands, 100, tag, 0,
                );
                options.push(food_tour);
                // Independent routes tend to visit the same attractive shops. Add
                // alternatives priced against the incumbent teammates' actual stock
                // reservations, then let the joint choice decide which to retain.
                let mut reserved = vec![0u8; solver.p.spots.len()];
                let mut covered = 0u64;
                for &b in &cars {
                    if b == a {
                        continue;
                    }
                    for (s, spot) in solver.p.spots.iter().enumerate() {
                        if c.masks[b] >> s & 1 != 0 {
                            reserved[s] += 1;
                            covered |= 1 << spot.brand;
                        }
                    }
                }
                solver.captured_routes = Some(Vec::new());
                let residual = solver.cell_route(
                    a, agents, net, supply, day, steps, &reserved, covered, 220, tag, 0,
                );
                options.extend(solver.captured_routes.take().unwrap());
                options.push(residual);
                options.push(solver.car_route(
                    a, agents, net, supply, day, steps, &reserved, covered, 120, tag, 0,
                ));
                if let Some((brand, _, target)) = focus {
                    if cars[target] == a {
                        solver.captured_routes = Some(Vec::new());
                        let transfer = solver.cell_route(
                            a,
                            agents,
                            net,
                            supply,
                            day,
                            steps,
                            &used,
                            all_brands & !(1 << brand),
                            160,
                            0,
                            0,
                        );
                        options.extend(solver.captured_routes.take().unwrap());
                        options.push(transfer);
                    }
                }
                let mut old_brands = 0u64;
                for (s, spot) in solver.p.spots.iter().enumerate() {
                    if c.masks[a] >> s & 1 != 0 {
                        old_brands |= 1 << spot.brand;
                    }
                }
                let old = CarRoute {
                    plan: c.plan[a].clone(),
                    mask: c.masks[a],
                    brands: old_brands,
                    pos: c.end[a].pos,
                    fuel: c.end[a].fuel as u16,
                };
                let mut unique = std::collections::BTreeMap::<(u32, usize), CarRoute>::new();
                for route in options {
                    let position = if day + 1 == solver.p.day_steps.len() {
                        0
                    } else {
                        route.pos
                    };
                    let key = (route.mask, position);
                    if route.mask == old.mask && route.pos == old.pos && route.fuel <= old.fuel {
                        continue;
                    }
                    if unique.get(&key).is_none_or(|prior| route.fuel > prior.fuel) {
                        unique.insert(key, route);
                    }
                }
                let mut options = unique.into_values().collect::<Vec<_>>();
                options.insert(0, old);
                let pool = options
                    .into_iter()
                    .map(|route| {
                        let endpoint = solver.terminal_pos(
                            route.pos,
                            steps,
                            route.fuel,
                            net,
                            supply,
                            steps,
                            day + 1 == solver.p.day_steps.len(),
                        );
                        OptionRoute { route, endpoint }
                    })
                    .collect::<Vec<_>>();
                pools.push(pool);
            }
            let mut choice = vec![0usize; cars.len()];
            let mut current = value(solver, &pools, &choice, day);
            let mut best = current;
            let mut best_choice = choice.clone();
            for iteration in 0..15000 {
                if iteration % 2000 == 1999 {
                    choice = best_choice.clone();
                    current = best;
                }
                let mut next = choice.clone();
                let a = solver.rng.n(cars.len());
                next[a] = solver.rng.n(pools[a].len());
                if solver.rng.n(4) == 0 {
                    let b = solver.rng.n(cars.len());
                    next[b] = solver.rng.n(pools[b].len());
                }
                let score = value(solver, &pools, &next, day);
                let temperature = 3.0 + 220.0 * (1.0 - (iteration % 2000) as f64 / 2000.0).powi(2);
                if score >= current || solver.rng.f() < ((score - current) / temperature).exp() {
                    choice = next;
                    current = score;
                    if score > best {
                        best = score;
                        best_choice = choice.clone();
                        let mut candidate = c.clone();
                        for (i, &a) in cars.iter().enumerate() {
                            let route = &pools[i][choice[i]].route;
                            candidate.plan[a] = route.plan.clone();
                            candidate.masks[a] = route.mask;
                            candidate.end[a].pos = route.pos;
                            candidate.end[a].fuel = route.fuel as u32;
                        }
                        candidate.score = solver.local_value(
                            &candidate.masks,
                            &candidate.end,
                            net,
                            supply,
                            day,
                            steps,
                        );
                        if candidate.score > c.score {
                            *c = candidate;
                        }
                    }
                }
            }
            // Enumerate all two-car replacements. With the other routes fixed, the
            // shared-stock correction is one intersection with the one-item mask.
            choice = best_choice;
            for _ in 0..3 {
                let mut changed = false;
                for a in 0..cars.len() {
                    for b in a + 1..cars.len() {
                        let mut stocks = [0u8; 32];
                        let mut covered = 0u64;
                        for i in 0..cars.len() {
                            if i == a || i == b {
                                continue;
                            }
                            let route = &pools[i][choice[i]].route;
                            covered |= route.brands;
                            let mut mask = route.mask;
                            while mask != 0 {
                                stocks[mask.trailing_zeros() as usize] += 1;
                                mask &= mask - 1;
                            }
                        }
                        let mut available = 0u32;
                        let mut one = 0u32;
                        for (i, spot) in solver.p.spots.iter().enumerate() {
                            let left = spot.stocks.saturating_sub(stocks[i] as u32);
                            if left > 0 {
                                available |= 1 << i;
                            }
                            if left == 1 {
                                one |= 1 << i;
                            }
                        }
                        let pair_score = |x: &OptionRoute, y: &OptionRoute| {
                            let brands = covered | x.route.brands | y.route.brands;
                            let food = (x.route.mask & available).count_ones()
                                + (y.route.mask & available).count_ones()
                                - (x.route.mask & y.route.mask & one).count_ones();
                            brands.count_ones() as f64 * 1000.0
                                + (brands & !solver.seen).count_ones() as f64
                                    * solver.fresh_bonus(day)
                                + food as f64 * 10.0
                                + x.endpoint
                                + y.endpoint
                        };
                        let mut pair = (choice[a], choice[b]);
                        let mut score = pair_score(&pools[a][pair.0], &pools[b][pair.1]);
                        for (x, route_a) in pools[a].iter().enumerate() {
                            for (y, route_b) in pools[b].iter().enumerate() {
                                let next = pair_score(route_a, route_b);
                                if next > score + 1e-8 {
                                    score = next;
                                    pair = (x, y);
                                }
                            }
                        }
                        if pair == (choice[a], choice[b]) {
                            continue;
                        }
                        choice[a] = pair.0;
                        choice[b] = pair.1;
                        changed = true;
                        let mut candidate = c.clone();
                        for (i, &agent) in cars.iter().enumerate() {
                            let route = &pools[i][choice[i]].route;
                            candidate.plan[agent] = route.plan.clone();
                            candidate.masks[agent] = route.mask;
                            candidate.end[agent].pos = route.pos;
                            candidate.end[agent].fuel = route.fuel as u32;
                        }
                        candidate.score = solver.local_value(
                            &candidate.masks,
                            &candidate.end,
                            net,
                            supply,
                            day,
                            steps,
                        );
                        if candidate.score > c.score {
                            *c = candidate;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            if (3..=4).contains(&cars.len()) {
                // Three-way exchanges can transfer a private brand without ever
                // sacrificing coverage in the accepted team plan.
                let started = std::time::Instant::now();
                let mut group = if let Some((_, owner, target)) = focus {
                    let others = (0..cars.len())
                        .filter(|&i| i != owner && i != target)
                        .collect::<Vec<_>>();
                    vec![owner, target, others[solver.rng.n(others.len())]]
                } else {
                    let mut indices = (0..cars.len()).collect::<Vec<_>>();
                    for i in 1..indices.len() {
                        let j = solver.rng.n(i + 1);
                        indices.swap(i, j);
                    }
                    indices.truncate(3);
                    indices
                };
                group.sort_unstable();
                let mut rest_stocks = [0u8; 32];
                let mut rest_brands = 0u64;
                let mut rest_value = 0.0;
                for i in 0..cars.len() {
                    if group.contains(&i) {
                        continue;
                    }
                    let option = &pools[i][choice[i]];
                    rest_value += option.endpoint;
                    rest_brands |= option.route.brands;
                    let mut mask = option.route.mask;
                    while mask != 0 {
                        rest_stocks[mask.trailing_zeros() as usize] += 1;
                        mask &= mask - 1;
                    }
                }
                let mut remaining = [0u32; 3];
                for (s, spot) in solver.p.spots.iter().enumerate() {
                    rest_value += (rest_stocks[s] as u32).min(spot.stocks) as f64 * 10.0;
                    let left = spot.stocks.saturating_sub(rest_stocks[s] as u32);
                    for layer in remaining.iter_mut().take(left as usize) {
                        *layer |= 1 << s;
                    }
                }
                let fresh = solver.fresh_bonus(day);
                let reward = |brands: u64| {
                    let brands = brands | rest_brands;
                    brands.count_ones() as f64 * 1000.0
                        + (brands & !solver.seen).count_ones() as f64 * fresh
                };
                let mut orders = Vec::new();
                for &i in &group {
                    let pool = &pools[i];
                    let mut order = (0..pool.len()).collect::<Vec<_>>();
                    order.sort_unstable_by(|&a, &b| {
                        let merit = |r: &OptionRoute| {
                            reward(r.route.brands)
                                + r.route.mask.count_ones() as f64 * 10.0
                                + r.endpoint
                        };
                        merit(&pool[b]).total_cmp(&merit(&pool[a])).then(a.cmp(&b))
                    });
                    orders.push(order);
                }
                let third = &pools[group[2]];
                let third_brands = third.iter().fold(0u64, |v, r| v | r.route.brands);
                let third_food = third
                    .iter()
                    .map(|r| r.route.mask.count_ones())
                    .max()
                    .unwrap();
                let third_endpoint = third
                    .iter()
                    .map(|r| r.endpoint)
                    .fold(f64::NEG_INFINITY, f64::max);
                let mut best = value(solver, &pools, &choice, day) - rest_value;
                for &x in &orders[0] {
                    if started.elapsed().as_millis() >= 100 {
                        break;
                    }
                    let a = &pools[group[0]][x];
                    for &y in &orders[1] {
                        let b = &pools[group[1]][y];
                        let covered = a.route.brands | b.route.brands;
                        let union = a.route.mask | b.route.mask;
                        let both = a.route.mask & b.route.mask;
                        let food = (union & remaining[0]).count_ones()
                            + (both & remaining[1]).count_ones();
                        let endpoint = a.endpoint + b.endpoint;
                        let upper = reward(covered | third_brands)
                            + (food + third_food) as f64 * 10.0
                            + endpoint
                            + third_endpoint;
                        if upper <= best + 1e-8 {
                            continue;
                        }
                        let available = (!union & remaining[0])
                            | ((union ^ both) & remaining[1])
                            | (both & remaining[2]);
                        for &z in &orders[2] {
                            let d = &third[z];
                            let score = reward(covered | d.route.brands)
                                + (food + (d.route.mask & available).count_ones()) as f64 * 10.0
                                + endpoint
                                + d.endpoint;
                            if score <= best + 1e-8 {
                                continue;
                            }
                            best = score;
                            let mut selected = choice.clone();
                            for (j, index) in [x, y, z].into_iter().enumerate() {
                                selected[group[j]] = index;
                            }
                            let mut candidate = c.clone();
                            for (i, &agent) in cars.iter().enumerate() {
                                let route = &pools[i][selected[i]].route;
                                candidate.plan[agent] = route.plan.clone();
                                candidate.masks[agent] = route.mask;
                                candidate.end[agent].pos = route.pos;
                                candidate.end[agent].fuel = route.fuel as u32;
                            }
                            candidate.score = solver.local_value(
                                &candidate.masks,
                                &candidate.end,
                                net,
                                supply,
                                day,
                                steps,
                            );
                            if candidate.score > c.score {
                                *c = candidate;
                            }
                        }
                    }
                }
            }
            // Polish endpoints with the full team objective, including future access
            // to brands; this term cannot be separated into individual route values.
            for _ in 0..2 {
                let before = c.score;
                for (i, &a) in cars.iter().enumerate() {
                    for option in &pools[i] {
                        let route = &option.route;
                        let mut masks = c.masks.clone();
                        let mut end = c.end.clone();
                        masks[a] = route.mask;
                        end[a].pos = route.pos;
                        end[a].fuel = route.fuel as u32;
                        let score = solver.local_value(&masks, &end, net, supply, day, steps);
                        if score > c.score + 1e-8 {
                            c.score = score;
                            c.masks = masks;
                            c.end = end;
                            c.plan[a] = route.plan.clone();
                        }
                    }
                }
                if c.score <= before + 1e-8 {
                    break;
                }
            }
            if c.score > original {
                solver.tour_stats[8] += 1;
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn combining_routes_respects_shared_stock_and_exact_fuel() {
                let p = tester::model::Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![12],
                    map: tester::model::Map {
                        width: 4,
                        height: 3,
                        cells: vec![vec![0; 4]; 3],
                    },
                    spots: [(0, 1, 1), (1, 2, 2), (0, 5, 1), (2, 6, 1)]
                        .into_iter()
                        .map(|(brand, pos, stocks)| tester::model::Spot { brand, pos, stocks })
                        .collect(),
                    agents: vec![0, 3, 4, 7, 11],
                    fuel_limits: 4,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let kinds = [0, 0, 0, 0, 1];
                let mut game = tester::engine::Game::new(p.clone(), &kinds);
                let agents = game.agents.clone();
                let net = Network::new(&p, &agents, &[]);
                let support = vec![super::super::SupportRoute {
                    stops: vec![],
                    direct: Some(vec![-12]),
                }];
                let supply = Supply::build(&p, &agents, &net, &support, 12);
                let mut solver = Solver::new(p, 26);
                let mut candidate = Candidate {
                    plan: game.waiting_plan(),
                    score: 0.,
                    masks: vec![0; 5],
                    end: agents.clone(),
                    support,
                };
                // One patrol initially owns brand 0, so the focused-transfer path is
                // exercised while another patrol remains outside the three-car group.
                candidate.plan[0] = vec![2, -10];
                candidate.masks[0] = 1;
                candidate.end[0].pos = 1;
                candidate.end[0].fuel = 3;
                candidate.score =
                    solver.local_value(&candidate.masks, &candidate.end, &net, &supply, 0, 12);
                let before = candidate.score;
                improve(&mut solver, &agents, &net, &supply, &mut candidate, 0, 12);
                assert!(candidate.score > before);
                assert!(candidate.score > 0.);
                let (brands, food) = solver.statistics(&candidate);
                game.play_day(candidate.plan).unwrap();
                assert_eq!(game.score.daily_brands, brands.count_ones() as usize);
                assert_eq!(game.score.udon, food);
                assert_eq!(game.agents, candidate.end);
                assert!(solver.captured_routes.is_none());
            }
        }
    }
    // 保存元: solver04/tomorrow.rs
    mod tomorrow {
        //! Compare endpoints only when the current day cannot cover every brand.
        use super::{Candidate, Network, Solver, Supply, escort, route_set, tour};
        use std::time::{Duration, Instant};
        use tester::model::Traffic;

        pub(super) fn retain(solver: &Solver, pool: &mut Vec<Candidate>, candidate: &Candidate) {
            let brands = solver.statistics(candidate).0.count_ones();
            let maximum = pool
                .iter()
                .map(|c| solver.statistics(c).0.count_ones())
                .max()
                .unwrap_or(0);
            if brands < maximum {
                return;
            }
            if brands > maximum {
                pool.clear();
            }
            if let Some(i) = pool.iter().position(|c| {
                c.end
                    .iter()
                    .zip(&candidate.end)
                    .all(|(a, b)| a.pos == b.pos && a.fuel / 8 == b.fuel / 8)
            }) {
                if candidate.score > pool[i].score {
                    pool[i] = candidate.clone();
                }
                return;
            }
            pool.push(candidate.clone());
            pool.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
            // Preserve distinct endpoints, while bounding next-day comparisons.
            if pool.len() > 6 {
                let mut selected = vec![0];
                while selected.len() < 6 {
                    let index = (1..pool.len())
                        .filter(|i| !selected.contains(i))
                        .max_by(|&a, &b| {
                            let diversity = |i: usize| {
                                let distance = selected
                                    .iter()
                                    .map(|&j| {
                                        pool[i]
                                            .end
                                            .iter()
                                            .zip(&pool[j].end)
                                            .map(|(a, b)| {
                                                let w = solver.p.map.width;
                                                (a.pos % w).abs_diff(b.pos % w)
                                                    + (a.pos / w).abs_diff(b.pos / w)
                                                    + a.fuel.abs_diff(b.fuel) as usize / 4
                                            })
                                            .sum::<usize>()
                                    })
                                    .min()
                                    .unwrap_or(0);
                                pool[i].score + distance as f64 * 2.0
                            };
                            diversity(a).total_cmp(&diversity(b))
                        })
                        .unwrap();
                    selected.push(index);
                }
                *pool = selected.into_iter().map(|i| pool[i].clone()).collect();
            }
        }

        pub(super) fn choose(
            solver: &mut Solver,
            mut pool: Vec<Candidate>,
            baseline: Candidate,
            day: usize,
            observed: &[Traffic],
            remaining: Duration,
        ) -> Candidate {
            retain(solver, &mut pool, &baseline);
            pool.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
            let started = Instant::now();
            let next_day = day + 1;
            let traffic = if day > 0 {
                observed.to_vec()
            } else {
                (0..solver.p.map.len())
                    .filter(|&pos| solver.p.map.terrain(pos) == 1)
                    .map(|pos| Traffic { pos, status: 1 })
                    .collect::<Vec<_>>()
            };
            let mut best = baseline.clone();
            let mut best_value = f64::NEG_INFINITY;
            let mut future_result = (0u32, 0u32);
            for (index, current) in pool.iter().enumerate() {
                let left = remaining.saturating_sub(started.elapsed());
                if left < Duration::from_millis(5) {
                    break;
                }
                let share = left / (pool.len() - index) as u32;
                let mut probe = Solver::new(solver.p.clone(), solver.rng.next());
                probe.seen = solver.seen | solver.statistics(current).0;
                let net = Network::new(&probe.p, &current.end, &traffic);
                let steps = probe.p.day_steps[next_day] as u16;
                let support = probe.initial_support(&current.end, &net, steps);
                let supply = Supply::build(&probe.p, &current.end, &net, &support, steps);
                let mut future = probe.build_candidate(
                    &current.end,
                    &net,
                    &supply,
                    support,
                    next_day,
                    steps,
                    0,
                    65,
                );
                let local_start = Instant::now();
                let mut iteration = 0;
                while local_start.elapsed() < share.saturating_sub(Duration::from_millis(5))
                    && started.elapsed() < remaining
                {
                    let mut c = future.clone();
                    if iteration % 5 == 0 {
                        let support = if iteration % 10 == 0 {
                            probe.hub_support(&current.end, &net, steps, iteration / 5)
                        } else {
                            escort::optimize(
                                &probe.p,
                                &net,
                                &current.end,
                                &c,
                                steps,
                                next_day + 1 == probe.p.day_steps.len(),
                                100,
                                &mut probe.rng,
                            )
                        };
                        let supply = Supply::build(&probe.p, &current.end, &net, &support, steps);
                        c = probe
                            .preserve_patrols(
                                &c,
                                &current.end,
                                &net,
                                &supply,
                                support,
                                next_day,
                                steps,
                            )
                            .unwrap();
                        probe.rebuild_subset(
                            &mut c,
                            &current.end,
                            &net,
                            &supply,
                            next_day,
                            steps,
                            iteration,
                            45,
                        );
                    } else {
                        let supply = Supply::build(&probe.p, &current.end, &net, &c.support, steps);
                        if iteration % 11 == 1 {
                            route_set::improve(
                                &mut probe,
                                &current.end,
                                &net,
                                &supply,
                                &mut c,
                                next_day,
                                steps,
                            );
                        } else if iteration % 7 == 1 {
                            tour::improve(
                                &mut probe,
                                &current.end,
                                &net,
                                &supply,
                                &mut c,
                                next_day,
                                steps,
                                1800,
                            );
                        } else {
                            probe.rebuild_subset(
                                &mut c,
                                &current.end,
                                &net,
                                &supply,
                                next_day,
                                steps,
                                iteration,
                                55,
                            );
                        }
                    }
                    if c.score > future.score {
                        future = c;
                    }
                    iteration += 1;
                }
                let projected = current.score + future.score * 0.65;
                if projected > best_value {
                    best_value = projected;
                    best = current.clone();
                    let (brands, food) = probe.statistics(&future);
                    future_result = (brands.count_ones(), food);
                }
            }
            eprintln!(
                "tomorrow candidates={} today_delta={:.1} projected={:?} elapsed_ms={}",
                pool.len(),
                best.score - baseline.score,
                future_result,
                started.elapsed().as_millis()
            );
            best
        }
    }
    // 保存元: solver04/tour.rs
    mod tour {
        //! Local search on the order of patrol visits, retaining timed refill checkpoints.
        use super::{Candidate, CarRoute, Network, Rng, Solver, Supply};
        use tester::model::{Agent, Problem, Traffic};

        #[derive(Clone)]
        pub(super) struct Visit {
            pub(super) node: usize,
            pub(super) earliest: u16,
            pub(super) mode: usize,
        }
        fn node_for(pos: usize, nodes: &mut Vec<usize>) -> usize {
            if let Some(n) = nodes.iter().position(|&v| v == pos) {
                n
            } else {
                nodes.push(pos);
                nodes.len() - 1
            }
        }
        pub(super) fn extract(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agents: &[Agent],
            c: &Candidate,
            steps: u16,
        ) -> (Vec<usize>, Vec<Vec<Visit>>) {
            let mut nodes = net.pos.clone();
            let mut tours = vec![Vec::new(); agents.len()];
            for (a, ag) in agents.iter().enumerate() {
                if ag.kind != 0 {
                    continue;
                }
                let mut pos = ag.pos;
                let mut fuel = ag.fuel as u16;
                let mut t = 0u16;
                let mut seen = 0u32;
                for &action in &c.plan[a] {
                    let dt = if action < 0 {
                        (-action) as u16
                    } else {
                        net.time[pos] as u16
                    };
                    let to = if action < 0 {
                        pos
                    } else {
                        p.map.neighbor(pos, action as usize).unwrap()
                    };
                    let cost = if action < 0 { 0 } else { net.cost[pos] as u16 };
                    for k in 1..=dt {
                        t += 1;
                        if t > steps {
                            break;
                        }
                        if k == dt {
                            fuel = fuel.saturating_sub(cost);
                            pos = to;
                        }
                        let refill = supply.at[t as usize][pos] && fuel < p.fuel_limits as u16;
                        let s = net.spot_at[pos];
                        let pickup = s >= 0 && seen >> (s as usize) & 1 == 0;
                        if s >= 0 {
                            seen |= 1u32 << (s as usize);
                        }
                        if refill || pickup {
                            let mut window_start = t;
                            if refill {
                                while window_start > 0 && supply.at[window_start as usize - 1][pos]
                                {
                                    window_start -= 1;
                                }
                            }
                            let anchor = if action >= 0 && k < dt {
                                window_start.saturating_sub(dt - 1)
                            } else {
                                window_start
                            };
                            let node = node_for(pos, &mut nodes);
                            if let Some(last) = tours[a].last_mut() {
                                let last: &mut Visit = last;
                                if last.node == node {
                                    if refill {
                                        last.earliest = anchor;
                                    }
                                } else {
                                    tours[a].push(Visit {
                                        node,
                                        earliest: if refill { anchor } else { 0 },
                                        mode: 0,
                                    });
                                }
                            } else {
                                tours[a].push(Visit {
                                    node,
                                    earliest: if refill { anchor } else { 0 },
                                    mode: 0,
                                });
                            }
                        }
                        if refill {
                            fuel = p.fuel_limits as u16;
                        }
                    }
                    if t >= steps {
                        break;
                    }
                }
                let node = node_for(pos, &mut nodes);
                if tours[a].last().is_none_or(|v| v.node != node) {
                    tours[a].push(Visit {
                        node,
                        earliest: 0,
                        mode: 0,
                    });
                }
            }
            (nodes, tours)
        }
        fn insertion_position(
            tour: &[Visit],
            node: usize,
            start: usize,
            net: &Network,
            rng: &mut Rng,
        ) -> usize {
            let mut choices = (0..=tour.len())
                .map(|j| {
                    let prev = if j == 0 { start } else { tour[j - 1].node };
                    let added = net.path(prev, node, 0).time as i32
                        + if j < tour.len() {
                            net.path(node, tour[j].node, 0).time as i32
                                - net.path(prev, tour[j].node, 0).time as i32
                        } else {
                            0
                        };
                    (added, j)
                })
                .collect::<Vec<_>>();
            choices.sort_unstable();
            let k = if rng.n(3) == 0 {
                rng.n(choices.len().min(4))
            } else {
                0
            };
            choices[k].1
        }
        struct Evaluation {
            route: CarRoute,
            time: u16,
        }
        fn evaluate_detailed(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            ag: &Agent,
            a: usize,
            tour: &[Visit],
            steps: u16,
        ) -> Result<Evaluation, (usize, bool)> {
            let mut node = p.spots.len() + a;
            let mut pos = ag.pos;
            let mut fuel = ag.fuel as u16;
            let mut t = 0u16;
            let mut mask = 0u32;
            let mut plan = Vec::new();
            let collect = |at: usize, mask: &mut u32| {
                let s = net.spot_at[at];
                if s >= 0 {
                    *mask |= 1u32 << (s as usize);
                }
            };
            let limit = p.fuel_limits as u16;
            for (index, visit) in tour.iter().enumerate() {
                let path = net.path(node, visit.node, visit.mode);
                for &(to, dir, dt, cost) in &path.moves {
                    let dt = dt as u16;
                    let cost = cost as u16;
                    if t + dt > steps {
                        return Err((index, false));
                    }
                    let available = supply.wait_fuel(pos, t, t + dt - 1, fuel, limit);
                    if available < cost {
                        let meet = supply.next[t as usize][pos];
                        if meet == u16::MAX || meet + dt > steps {
                            return Err((index, true));
                        }
                        plan.push(-((meet - t) as i64));
                        t = meet;
                        fuel = limit;
                        collect(pos, &mut mask);
                    }
                    if dt > 1 {
                        fuel = supply.wait_fuel(pos, t, t + dt - 1, fuel, limit);
                        collect(pos, &mut mask);
                    }
                    if fuel < cost {
                        return Err((index, true));
                    }
                    fuel -= cost;
                    t += dt;
                    pos = to as usize;
                    if supply.at[t as usize][pos] {
                        fuel = limit;
                    }
                    collect(pos, &mut mask);
                    plan.push(dir as i64);
                }
                if t < visit.earliest {
                    if visit.earliest > steps {
                        return Err((index, false));
                    }
                    fuel = supply.wait_fuel(pos, t, visit.earliest, fuel, limit);
                    collect(pos, &mut mask);
                    plan.push(-((visit.earliest - t) as i64));
                    t = visit.earliest;
                }
                node = visit.node;
            }
            let time = t;
            if t < steps {
                plan.push(-((steps - t) as i64));
                fuel = supply.wait_fuel(pos, t, steps, fuel, limit);
                collect(pos, &mut mask);
            }
            let mut brands = 0;
            for (s, spot) in p.spots.iter().enumerate() {
                if mask >> s & 1 != 0 {
                    brands |= 1 << spot.brand;
                }
            }
            Ok(Evaluation {
                route: CarRoute {
                    plan,
                    mask,
                    brands,
                    pos,
                    fuel,
                },
                time,
            })
        }
        fn evaluate(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            ag: &Agent,
            a: usize,
            tour: &[Visit],
            steps: u16,
        ) -> Option<Evaluation> {
            evaluate_detailed(p, net, supply, ag, a, tour, steps).ok()
        }
        fn evaluate_assisted(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            ag: &Agent,
            a: usize,
            tour: &mut Vec<Visit>,
            steps: u16,
            rng: &mut Rng,
            stats: &mut [usize],
        ) -> Option<Evaluation> {
            let failure = match evaluate_detailed(p, net, supply, ag, a, tour, steps) {
                Ok(e) => return Some(e),
                Err(e) => e,
            };
            if !failure.1 || rng.n(4) != 0 {
                return None;
            }
            stats[0] += 1;
            let mut options = Vec::new();
            for i in failure.0.saturating_sub(2)..=failure.0 {
                let prev = if i == 0 {
                    p.spots.len() + a
                } else {
                    tour[i - 1].node
                };
                let next = tour[i].node;
                let Some(prefix) = evaluate(p, net, supply, ag, a, &tour[..i], steps) else {
                    continue;
                };
                for &(node, arrive, depart) in &supply.windows {
                    if node == prev || node == next || depart == 0 {
                        continue;
                    }
                    let arrival = prefix.time + net.path(prev, node, 2).time;
                    if arrival > depart {
                        continue;
                    }
                    let extra = net.path(prev, node, 2).time as i32
                        + net.path(node, next, 2).time as i32
                        - net.path(prev, next, 2).time as i32
                        + arrive.saturating_sub(arrival) as i32;
                    options.push((extra, i, node, arrive));
                }
            }
            options.sort_unstable();
            options.dedup();
            for &(_, at, node, earliest) in options.iter().take(3) {
                let mut revised = tour.clone();
                revised.insert(
                    at,
                    Visit {
                        node,
                        earliest,
                        mode: 2,
                    },
                );
                if let Some(e) = evaluate(p, net, supply, ag, a, &revised, steps) {
                    stats[1] += 1;
                    *tour = revised;
                    return Some(e);
                }
            }
            None
        }
        pub(super) fn improve(
            solver: &mut Solver,
            agents: &[Agent],
            original_net: &Network,
            supply: &Supply,
            c: &mut Candidate,
            day: usize,
            steps: u16,
            iterations: usize,
        ) {
            solver.tour_stats[0] += 1;
            let old_score = c.score;
            let (nodes, mut tours) = extract(&solver.p, original_net, supply, agents, c, steps);
            let traffic = (0..solver.p.map.len())
                .filter(|&v| solver.p.map.terrain(v) == 1)
                .map(|pos| Traffic {
                    pos,
                    status: match original_net.time[pos] {
                        1 => 0,
                        2 => 1,
                        _ => 2,
                    },
                })
                .collect::<Vec<_>>();
            let net = Network::from_positions(&solver.p, nodes, &traffic);
            // Roadside refill checkpoints extracted from patrol paths are also available
            // as insertion targets when another patrol needs a new fuel stop.
            let timed_supply = Supply::build(&solver.p, agents, &net, &c.support, steps);
            let supply = &timed_supply;
            let cars = agents
                .iter()
                .enumerate()
                .filter(|(_, ag)| ag.kind == 0)
                .map(|(a, _)| a)
                .collect::<Vec<_>>();
            let mut current = c.clone();
            let mut times = vec![steps; agents.len()];
            let mut active = Vec::new();
            for &a in &cars {
                let mut trial = evaluate(&solver.p, &net, supply, &agents[a], a, &tours[a], steps);
                if trial.is_none() {
                    for v in &mut tours[a] {
                        v.mode = 2;
                    }
                    trial = evaluate(&solver.p, &net, supply, &agents[a], a, &tours[a], steps);
                }
                let Some(e) = trial else {
                    continue;
                };
                active.push(a);
                times[a] = e.time;
                current.plan[a] = e.route.plan;
                current.masks[a] = e.route.mask;
                current.end[a].pos = e.route.pos;
                current.end[a].fuel = e.route.fuel as u32;
            }
            if active.is_empty() {
                return;
            }
            let cars = active;
            solver.tour_stats[1] += 1;
            current.score =
                solver.local_value(&current.masks, &current.end, &net, supply, day, steps);
            if current.score > c.score {
                *c = current.clone();
            }
            let mut best_tours = tours.clone();
            let mut best_local = current.clone();
            let mut best_times = times.clone();
            let metric = |c: &Candidate, tt: &[u16]| {
                c.score + tt.iter().map(|&t| (steps - t) as f64).sum::<f64>() * 0.15
            };
            let mut best_metric = metric(&current, &times);
            for it in 0..iterations {
                if it % 700 == 699 {
                    current = best_local.clone();
                    tours = best_tours.clone();
                    times = best_times.clone();
                }
                let a = cars[solver.rng.n(cars.len())];
                let mut proposed = tours[a].clone();
                let op = solver.rng.n(14);
                let mut other = None;
                match op {
                    0 | 1 | 2 => {
                        if proposed.len() > 40 {
                            continue;
                        }
                        let node = solver.rng.n(solver.p.spots.len());
                        if proposed.iter().any(|v| v.node == node) && solver.rng.n(5) != 0 {
                            continue;
                        }
                        if current
                            .masks
                            .iter()
                            .filter(|&&mask| mask >> node & 1 != 0)
                            .count() as u32
                            >= solver.p.spots[node].stocks
                            && !supply.nodes.contains(&node)
                        {
                            continue;
                        }
                        let k = if op == 0 {
                            solver.rng.n(proposed.len() + 1)
                        } else {
                            insertion_position(
                                &proposed,
                                node,
                                solver.p.spots.len() + a,
                                &net,
                                &mut solver.rng,
                            )
                        };
                        proposed.insert(
                            k,
                            Visit {
                                node,
                                earliest: 0,
                                mode: solver.rng.n(3),
                            },
                        );
                    }
                    3 => {
                        if proposed.is_empty() {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        proposed.remove(k);
                    }
                    4 => {
                        if proposed.len() < 2 {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        let v = proposed.remove(k);
                        let j = if solver.rng.n(3) == 0 {
                            solver.rng.n(proposed.len() + 1)
                        } else {
                            insertion_position(
                                &proposed,
                                v.node,
                                solver.p.spots.len() + a,
                                &net,
                                &mut solver.rng,
                            )
                        };
                        proposed.insert(j, v);
                    }
                    5 => {
                        if proposed.len() < 2 {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        let j = solver.rng.n(proposed.len());
                        proposed.swap(k, j);
                    }
                    6 => {
                        if proposed.len() < 2 {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        let j = solver.rng.n(proposed.len());
                        proposed[k.min(j)..=k.max(j)].reverse();
                    }
                    7 => {
                        if proposed.is_empty() {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        proposed[k].mode = solver.rng.n(3);
                    }
                    8 => {
                        if proposed.is_empty() {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        if proposed[k].earliest > 0 {
                            proposed[k].earliest = proposed[k]
                                .earliest
                                .saturating_add_signed(solver.rng.n(9) as i16 - 4)
                                .min(steps);
                        } else {
                            proposed[k].node = solver.rng.n(solver.p.spots.len());
                        }
                    }
                    12 | 13 => {
                        if proposed.is_empty() {
                            continue;
                        }
                        let k = solver.rng.n(proposed.len());
                        let old = proposed[k].node;
                        if old >= solver.p.spots.len() || proposed[k].earliest > 0 {
                            continue;
                        }
                        let brand = solver.p.spots[old].brand;
                        let options = solver
                            .p
                            .spots
                            .iter()
                            .enumerate()
                            .filter(|&(s, spot)| {
                                s != old
                                    && spot.brand == brand
                                    && !proposed.iter().any(|v| v.node == s)
                            })
                            .map(|(s, _)| s)
                            .collect::<Vec<_>>();
                        if options.is_empty() {
                            continue;
                        }
                        let node = options[solver.rng.n(options.len())];
                        if op == 12 {
                            proposed[k].node = node;
                            proposed[k].mode = solver.rng.n(3);
                        } else {
                            proposed.remove(k);
                            let j = insertion_position(
                                &proposed,
                                node,
                                solver.p.spots.len() + a,
                                &net,
                                &mut solver.rng,
                            );
                            proposed.insert(
                                j,
                                Visit {
                                    node,
                                    earliest: 0,
                                    mode: solver.rng.n(3),
                                },
                            );
                        }
                    }
                    _ => {
                        let b = cars[solver.rng.n(cars.len())];
                        if a == b || proposed.is_empty() {
                            continue;
                        }
                        let mut second = tours[b].clone();
                        let k = solver.rng.n(proposed.len());
                        let j = if op == 9 && solver.rng.n(3) != 0 {
                            insertion_position(
                                &second,
                                proposed[k].node,
                                solver.p.spots.len() + b,
                                &net,
                                &mut solver.rng,
                            )
                        } else {
                            solver.rng.n(second.len() + 1)
                        };
                        if op == 9 {
                            let mut v = proposed.remove(k);
                            v.earliest = 0;
                            second.insert(j, v);
                        } else if op == 10 {
                            if j == second.len() {
                                continue;
                            }
                            std::mem::swap(&mut proposed[k], &mut second[j]);
                            proposed[k].earliest = 0;
                            second[j].earliest = 0;
                        } else {
                            let left = proposed.split_off(k);
                            let right = second.split_off(j);
                            proposed.extend(right);
                            second.extend(left);
                        }
                        other = Some((b, second));
                    }
                }
                let Some(e) = evaluate_assisted(
                    &solver.p,
                    &net,
                    supply,
                    &agents[a],
                    a,
                    &mut proposed,
                    steps,
                    &mut solver.rng,
                    &mut solver.tour_stats[3..],
                ) else {
                    continue;
                };
                let mut candidate = current.clone();
                candidate.plan[a] = e.route.plan;
                candidate.masks[a] = e.route.mask;
                candidate.end[a].pos = e.route.pos;
                candidate.end[a].fuel = e.route.fuel as u32;
                let mut time_delta = times[a] as f64 - e.time as f64;
                let mut other_time = 0;
                if let Some((b, second)) = &mut other {
                    let Some(e) = evaluate_assisted(
                        &solver.p,
                        &net,
                        supply,
                        &agents[*b],
                        *b,
                        second,
                        steps,
                        &mut solver.rng,
                        &mut solver.tour_stats[3..],
                    ) else {
                        continue;
                    };
                    candidate.plan[*b] = e.route.plan;
                    candidate.masks[*b] = e.route.mask;
                    candidate.end[*b].pos = e.route.pos;
                    candidate.end[*b].fuel = e.route.fuel as u32;
                    time_delta += times[*b] as f64 - e.time as f64;
                    other_time = e.time;
                }
                candidate.score =
                    solver.local_value(&candidate.masks, &candidate.end, &net, supply, day, steps);
                let delta = candidate.score - current.score + time_delta * 0.15;
                let temperature = 1.0 + 30.0 * (1.0 - (it % 700) as f64 / 700.0).powi(2);
                if delta >= 0.0 || solver.rng.f() < (delta / temperature).exp() {
                    current = candidate;
                    tours[a] = proposed;
                    times[a] = e.time;
                    if let Some((b, second)) = other {
                        tours[b] = second;
                        times[b] = other_time;
                    }
                    let new_metric = metric(&current, &times);
                    if new_metric > best_metric {
                        best_metric = new_metric;
                        best_local = current.clone();
                        best_tours = tours.clone();
                        best_times = times.clone();
                    }
                    if current.score > c.score {
                        *c = current.clone();
                    }
                }
            }
            if c.score > old_score {
                solver.tour_stats[2] += 1;
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn failed_tour_can_insert_a_refill_before_the_fuel_shortage() {
                let p = Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![10],
                    map: tester::model::Map {
                        width: 3,
                        height: 2,
                        cells: vec![vec![0, 0, 0], vec![0, 0, 0]],
                    },
                    spots: vec![tester::model::Spot {
                        brand: 0,
                        pos: 2,
                        stocks: 1,
                    }],
                    agents: vec![0, 3],
                    fuel_limits: 4,
                    players: 1,
                    busy_threshold: 2,
                    jammed_threshold: 4,
                };
                let agents = vec![
                    Agent {
                        kind: 0,
                        pos: 0,
                        fuel: 1,
                    },
                    Agent {
                        kind: 1,
                        pos: 3,
                        fuel: 4,
                    },
                ];
                let net = Network::new(&p, &agents, &[]);
                let support = vec![super::super::SupportRoute {
                    direct: Some(vec![-10]),
                    stops: Vec::new(),
                }];
                let supply = Supply::build(&p, &agents, &net, &support, 10);
                let mut rng = Rng(13);
                let mut found = None;
                let mut stats = [0; 2];
                for _ in 0..40 {
                    let mut tour = vec![Visit {
                        node: 0,
                        earliest: 0,
                        mode: 0,
                    }];
                    if let Some(e) = evaluate_assisted(
                        &p, &net, &supply, &agents[0], 0, &mut tour, 10, &mut rng, &mut stats,
                    ) {
                        found = Some(e);
                        break;
                    }
                }
                let e = found.unwrap();
                let mut game = tester::engine::Game::new(p, &[0, 1]);
                game.agents = agents;
                game.play_day(vec![e.route.plan, vec![-10]]).unwrap();
                assert_eq!(game.score.udon, 1);
                assert_eq!(game.agents[0].fuel, e.route.fuel as u32);
            }
        }
    }
    // 保存元: solver04/window.rs
    mod window {
        //! Exact-time replacement of a portion of one patrol route.
        use super::{Candidate, CarRoute, Network, Rng, SearchMap, Supply};
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use tester::model::{Agent, Problem};

        struct Trace {
            actions: Vec<i64>,
            bounds: Vec<(u16, usize, u16)>,
            pos: Vec<usize>,
            mask: Vec<u32>,
            cost: Vec<u16>,
        }
        fn trace(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agent: &Agent,
            plan: &[i64],
            steps: u16,
        ) -> Option<Trace> {
            let mut actions = Vec::new();
            for &act in plan {
                if act < 0 {
                    actions.extend(std::iter::repeat_n(-1, (-act) as usize));
                } else {
                    actions.push(act);
                }
            }
            let mut bounds = vec![(0, agent.pos, agent.fuel as u16)];
            let mut positions = vec![agent.pos; steps as usize + 1];
            let mut masks = vec![0u32; steps as usize + 1];
            let mut costs = vec![0u16; steps as usize + 1];
            let mut pos = agent.pos;
            let mut t = 0u16;
            let mut fuel = agent.fuel as u16;
            let mut mask = 0u32;
            for &act in &actions {
                let dt = if act < 0 { 1 } else { net.time[pos] as u16 };
                let end = t + dt;
                if end > steps {
                    return None;
                }
                let to = if act < 0 {
                    pos
                } else {
                    p.map.neighbor(pos, act as usize)?
                };
                let cost = if act < 0 { 0 } else { net.cost[pos] as u16 };
                for k in t + 1..=end {
                    if k == end {
                        if fuel < cost {
                            return None;
                        }
                        fuel -= cost;
                        pos = to;
                        costs[k as usize] = cost;
                    }
                    if supply.at[k as usize][pos] {
                        fuel = p.fuel_limits as u16;
                    }
                    let s = net.spot_at[pos];
                    if s >= 0 {
                        mask |= 1 << s;
                    }
                    positions[k as usize] = pos;
                    masks[k as usize] = mask;
                }
                t = end;
                bounds.push((t, pos, fuel));
            }
            if t != steps {
                return None;
            }
            Some(Trace {
                actions,
                bounds,
                pos: positions,
                mask: masks,
                cost: costs,
            })
        }
        /// Retain the sequence of moves while jointly retiming waits against the new supplier path.
        /// At a fixed (move index, time), larger fuel dominates every smaller-fuel state.
        pub(super) fn retime_route(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agent: &Agent,
            old: &[i64],
            steps: u16,
        ) -> Option<CarRoute> {
            let moves = old.iter().copied().filter(|&x| x >= 0).collect::<Vec<_>>();
            let mut positions = vec![agent.pos];
            let mut durations = Vec::new();
            let mut costs = Vec::new();
            for &action in &moves {
                let pos = *positions.last().unwrap();
                durations.push(net.time[pos] as u16);
                costs.push(net.cost[pos] as u16);
                positions.push(p.map.neighbor(pos, action as usize)?);
            }
            let stride = steps as usize + 1;
            let mut fuel = vec![u16::MAX; (moves.len() + 1) * stride];
            let mut previous_move = vec![false; fuel.len()];
            fuel[0] = agent.fuel as u16;
            let limit = p.fuel_limits as u16;
            let mut best = None;
            for t in 0..=steps {
                for i in 0..=moves.len() {
                    let f = fuel[i * stride + t as usize];
                    if f == u16::MAX {
                        continue;
                    }
                    let pos = positions[i];
                    if i == moves.len() {
                        let ending = supply.wait_fuel(pos, t, steps, f, limit);
                        if best.is_none_or(|(ff, _)| ending > ff) {
                            best = Some((ending, t));
                        }
                        continue;
                    }
                    if t < steps {
                        let ff = if supply.at[t as usize + 1][pos] {
                            limit
                        } else {
                            f
                        };
                        let index = i * stride + t as usize + 1;
                        if fuel[index] == u16::MAX || ff > fuel[index] {
                            fuel[index] = ff;
                            previous_move[index] = false;
                        }
                    }
                    let end = t + durations[i];
                    if end > steps {
                        continue;
                    }
                    let available = supply.wait_fuel(pos, t, end - 1, f, limit);
                    if available < costs[i] {
                        continue;
                    }
                    let ff = if supply.at[end as usize][positions[i + 1]] {
                        limit
                    } else {
                        available - costs[i]
                    };
                    let index = (i + 1) * stride + end as usize;
                    if fuel[index] == u16::MAX || ff > fuel[index] {
                        fuel[index] = ff;
                        previous_move[index] = true;
                    }
                }
            }
            let (_, finish) = best?;
            let mut t = finish;
            let mut i = moves.len();
            let mut actions = Vec::new();
            while t > 0 {
                if previous_move[i * stride + t as usize] {
                    i -= 1;
                    t -= durations[i];
                    actions.push(moves[i]);
                } else {
                    t -= 1;
                    actions.push(-1);
                }
            }
            actions.reverse();
            if finish < steps {
                actions.push(-((steps - finish) as i64));
            }
            let mut plan = Vec::new();
            for action in actions {
                if action < 0 && plan.last().is_some_and(|&a| a < 0) {
                    *plan.last_mut().unwrap() += action;
                } else {
                    plan.push(action);
                }
            }
            replay_route(p, net, supply, agent, &plan, steps)
        }
        #[cfg(test)]
        pub(super) fn repair_route(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agent: &Agent,
            old: &[i64],
            steps: u16,
        ) -> CarRoute {
            let mut pos = agent.pos;
            let mut fuel = agent.fuel as u16;
            let mut t = 0u16;
            let limit = p.fuel_limits as u16;
            let mut plan = Vec::new();
            for &action in old {
                if t >= steps {
                    break;
                }
                let dt = if action < 0 {
                    ((-action) as u16).min(steps - t)
                } else {
                    net.time[pos] as u16
                };
                if t + dt > steps {
                    break;
                }
                let cost = if action < 0 { 0 } else { net.cost[pos] as u16 };
                let available = supply.wait_fuel(pos, t, t + dt - 1, fuel, limit);
                if available < cost {
                    let meet = supply.next[t as usize][pos];
                    if meet == u16::MAX || meet + dt > steps {
                        break;
                    }
                    plan.push(-((meet - t) as i64));
                    t = meet;
                    fuel = limit;
                }
                if dt > 1 {
                    fuel = supply.wait_fuel(pos, t, t + dt - 1, fuel, limit);
                }
                fuel -= cost;
                t += dt;
                if action >= 0 {
                    pos = p.map.neighbor(pos, action as usize).unwrap();
                }
                if supply.at[t as usize][pos] {
                    fuel = limit;
                }
                plan.push(if action < 0 { -(dt as i64) } else { action });
            }
            if t < steps {
                plan.push(-((steps - t) as i64));
            }
            replay_route(p, net, supply, agent, &plan, steps).unwrap()
        }
        pub(super) fn replay_route(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agent: &Agent,
            plan: &[i64],
            steps: u16,
        ) -> Option<CarRoute> {
            let state = trace(p, net, supply, agent, plan, steps)?;
            let mask = state.mask[steps as usize];
            let brands = p
                .spots
                .iter()
                .enumerate()
                .filter(|(s, _)| mask >> s & 1 != 0)
                .fold(0u64, |bb, (_, s)| bb | (1 << s.brand));
            let &(_, pos, fuel) = state.bounds.last()?;
            Some(CarRoute {
                plan: plan.to_vec(),
                mask,
                brands,
                pos,
                fuel,
            })
        }
        #[derive(Clone)]
        struct State {
            pos: usize,
            fuel: u16,
            mask: u32,
            brands: u64,
            gain: f64,
            rank: f64,
            parent: usize,
            act: i64,
        }
        pub(super) fn improve(
            p: &Problem,
            net: &Network,
            supply: &Supply,
            agents: &[Agent],
            candidate: &mut Candidate,
            rng: &mut Rng,
            steps: u16,
            seen_brands: u64,
            new_bonus: f64,
            last: bool,
        ) -> bool {
            let cars = (0..agents.len())
                .filter(|&a| agents[a].kind == 0)
                .collect::<Vec<_>>();
            let a = cars[rng.n(cars.len())];
            let Some(before) = trace(p, net, supply, &agents[a], &candidate.plan[a], steps) else {
                return false;
            };
            if before.actions.len() < 3 {
                return false;
            }
            let tail = rng.n(3) == 0;
            let begin = if tail {
                let options = (0..before.actions.len())
                    .filter(|&i| before.bounds[i].0 + 64 >= steps && before.bounds[i].0 + 4 < steps)
                    .collect::<Vec<_>>();
                if options.is_empty() {
                    return false;
                }
                options[rng.n(options.len())]
            } else {
                rng.n(before.actions.len())
            };
            let start = before.bounds[begin].0;
            let horizon = (10 + rng.n(31)) as u16;
            let possible = (begin + 1..before.bounds.len())
                .filter(|&j| {
                    before.bounds[j].0 > start + 3 && before.bounds[j].0 <= start + horizon
                })
                .collect::<Vec<_>>();
            if !tail && possible.is_empty() {
                return false;
            }
            let finish = if tail {
                before.actions.len()
            } else {
                possible[rng.n(possible.len())]
            };
            let end = before.bounds[finish].0;
            let target = before.bounds[finish].1;
            let mut required = 0u16;
            for t in (end + 1..=steps).rev() {
                if supply.at[t as usize][before.pos[t as usize]] {
                    required = 0;
                }
                required += before.cost[t as usize];
            }
            let mut fixed = before.mask[start as usize];
            for t in end + 1..=steps {
                let s = net.spot_at[before.pos[t as usize]];
                if s >= 0 {
                    fixed |= 1 << s;
                }
            }
            let mut used = vec![0u8; p.spots.len()];
            let mut covered = 0u64;
            for (b, &mask) in candidate.masks.iter().enumerate() {
                if b == a {
                    continue;
                }
                for (s, spot) in p.spots.iter().enumerate() {
                    if mask >> s & 1 != 0 {
                        used[s] += 1;
                        covered |= 1 << spot.brand;
                    }
                }
            }
            for (s, spot) in p.spots.iter().enumerate() {
                if fixed >> s & 1 != 0 {
                    covered |= 1 << spot.brand;
                }
            }
            let mut dist = vec![u16::MAX; p.map.len()];
            dist[target] = 0;
            let mut heap = BinaryHeap::new();
            heap.push(Reverse((0u16, target)));
            while let Some(Reverse((d, v))) = heap.pop() {
                if dist[v] != d {
                    continue;
                }
                for prev in p.map.neighbors(v) {
                    if p.map.terrain(prev) == 3 {
                        continue;
                    }
                    let nd = d + net.time[prev] as u16;
                    if nd < dist[prev] {
                        dist[prev] = nd;
                        heap.push(Reverse((nd, prev)));
                    }
                }
            }
            if tail {
                dist.fill(0);
            }
            let mut buckets = vec![Vec::new(); (end - start + 1) as usize];
            let (_, initial, fuel) = before.bounds[begin];
            buckets[0].push(State {
                pos: initial,
                fuel,
                mask: fixed,
                brands: covered,
                gain: 0.0,
                rank: 0.0,
                parent: usize::MAX,
                act: 0,
            });
            let mut arena = Vec::<State>::new();
            let mut best = None;
            let mut value = f64::NEG_INFINITY;
            let width = 180;
            for t in start..=end {
                let states = std::mem::take(&mut buckets[(t - start) as usize]);
                let mut frontier = SearchMap::<(usize, u32), State>::default();
                for b in states {
                    match frontier.entry((b.pos, b.mask)) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(b);
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            if b.fuel > entry.get().fuel {
                                entry.insert(b);
                            }
                        }
                    }
                }
                let mut states = frontier.into_values().collect::<Vec<_>>();
                states.sort_unstable_by(|a, b| {
                    b.rank
                        .total_cmp(&a.rank)
                        .then(b.fuel.cmp(&a.fuel))
                        .then(a.mask.cmp(&b.mask))
                        .then(a.pos.cmp(&b.pos))
                });
                for b in states.into_iter().take(width) {
                    let id = arena.len();
                    arena.push(b.clone());
                    if t == end {
                        if (tail || b.pos == target) && b.fuel >= required {
                            let v = b.gain
                                + if tail {
                                    super::endpoint_value(
                                        p, net, b.pos, t, b.fuel, supply, steps, last,
                                    )
                                } else {
                                    b.fuel as f64 * 0.02
                                };
                            if v > value {
                                value = v;
                                best = Some(id);
                            }
                        }
                        continue;
                    }
                    for act in -1i64..6 {
                        let (pos, dt, cost) = if act < 0 {
                            (b.pos, 1u16, 0u16)
                        } else {
                            let Some(to) = p.map.neighbor(b.pos, act as usize) else {
                                continue;
                            };
                            if p.map.terrain(to) == 3 {
                                continue;
                            }
                            (to, net.time[b.pos] as u16, net.cost[b.pos] as u16)
                        };
                        let tt = t + dt;
                        if tt > end || dist[pos] > end - tt {
                            continue;
                        }
                        let mut fuel = b.fuel;
                        if dt > 1 {
                            fuel = supply.wait_fuel(b.pos, t, tt - 1, fuel, p.fuel_limits as u16);
                        }
                        if fuel < cost {
                            continue;
                        }
                        fuel -= cost;
                        if supply.at[tt as usize][pos] {
                            fuel = p.fuel_limits as u16;
                        }
                        let mut q = State {
                            pos,
                            fuel,
                            mask: b.mask,
                            brands: b.brands,
                            gain: b.gain,
                            rank: 0.0,
                            parent: id,
                            act,
                        };
                        for at in [if dt > 1 { b.pos } else { pos }, pos] {
                            let s = net.spot_at[at];
                            if s < 0 {
                                continue;
                            }
                            let s = s as usize;
                            if q.mask >> s & 1 != 0 {
                                continue;
                            }
                            q.mask |= 1 << s;
                            let brand = 1 << p.spots[s].brand;
                            if used[s] as u32 >= p.spots[s].stocks {
                                q.brands |= brand;
                                continue;
                            }
                            q.gain += 10.0;
                            if q.brands & brand == 0 {
                                q.gain += 1000.0
                                    + if seen_brands & brand == 0 {
                                        new_bonus
                                    } else {
                                        0.0
                                    };
                            }
                            q.brands |= brand;
                        }
                        let mut potential = 0.0f64;
                        for (s, spot) in p.spots.iter().enumerate() {
                            if q.mask >> s & 1 != 0 || used[s] as u32 >= spot.stocks {
                                continue;
                            }
                            let d = net.dist_t[s][pos];
                            if d > end - tt {
                                continue;
                            }
                            let reward = if q.brands & (1 << spot.brand) == 0 {
                                1000.0
                            } else {
                                10.0
                            };
                            potential = potential.max(reward * 2.0 / (d as f64 + 2.0));
                        }
                        q.rank = q.gain
                            + potential
                            + if tail {
                                super::endpoint_value(p, net, pos, tt, fuel, supply, steps, last)
                            } else {
                                fuel.min(required + 10) as f64 * 0.04 - dist[pos] as f64 * 0.02
                            };
                        buckets[(tt - start) as usize].push(q);
                    }
                }
            }
            let Some(mut id) = best else {
                return false;
            };
            let mut replacement = Vec::new();
            while arena[id].parent != usize::MAX {
                replacement.push(arena[id].act);
                id = arena[id].parent;
            }
            replacement.reverse();
            let mut raw = before.actions[..begin].to_vec();
            raw.extend(replacement);
            raw.extend_from_slice(&before.actions[finish..]);
            let mut plan = Vec::new();
            for act in raw {
                if act < 0 && plan.last().is_some_and(|x| *x < 0) {
                    *plan.last_mut().unwrap() -= 1;
                } else {
                    plan.push(act);
                }
            }
            let Some(after) = trace(p, net, supply, &agents[a], &plan, steps) else {
                return false;
            };
            candidate.plan[a] = plan;
            candidate.masks[a] = after.mask[steps as usize];
            candidate.end[a].pos = after.bounds.last().unwrap().1;
            candidate.end[a].fuel = after.bounds.last().unwrap().2 as u32;
            true
        }
    }
    use std::{
        cmp::Reverse,
        collections::{BinaryHeap, HashMap},
        hash::{BuildHasherDefault, Hasher},
        time::{Duration, Instant},
    };
    use tester::engine::Game;
    use tester::model::{Agent, Observation, Plan, Problem, Traffic};
    const MODES: usize = 3;

    // Search keys contain only small integer positions, visit masks and times.
    // A fixed inexpensive mixer avoids the cost of hashing general byte strings
    // for every successor in the time-expanded route search.
    #[derive(Default)]
    struct SearchHasher(u64);
    impl Hasher for SearchHasher {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.write_u64(byte as u64);
            }
        }
        fn write_u64(&mut self, value: u64) {
            self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517cc1b727220a95);
        }
        fn write_usize(&mut self, value: usize) {
            self.write_u64(value as u64);
        }
        fn write_u32(&mut self, value: u32) {
            self.write_u64(value as u64);
        }
        fn write_u16(&mut self, value: u16) {
            self.write_u64(value as u64);
        }
    }
    type SearchMap<K, V> = HashMap<K, V, BuildHasherDefault<SearchHasher>>;

    #[derive(Clone)]
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 7;
            self.0 ^= self.0 >> 9;
            self.0 ^= self.0 << 8;
            self.0
        }
        fn n(&mut self, n: usize) -> usize {
            (self.next() as usize) % n
        }
        fn f(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
    }
    #[derive(Clone, Default)]
    struct Path {
        moves: Vec<(u16, u8, u8, u8)>,
        time: u16,
        fuel: u16,
    }
    #[derive(Clone)]
    struct Network {
        pos: Vec<usize>,
        targets: Vec<usize>,
        paths: Vec<Path>,
        n: usize,
        spot_at: Vec<i8>,
        dist_t: Vec<Vec<u16>>,
        dist_f: Vec<Vec<u16>>,
        escape_fuel: Vec<u16>,
        near: Vec<Vec<usize>>,
        time: Vec<u8>,
        cost: Vec<u8>,
    }
    impl Network {
        fn new(p: &Problem, agents: &[Agent], traffic: &[Traffic]) -> Self {
            let mut pos = p.spots.iter().map(|s| s.pos).collect::<Vec<_>>();
            pos.extend(agents.iter().map(|a| a.pos));
            Self::from_positions(p, pos, traffic)
        }
        fn from_positions(p: &Problem, pos: Vec<usize>, traffic: &[Traffic]) -> Self {
            let n = pos.len();
            let canonical = (0..n)
                .map(|i| pos[..i].iter().position(|&v| v == pos[i]).unwrap_or(i))
                .collect::<Vec<_>>();
            let targets = (0..n).filter(|&i| canonical[i] == i).collect::<Vec<_>>();
            let len = p.map.len();
            let mut time = vec![2; len];
            let mut cost = vec![1; len];
            for v in 0..len {
                match p.map.terrain(v) {
                    1 => {
                        time[v] = 1;
                        cost[v] = 2
                    }
                    2 => {
                        time[v] = 3;
                        cost[v] = 2
                    }
                    _ => {}
                }
            }
            for t in traffic {
                time[t.pos] = [1, 2, 4][t.status as usize];
            }
            let mut paths = vec![Path::default(); n * n * MODES];
            for (s, &start) in pos.iter().enumerate() {
                if canonical[s] != s {
                    for j in 0..n * MODES {
                        paths[s * n * MODES + j] = paths[canonical[s] * n * MODES + j].clone();
                    }
                    continue;
                }
                for mode in 0..MODES {
                    let weight = [0, 2, 9][mode];
                    let mut ds = vec![u32::MAX; len];
                    let mut prev = vec![(usize::MAX, 0u8); len];
                    let mut q = BinaryHeap::new();
                    ds[start] = 0;
                    q.push(Reverse((0u32, start)));
                    while let Some(Reverse((d, v))) = q.pop() {
                        if ds[v] != d {
                            continue;
                        }
                        for dir in 0..6 {
                            if let Some(to) = p.map.neighbor(v, dir) {
                                if p.map.terrain(to) == 3 {
                                    continue;
                                }
                                let nd = d + time[v] as u32 * 4 + cost[v] as u32 * weight;
                                if nd < ds[to] {
                                    ds[to] = nd;
                                    prev[to] = (v, dir as u8);
                                    q.push(Reverse((nd, to)));
                                }
                            }
                        }
                    }
                    for (target, &finish) in pos.iter().enumerate() {
                        let mut v = finish;
                        let mut moves = Vec::new();
                        let mut tt = 0;
                        let mut ff = 0;
                        while v != start {
                            let (from, dir) = prev[v];
                            if from == usize::MAX {
                                break;
                            }
                            moves.push((v as u16, dir, time[from], cost[from]));
                            tt += time[from] as u16;
                            ff += cost[from] as u16;
                            v = from;
                        }
                        moves.reverse();
                        paths[(s * n + target) * MODES + mode] = Path {
                            moves,
                            time: tt,
                            fuel: ff,
                        };
                    }
                }
            }
            let mut spot_at = vec![-1; len];
            for (i, s) in p.spots.iter().enumerate() {
                spot_at[s.pos] = i as i8;
            }
            let mut dist_t = vec![vec![u16::MAX; len]; n];
            let mut dist_f = dist_t.clone();
            for (i, &target) in pos.iter().enumerate() {
                if canonical[i] != i {
                    dist_t[i] = dist_t[canonical[i]].clone();
                    dist_f[i] = dist_f[canonical[i]].clone();
                    continue;
                }
                for mode in 0..2 {
                    let mut dd = vec![u16::MAX; len];
                    dd[target] = 0;
                    let mut heap = BinaryHeap::new();
                    heap.push(Reverse((0u16, target)));
                    while let Some(Reverse((d, v))) = heap.pop() {
                        if d != dd[v] {
                            continue;
                        }
                        for prev in p.map.neighbors(v) {
                            if p.map.terrain(prev) == 3 {
                                continue;
                            }
                            let nd = d + if mode == 0 {
                                time[prev] as u16
                            } else {
                                cost[prev] as u16
                            };
                            if nd < dd[prev] {
                                dd[prev] = nd;
                                heap.push(Reverse((nd, prev)));
                            }
                        }
                    }
                    if mode == 0 {
                        dist_t[i] = dd;
                    } else {
                        dist_f[i] = dd;
                    }
                }
            }
            let escape_fuel = (0..len)
                .map(|cell| {
                    (0..p.spots.len())
                        .filter(|&spot| p.spots[spot].pos != cell)
                        .map(|spot| dist_f[spot][cell])
                        .min()
                        .unwrap_or(0)
                })
                .collect();
            let near = (0..len)
                .map(|v| {
                    let mut order = (0..p.spots.len()).collect::<Vec<_>>();
                    order.sort_unstable_by_key(|&s| dist_t[s][v]);
                    order
                })
                .collect();
            Self {
                pos,
                targets,
                paths,
                n,
                spot_at,
                dist_t,
                dist_f,
                escape_fuel,
                near,
                time,
                cost,
            }
        }
        fn path(&self, a: usize, b: usize, m: usize) -> &Path {
            &self.paths[(a * self.n + b) * MODES + m]
        }
    }
    #[derive(Clone)]
    struct SupportRoute {
        stops: Vec<(usize, u16)>,
        direct: Option<Vec<i64>>,
    }
    #[derive(Clone)]
    struct Supply {
        at: Vec<Vec<bool>>,
        next: Vec<Vec<u16>>,
        plans: Plan,
        end: Vec<usize>,
        nodes: Vec<usize>,
        windows: Vec<(usize, u16, u16)>,
        end_t: Vec<u16>,
        end_f: Vec<u16>,
    }
    impl Supply {
        fn build(
            p: &Problem,
            agents: &[Agent],
            net: &Network,
            routes: &[SupportRoute],
            steps: u16,
        ) -> Self {
            let mut at = vec![vec![false; p.map.len()]; steps as usize + 1];
            let mut occupied = vec![Vec::<usize>::new(); steps as usize + 1];
            let mut plans = vec![Vec::new(); agents.len()];
            let mut end = vec![0; agents.len()];
            let mut ri = 0;
            for (a, ag) in agents.iter().enumerate() {
                if ag.kind != 1 {
                    continue;
                }
                let route = &routes[ri];
                let mut node = p.spots.len() + a;
                let mut t = 0u16;
                if let Some(direct) = &route.direct {
                    plans[a] = direct.clone();
                } else {
                    for &(dest, wait) in &route.stops {
                        let path = net.path(node, dest, 0);
                        if t + path.time > steps {
                            break;
                        }
                        plans[a].extend(path.moves.iter().map(|&(_, dir, _, _)| dir as i64));
                        t += path.time;
                        node = dest;
                        let wait = wait.min(steps - t);
                        if wait > 0 {
                            plans[a].push(-(wait as i64));
                            t += wait;
                        }
                    }
                    if t < steps {
                        plans[a].push(-((steps - t) as i64));
                    }
                }
                let mut pos = ag.pos;
                t = 0;
                at[0][pos] = true;
                let direct_actions = std::mem::take(&mut plans[a]);
                for act in direct_actions {
                    let act = if act < 0 {
                        act.max(-((steps - t) as i64))
                    } else {
                        act
                    };
                    let dt = if act < 0 {
                        (-act) as u16
                    } else {
                        net.time[pos] as u16
                    };
                    if dt == 0 || t + dt > steps {
                        break;
                    }
                    plans[a].push(act);
                    let dest = if act < 0 {
                        pos
                    } else {
                        p.map.neighbor(pos, act as usize).unwrap()
                    };
                    for k in 1..=dt {
                        t += 1;
                        if t > steps {
                            break;
                        }
                        if k == dt {
                            pos = dest;
                        }
                        at[t as usize][pos] = true;
                        occupied[t as usize].push(pos);
                    }
                    if t >= steps {
                        break;
                    }
                }
                if t < steps {
                    plans[a].push(-((steps - t) as i64));
                    for k in t + 1..=steps {
                        at[k as usize][pos] = true;
                        occupied[k as usize].push(pos);
                    }
                }
                end[a] = pos;
                ri += 1;
            }
            let mut nodes = Vec::new();
            let mut windows = Vec::new();
            for &node in &net.targets {
                let pos = net.pos[node];
                let mut t = 0;
                while t <= steps {
                    if !at[t as usize][pos] {
                        t += 1;
                        continue;
                    }
                    let start = t;
                    while t < steps && at[t as usize + 1][pos] {
                        t += 1;
                    }
                    windows.push((node, start, t));
                    nodes.push(node);
                    t += 1;
                }
            }
            nodes.sort_unstable();
            nodes.dedup();
            let mut next = vec![vec![u16::MAX; p.map.len()]; steps as usize + 1];
            for t in (0..steps as usize).rev() {
                let (earlier, later) = next.split_at_mut(t + 1);
                earlier[t].copy_from_slice(&later[0]);
                for &pos in &occupied[t + 1] {
                    earlier[t][pos] = (t + 1) as u16;
                }
            }
            let mut distances = Vec::new();
            let depot_nodes = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(a, _)| net.pos.iter().position(|&v| v == end[a]))
                .collect::<Option<Vec<_>>>();
            for mode in 0..2 {
                let mut dd = vec![u16::MAX; p.map.len()];
                if let Some(depots) = depot_nodes.as_ref() {
                    let rows = if mode == 0 { &net.dist_t } else { &net.dist_f };
                    for &node in depots {
                        for (distance, &known) in dd.iter_mut().zip(&rows[node]) {
                            *distance = (*distance).min(known);
                        }
                    }
                    distances.push(dd);
                    continue;
                }
                let mut heap = BinaryHeap::new();
                for (a, ag) in agents.iter().enumerate() {
                    if ag.kind == 1 {
                        dd[end[a]] = 0;
                        heap.push(Reverse((0u16, end[a])));
                    }
                }
                while let Some(Reverse((d, v))) = heap.pop() {
                    if d != dd[v] {
                        continue;
                    }
                    for prev in p.map.neighbors(v) {
                        if p.map.terrain(prev) == 3 {
                            continue;
                        }
                        let nd = d + if mode == 0 {
                            net.time[prev] as u16
                        } else {
                            net.cost[prev] as u16
                        };
                        if nd < dd[prev] {
                            dd[prev] = nd;
                            heap.push(Reverse((nd, prev)));
                        }
                    }
                }
                distances.push(dd);
            }
            Self {
                at,
                next,
                plans,
                end,
                nodes,
                windows,
                end_t: distances.remove(0),
                end_f: distances.remove(0),
            }
        }
        fn wait_fuel(&self, pos: usize, t: u16, end: u16, fuel: u16, limit: u16) -> u16 {
            if self.next[t as usize][pos] <= end {
                limit
            } else {
                fuel
            }
        }
    }
    #[derive(Clone)]
    struct Beam {
        node: usize,
        t: u16,
        fuel: u16,
        mask: u32,
        brands: u64,
        gain: f64,
        rank: f64,
        route: Vec<(usize, u8, u16)>,
    }
    #[derive(Clone)]
    struct CarRoute {
        plan: Vec<i64>,
        mask: u32,
        brands: u64,
        pos: usize,
        fuel: u16,
    }
    #[derive(Clone)]
    struct Candidate {
        plan: Plan,
        score: f64,
        masks: Vec<u32>,
        end: Vec<Agent>,
        support: Vec<SupportRoute>,
    }
    pub struct Solver {
        p: Problem,
        game: Option<Game>,
        seen: u64,
        last: Option<Plan>,
        rng: Rng,
        chosen: Vec<u8>,
        opening: Option<Candidate>,
        tour_stats: [usize; 11],
        captured_routes: Option<Vec<CarRoute>>,
        beam_scale: usize,
        stock_layers: [u32; 8],
    }
    impl Solver {
        pub fn new(p: Problem, seed: u64) -> Self {
            let mut stock_layers = [0u32; 8];
            for (s, spot) in p.spots.iter().enumerate() {
                for layer in stock_layers.iter_mut().take(spot.stocks as usize) {
                    *layer |= 1 << s;
                }
            }
            Self {
                p,
                game: None,
                seen: 0,
                last: None,
                rng: Rng(seed.wrapping_add(0x294caf05953e31)),
                chosen: Vec::new(),
                opening: None,
                tour_stats: [0; 11],
                captured_routes: None,
                beam_scale: 1,
                stock_layers,
            }
        }
        pub fn select(&mut self, ms: u64) -> Vec<u8> {
            // Compare the initial position of a single supplier using route search.
            let n = self.p.agents.len();
            let agents = self
                .p
                .agents
                .iter()
                .map(|&pos| Agent {
                    kind: 0,
                    pos,
                    fuel: self.p.fuel_limits,
                })
                .collect::<Vec<_>>();
            let net = Network::new(&self.p, &agents, &[]);
            let mut central = (0..n)
                .map(|a| {
                    let mut d = self
                        .p
                        .spots
                        .iter()
                        .enumerate()
                        .map(|(s, _)| net.path(self.p.spots.len() + a, s, 1).time as u32)
                        .collect::<Vec<_>>();
                    d.sort_unstable();
                    (d.iter().take(self.p.spots.len() * 3 / 4).sum::<u32>(), a)
                })
                .collect::<Vec<_>>();
            central.sort_unstable();
            self.chosen = vec![0; n];
            self.chosen[central[0].1] = 1;
            let start = Instant::now();
            let budget = Duration::from_millis((ms * 80 / 100).saturating_sub(100));
            let mut choices = Vec::<(Candidate, Vec<u8>)>::new();
            let mut best = f64::NEG_INFINITY;
            let mut round = 0;
            while start.elapsed() < budget {
                self.beam_scale = if start.elapsed() >= budget / 2 {
                    1 + (ms / 30000).min(2) as usize
                } else {
                    1
                };
                let mut aa = agents.clone();
                aa[round % n].kind = 1;
                let steps = self.p.day_steps[0] as u16;
                let kinds = aa.iter().map(|a| a.kind).collect::<Vec<_>>();
                let previous = choices
                    .iter()
                    .find(|(_, kk)| kk == &kinds)
                    .map(|(c, _)| c.clone());
                let candidate;
                let epoch = round / n;
                if round >= n && epoch % 3 != 0 {
                    let mut c = previous.clone().unwrap();
                    let supply = Supply::build(&self.p, &aa, &net, &c.support, steps);
                    if epoch % 11 == 1 {
                        if epoch % 22 == 1 {
                            tour::improve(self, &aa, &net, &supply, &mut c, 0, steps, 1800);
                        } else {
                            route_set::improve(self, &aa, &net, &supply, &mut c, 0, steps);
                        }
                    } else if epoch % 19 == 2 {
                        joint::improve(self, &aa, &net, &supply, &mut c, 0, steps, 800);
                    } else {
                        self.rebuild_subset(&mut c, &aa, &net, &supply, 0, steps, round, 45);
                    }
                    candidate = c;
                } else {
                    let mut routes = previous
                        .as_ref()
                        .map(|c| c.support.clone())
                        .unwrap_or_else(|| self.initial_support(&aa, &net, steps));
                    if round >= n {
                        if epoch % 6 == 0 {
                            routes = escort::optimize(
                                &self.p,
                                &net,
                                &aa,
                                previous.as_ref().unwrap(),
                                steps,
                                self.p.day_steps.len() == 1,
                                160,
                                &mut self.rng,
                            );
                        } else {
                            self.mutate_support(&mut routes, &aa, &net, steps, round);
                        }
                    }
                    let supply = Supply::build(&self.p, &aa, &net, &routes, steps);
                    let preserved = previous.as_ref().and_then(|c| {
                        self.preserve_patrols(c, &aa, &net, &supply, routes.clone(), 0, steps)
                    });
                    let built =
                        self.build_candidate(&aa, &net, &supply, routes, 0, steps, round, 35);
                    candidate = if let Some(c) = preserved {
                        if c.score > built.score { c } else { built }
                    } else {
                        built
                    };
                }
                if let Some(i) = choices.iter().position(|(_, kk)| kk == &kinds) {
                    if candidate.score > choices[i].0.score {
                        choices[i] = (candidate.clone(), kinds.clone());
                    }
                } else {
                    choices.push((candidate.clone(), kinds.clone()));
                }
                if candidate.score > best {
                    best = candidate.score;
                    self.chosen = kinds;
                    self.opening = Some(candidate);
                }
                round += 1;
            }
            eprintln!(
                "selection rounds={} alternatives={:?}",
                round,
                choices
                    .iter()
                    .map(|(c, kk)| (
                        kk.iter().position(|&k| k == 1).unwrap(),
                        c.score.round() as i64
                    ))
                    .collect::<Vec<_>>()
            );
            self.game = Some(Game::new(self.p.clone(), &self.chosen));
            self.chosen.clone()
        }
        pub fn plan(&mut self, obs: Observation, accepted: bool, ms: u64) -> Plan {
            self.beam_scale = 1;
            self.tour_stats = [0; 11];
            let start = Instant::now();
            let budget = Duration::from_millis((ms * 86 / 100).saturating_sub(60).max(20));
            let previous_plan = if accepted { self.last.clone() } else { None };
            let game = self.game.as_mut().unwrap();
            if let Some(last) = self.last.take() {
                let plan = if accepted { last } else { game.waiting_plan() };
                if let Ok(replay) = game.play_day(plan) {
                    for f in replay.frames {
                        for e in f.events {
                            if let tester::model::Event::Collect { brand, .. } = e {
                                self.seen |= 1u64 << brand;
                            }
                        }
                    }
                }
            }
            game.agents = obs.agents.clone();
            game.day = obs.day;
            game.set_observed_traffic(obs.traffics.clone());
            let p = self.p.clone();
            let steps = p.day_steps[obs.day] as u16;
            let net = Network::new(&p, &obs.agents, &obs.traffics);
            let mut base = self.initial_support(&obs.agents, &net, steps);
            let mut best: Option<Candidate> = None;
            let mut incumbent: Option<Candidate> = None;
            let mut iterations = 0;
            let mut projected_best = f64::NEG_INFINITY;
            let mut best_current = f64::NEG_INFINITY;
            let mut lookahead =
                obs.agents.len() <= 4 && obs.day + 1 < p.day_steps.len() && ms >= 5000;
            let all_brands = p.spots.iter().fold(0u64, |mask, s| mask | (1 << s.brand));
            let mut endings = Vec::new();
            if obs.day == 0 {
                if let Some(c) = self.opening.take() {
                    projected_best = c.score;
                    best_current = c.score;
                    base = c.support.clone();
                    best = Some(c.clone());
                    incumbent = Some(c);
                }
            }

            if let Some(previous) = previous_plan {
                let candidate =
                    self.reversed_candidate(&previous, &obs.agents, &net, obs.day, steps);
                if best.as_ref().is_none_or(|c| candidate.score > c.score) {
                    projected_best = candidate.score;
                    best_current = candidate.score;
                    base = candidate.support.clone();
                    best = Some(candidate.clone());
                    incumbent = Some(candidate);
                }
            }

            if lookahead {
                if let Some(c) = best.as_ref() {
                    if self.statistics(c).0 == all_brands {
                        lookahead = false;
                    } else {
                        tomorrow::retain(self, &mut endings, c);
                    }
                }
            }
            while start.elapsed() < if lookahead { budget * 3 / 4 } else { budget }
                || best.is_none()
            {
                self.beam_scale = if start.elapsed() >= budget / 2 {
                    1 + (ms / 30000).min(2) as usize
                } else {
                    1
                };
                let mut candidate;
                if iterations > 0 && iterations % 4 != 0 {
                    let source = if self.rng.n(3) == 0 {
                        incumbent.as_ref().unwrap()
                    } else {
                        best.as_ref().unwrap()
                    };
                    candidate = source.clone();
                    let supply = Supply::build(&p, &obs.agents, &net, &candidate.support, steps);
                    if iterations % 48 == 37 {
                        route_set::improve(
                            self,
                            &obs.agents,
                            &net,
                            &supply,
                            &mut candidate,
                            obs.day,
                            steps,
                        );
                    } else if iterations % 24 == 13 {
                        joint::improve(
                            self,
                            &obs.agents,
                            &net,
                            &supply,
                            &mut candidate,
                            obs.day,
                            steps,
                            3500,
                        );
                    } else if iterations % 12 == 1 {
                        tour::improve(
                            self,
                            &obs.agents,
                            &net,
                            &supply,
                            &mut candidate,
                            obs.day,
                            steps,
                            8000,
                        );
                    } else if iterations % 3 == 1 {
                        let new_bonus = self.fresh_bonus(obs.day);
                        window::improve(
                            &self.p,
                            &net,
                            &supply,
                            &obs.agents,
                            &mut candidate,
                            &mut self.rng,
                            steps,
                            self.seen,
                            new_bonus,
                            obs.day + 1 == p.day_steps.len(),
                        );
                        candidate.score = self.local_value(
                            &candidate.masks,
                            &candidate.end,
                            &net,
                            &supply,
                            obs.day,
                            steps,
                        );
                    } else {
                        self.rebuild_subset(
                            &mut candidate,
                            &obs.agents,
                            &net,
                            &supply,
                            obs.day,
                            steps,
                            iterations,
                            if ms >= 10000 { 110 } else { 65 },
                        );
                    }
                } else if iterations % (if obs.agents.len() <= 5 { 32 } else { 64 }) == 8
                    && best.is_some()
                {
                    let source = best.as_ref().unwrap().clone();
                    candidate =
                        patrol_first::build(self, &obs.agents, &net, &source, obs.day, steps);
                } else {
                    let source =
                        if iterations % 11 == 0 || incumbent.is_none() || obs.agents.len() <= 5 {
                            best.as_ref()
                        } else {
                            incumbent.as_ref()
                        }
                        .cloned();
                    let mut routes = source
                        .as_ref()
                        .map(|c| c.support.clone())
                        .unwrap_or_else(|| base.clone());
                    if iterations > 0 {
                        if iterations % 16 == 0 {
                            routes = escort::optimize(
                                &p,
                                &net,
                                &obs.agents,
                                source.as_ref().unwrap(),
                                steps,
                                obs.day + 1 == p.day_steps.len(),
                                180,
                                &mut self.rng,
                            );
                        } else if iterations % 8 == 0 {
                            routes = if (iterations / 16) % 4 == 3 {
                                self.rendezvous_support(
                                    source.as_ref().unwrap(),
                                    &obs.agents,
                                    &net,
                                    steps,
                                )
                            } else {
                                self.hub_support(&obs.agents, &net, steps, iterations / 16)
                            };
                        } else {
                            self.mutate_support(&mut routes, &obs.agents, &net, steps, iterations);
                        }
                    }
                    let supply = Supply::build(&p, &obs.agents, &net, &routes, steps);
                    let preserved = source.as_ref().and_then(|c| {
                        self.preserve_patrols(
                            c,
                            &obs.agents,
                            &net,
                            &supply,
                            routes.clone(),
                            obs.day,
                            steps,
                        )
                    });
                    if iterations > 0 && iterations % 16 != 8 && preserved.is_some() {
                        candidate = preserved.unwrap();
                    } else {
                        candidate = self.build_candidate(
                            &obs.agents,
                            &net,
                            &supply,
                            routes,
                            obs.day,
                            steps,
                            iterations,
                            if ms >= 10000 { 70 } else { 45 },
                        );
                        if let Some(preserved) = preserved {
                            if preserved.score > candidate.score {
                                candidate = preserved;
                            }
                        }
                    }
                    if iterations % 16 != 8 {
                        if let Some(source) = source.as_ref() {
                            self.polish_service(
                                &mut candidate,
                                source,
                                &obs.agents,
                                &net,
                                &supply,
                                obs.day,
                                steps,
                            );
                        }
                    }
                }
                let score = candidate.score;
                if lookahead {
                    if self.statistics(&candidate).0 == all_brands {
                        lookahead = false;
                        endings.clear();
                    } else {
                        tomorrow::retain(self, &mut endings, &candidate);
                    }
                }
                best_current = best_current.max(score);
                if score >= best_current - 0.01 || best.is_none() {
                    let forecast = candidate.score;
                    if forecast > projected_best {
                        projected_best = forecast;
                        best = Some(candidate.clone());
                        base = candidate.support.clone();
                    }
                }
                let temp = 5.0
                    + 350.0 * (1.0 - start.elapsed().as_secs_f64() / budget.as_secs_f64()).max(0.0);
                if incumbent.as_ref().is_none_or(|b| score >= b.score)
                    || self.rng.f() < ((score - incumbent.as_ref().unwrap().score) / temp).exp()
                {
                    incumbent = Some(candidate);
                }
                iterations += 1;
            }
            let selected = if lookahead {
                tomorrow::choose(
                    self,
                    endings,
                    best.unwrap(),
                    obs.day,
                    &obs.traffics,
                    budget.saturating_sub(start.elapsed()),
                )
            } else {
                best.unwrap()
            };
            eprintln!(
                "local_max={:.1} chosen={:.1} search_best={:.1}",
                best_current, selected.score, projected_best
            );
            let (expected_brands, expected_udon) = self.statistics(&selected);
            let expected_end = selected.end;
            let mut plan = selected.plan;
            let mut check = self.game.as_ref().unwrap().clone();
            let old_score = check.score.clone();
            match check.play_day(plan.clone()) {
                Err(e) => {
                    eprintln!("validation fallback {}", e.message);
                    plan = self.game.as_ref().unwrap().waiting_plan();
                }
                Ok(_) => {
                    if check.agents != expected_end {
                        eprintln!(
                            "ENDPOINT_MISMATCH predicted={:?} actual={:?}",
                            expected_end, check.agents
                        );
                    }
                    if check.score.daily_brands - old_score.daily_brands
                        != expected_brands.count_ones() as usize
                        || check.score.udon - old_score.udon != expected_udon
                    {
                        eprintln!(
                            "PREDICTION_MISMATCH expected={}/{} actual={}/{}",
                            expected_brands.count_ones(),
                            expected_udon,
                            check.score.daily_brands - old_score.daily_brands,
                            check.score.udon - old_score.udon
                        );
                    }
                }
            }
            eprintln!(
                "day={} brands={} food={} iterations={} elapsed_ms={} tour={:?}",
                obs.day,
                expected_brands.count_ones(),
                expected_udon,
                iterations,
                start.elapsed().as_millis(),
                self.tour_stats
            );
            self.last = Some(plan.clone());
            plan
        }
        fn statistics(&self, c: &Candidate) -> (u64, u32) {
            self.mask_statistics(&c.masks)
        }
        fn mask_statistics(&self, masks: &[u32]) -> (u64, u32) {
            let mut counts = [0u32; 8];
            for &mask in masks {
                let mut carry = mask;
                for layer in &mut counts {
                    if carry == 0 {
                        break;
                    }
                    let next = *layer & carry;
                    *layer |= carry;
                    carry = next;
                }
            }
            let food = counts
                .iter()
                .zip(self.stock_layers)
                .map(|(&m, stock)| (m & stock).count_ones())
                .sum();
            let mut mask = counts[0];
            let mut brands = 0u64;
            while mask != 0 {
                brands |= 1 << self.p.spots[mask.trailing_zeros() as usize].brand;
                mask &= mask - 1;
            }
            (brands, food)
        }
        fn fresh_bonus(&self, day: usize) -> f64 {
            match self.p.day_steps.len() - day {
                1 => 100000.0,
                2 => 2000.0,
                _ => 300.0,
            }
        }
        fn initial_support(
            &self,
            agents: &[Agent],
            net: &Network,
            steps: u16,
        ) -> Vec<SupportRoute> {
            let spots = self.p.spots.len();
            agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(i, _)| {
                    let node = spots + i;
                    let best = (0..net.n)
                        .min_by_key(|&s| {
                            let patrol = agents
                                .iter()
                                .enumerate()
                                .filter(|&(a, ag)| {
                                    ag.kind == 0
                                        && agents
                                            .iter()
                                            .enumerate()
                                            .filter(|(_, b)| b.kind == 1)
                                            .min_by_key(|&(b, _)| {
                                                net.path(spots + a, spots + b, 0).time
                                            })
                                            .is_some_and(|(b, _)| b == i)
                                })
                                .map(|(a, _)| net.path(spots + a, s, 2).fuel as u32)
                                .sum::<u32>();
                            let d = net.path(node, s, 0).time as u32;
                            patrol + d * 2
                        })
                        .unwrap();
                    let weak = agents
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.kind == 0 && a.fuel < self.p.fuel_limits / 3)
                        .min_by_key(|(a, _)| net.path(node, spots + a, 0).time)
                        .map(|(a, _)| spots + a);
                    let stops = if let Some(weak) = weak {
                        if net.path(node, weak, 0).time + net.path(weak, best, 0).time + 3 < steps {
                            vec![(weak, 3), (best, steps)]
                        } else {
                            vec![(best, steps)]
                        }
                    } else {
                        vec![(best, steps)]
                    };
                    SupportRoute {
                        stops,
                        direct: None,
                    }
                })
                .collect()
        }
        fn hub_support(
            &self,
            agents: &[Agent],
            net: &Network,
            steps: u16,
            ordinal: usize,
        ) -> Vec<SupportRoute> {
            let spots = self.p.spots.len();
            agents
                .iter()
                .enumerate()
                .filter(|(_, ag)| ag.kind == 1)
                .map(|(a, _)| {
                    let start = spots + a;
                    let mut options = (0..spots)
                        .chain(std::iter::once(start))
                        .map(|hub| {
                            let arrive = net.path(start, hub, 0).time;
                            let density = self
                                .p
                                .spots
                                .iter()
                                .enumerate()
                                .map(|(s, spot)| {
                                    spot.stocks.min((agents.len() - 1) as u32) as f64
                                        / (net.path(hub, s, 0).time as f64 + 4.0)
                                })
                                .sum::<f64>();
                            (density - arrive as f64 * 0.003, hub)
                        })
                        .collect::<Vec<_>>();
                    options.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
                    let hub = options[ordinal % options.len().min(12)].1;
                    let mut weak = agents
                        .iter()
                        .enumerate()
                        .filter(|(_, ag)| ag.kind == 0 && ag.fuel < self.p.fuel_limits / 3)
                        .map(|(b, ag)| (ag.fuel, net.path(start, spots + b, 0).time, spots + b))
                        .collect::<Vec<_>>();
                    weak.sort_unstable();
                    let mut stops = Vec::new();
                    if agents.len() < 6 {
                        if let Some(&(_, _, target)) = weak
                            .iter()
                            .filter(|&&(fuel, _, _)| fuel < self.p.fuel_limits / 4)
                            .min_by_key(|&&(_, distance, _)| distance)
                        {
                            if net.path(start, target, 0).time + net.path(target, hub, 0).time + 2
                                < steps * 3 / 4
                            {
                                stops.push((target, 2));
                            }
                        }
                    } else if !weak.is_empty() {
                        let target = weak[(ordinal / 2) % weak.len()].2;
                        if net.path(start, target, 0).time < steps {
                            stops.push((target, 2));
                        }
                    }
                    stops.push((hub, steps));
                    SupportRoute {
                        stops,
                        direct: None,
                    }
                })
                .collect()
        }
        fn rendezvous_support(
            &mut self,
            c: &Candidate,
            agents: &[Agent],
            net: &Network,
            steps: u16,
        ) -> Vec<SupportRoute> {
            let old = Supply::build(&self.p, agents, net, &c.support, steps);
            let mut requests = Vec::new();
            for (a, ag) in agents.iter().enumerate() {
                if ag.kind != 0 {
                    continue;
                }
                let mut pos = ag.pos;
                let mut t = 0u16;
                let mut fuel = ag.fuel as u16;
                let mut options = Vec::new();
                if net.spot_at[pos] >= 0 {
                    options.push((
                        net.spot_at[pos] as usize,
                        0u16,
                        (self.p.fuel_limits as u16 - fuel + 2) as f64,
                    ));
                }
                for &action in &c.plan[a] {
                    if action < 0 {
                        let end = t + (-action) as u16;
                        fuel =
                            old.wait_fuel(pos, t, end.min(steps), fuel, self.p.fuel_limits as u16);
                        t = end;
                    } else {
                        let dt = net.time[pos] as u16;
                        let end = t + dt;
                        if end > steps {
                            break;
                        }
                        if dt > 1 {
                            fuel = old.wait_fuel(pos, t, end - 1, fuel, self.p.fuel_limits as u16);
                        }
                        fuel = fuel.saturating_sub(net.cost[pos] as u16);
                        pos = self.p.map.neighbor(pos, action as usize).unwrap();
                        t = end;
                        if net.spot_at[pos] >= 0 {
                            options.push((
                                net.spot_at[pos] as usize,
                                t,
                                (self.p.fuel_limits as u16 - fuel + 2) as f64,
                            ));
                        }
                        if old.at[t as usize][pos] {
                            fuel = self.p.fuel_limits as u16;
                        }
                    }
                }
                if !options.is_empty() {
                    let choice = options
                        .into_iter()
                        .max_by(|x, y| (x.2 * self.rng.f()).total_cmp(&(y.2 * self.rng.f())))
                        .unwrap();
                    requests.push((choice.0, choice.1));
                }
            }
            requests.sort_by_key(|&(_, t)| t);
            let mut out = Vec::new();
            let suppliers = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(a, _)| a)
                .collect::<Vec<_>>();
            for (ri, &a) in suppliers.iter().enumerate() {
                let mut node = self.p.spots.len() + a;
                let mut t = 0u16;
                let mut ss = Vec::new();
                for &(dest, want) in &requests {
                    let own = suppliers
                        .iter()
                        .enumerate()
                        .min_by_key(|&(_, b)| net.path(self.p.spots.len() + b, dest, 0).time)
                        .unwrap()
                        .0;
                    if own != ri {
                        continue;
                    }
                    let arrive = t + net.path(node, dest, 0).time;
                    let target = want.max(arrive).min(steps);
                    if arrive > steps || arrive > want + 10 {
                        continue;
                    }
                    let wait = (target + 2).min(steps) - arrive;
                    ss.push((dest, wait));
                    node = dest;
                    t = arrive + wait;
                }
                if ss.is_empty() {
                    out.push(c.support[ri].clone());
                } else {
                    out.push(SupportRoute {
                        stops: ss,
                        direct: None,
                    });
                }
            }
            out
        }
        fn mutate_direct(
            &mut self,
            route: &mut SupportRoute,
            start: usize,
            net: &Network,
            steps: u16,
        ) {
            let actions = route.direct.as_mut().unwrap();
            let old = std::mem::take(actions);
            let mut pos = start;
            let mut time = 0u16;
            for action in old {
                if time == steps {
                    break;
                }
                let dt = if action < 0 {
                    ((-action) as u16).min(steps - time)
                } else {
                    net.time[pos] as u16
                };
                if time + dt > steps {
                    break;
                }
                if action >= 0 {
                    pos = self.p.map.neighbor(pos, action as usize).unwrap();
                }
                actions.push(if action < 0 { -(dt as i64) } else { action });
                time += dt;
            }
            if time < steps {
                actions.push(-((steps - time) as i64));
            }
            let mut positions = vec![start];
            for &action in actions.iter() {
                let pos = *positions.last().unwrap();
                positions.push(if action < 0 {
                    pos
                } else {
                    self.p.map.neighbor(pos, action as usize).unwrap()
                });
            }
            let waits = actions
                .iter()
                .enumerate()
                .filter(|(_, x)| **x < 0)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            match self.rng.n(8) {
                0 | 1 if !waits.is_empty() => {
                    let i = waits[self.rng.n(waits.len())];
                    let delta = self.rng.n(21) as i64 - 10;
                    actions[i] = -((-actions[i] + delta).clamp(1, steps as i64));
                }
                2 if !waits.is_empty() => {
                    actions.remove(waits[self.rng.n(waits.len())]);
                }
                3 => {
                    let i = self.rng.n(actions.len() + 1);
                    let pos = positions[i];
                    let dirs = (0..6)
                        .filter(|&d| {
                            self.p
                                .map
                                .neighbor(pos, d)
                                .is_some_and(|v| self.p.map.terrain(v) != 3)
                        })
                        .collect::<Vec<_>>();
                    if !dirs.is_empty() {
                        let dir = dirs[self.rng.n(dirs.len())] as i64;
                        let mut detour = vec![dir];
                        if self.rng.n(2) == 0 {
                            detour.push(-((self.rng.n(8) + 1) as i64));
                        }
                        detour.push((dir + 3) % 6);
                        actions.splice(i..i, detour);
                    }
                }
                4 => {
                    let mut loops = Vec::new();
                    for i in 0..positions.len() {
                        for j in i + 2..positions.len() {
                            if positions[i] == positions[j] {
                                loops.push((i, j));
                            }
                        }
                    }
                    if !loops.is_empty() {
                        let (i, j) = loops[self.rng.n(loops.len())];
                        actions.drain(i..j);
                    }
                }
                5 if !waits.is_empty() => {
                    let i = waits[self.rng.n(waits.len())];
                    let wait = actions.remove(i);
                    let j = self.rng.n(actions.len() + 1);
                    actions.insert(j, wait);
                }
                6 => {
                    let nodes = positions
                        .iter()
                        .enumerate()
                        .filter_map(|(i, pos)| {
                            net.pos.iter().position(|p| p == pos).map(|n| (i, n))
                        })
                        .collect::<Vec<_>>();
                    if nodes.len() >= 2 {
                        let a = self.rng.n(nodes.len());
                        let b = self.rng.n(nodes.len());
                        if a != b {
                            let (i, u) = nodes[a.min(b)];
                            let (j, v) = nodes[a.max(b)];
                            let replacement = net
                                .path(u, v, 0)
                                .moves
                                .iter()
                                .map(|&(_, d, _, _)| d as i64)
                                .collect::<Vec<_>>();
                            actions.splice(i..j, replacement);
                        }
                    }
                }
                _ => {
                    let i = self.rng.n(actions.len() + 1);
                    actions.insert(i, -((self.rng.n(steps.min(12) as usize) + 1) as i64));
                }
            }
        }
        fn mutate_support(
            &mut self,
            routes: &mut [SupportRoute],
            agents: &[Agent],
            net: &Network,
            steps: u16,
            it: usize,
        ) {
            let r = self.rng.n(routes.len());
            let route = &mut routes[r];
            if route.direct.is_some() && self.rng.n(4) != 0 {
                let a = agents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.kind == 1)
                    .nth(r)
                    .unwrap()
                    .0;
                self.mutate_direct(route, agents[a].pos, net, steps);
                return;
            }
            route.direct = None;
            if route.stops.is_empty() {
                route.stops.push((self.rng.n(net.n), steps));
            }
            let spots = self.p.spots.len();
            let roll = self.rng.n(9);
            if roll == 8 {
                let agent = agents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.kind == 1)
                    .nth(r)
                    .unwrap()
                    .0;
                let order = circuit::supplier_circuit(
                    &self.p,
                    net,
                    spots + agent,
                    steps * 4 / 5,
                    &mut self.rng,
                );
                if !order.is_empty() {
                    route.stops = order.into_iter().map(|s| (s, 1)).collect();
                }
                return;
            }
            if roll >= 7 {
                let agent = agents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.kind == 1)
                    .nth(r)
                    .unwrap()
                    .0;
                let mut node = spots + agent;
                let mut remain = steps;
                let count = 2 + self.rng.n(3);
                let mut ss = Vec::new();
                if self.rng.n(3) == 0 {
                    let w = self.rng.n((steps / 4 + 1) as usize) as u16;
                    ss.push((node, w));
                    remain -= w;
                }
                for k in 0..count {
                    let options = (0..spots)
                        .filter(|&s| s != node && net.path(node, s, 0).time + 2 < remain)
                        .collect::<Vec<_>>();
                    if options.is_empty() {
                        break;
                    }
                    let target = if self.rng.n(3) == 0 {
                        let weak = agents
                            .iter()
                            .enumerate()
                            .filter(|(_, a)| a.kind == 0)
                            .min_by_key(|(_, a)| a.fuel)
                            .unwrap()
                            .0;
                        let w = spots + weak;
                        if net.path(node, w, 0).time + 2 < remain {
                            w
                        } else {
                            options[self.rng.n(options.len())]
                        }
                    } else {
                        options[self.rng.n(options.len())]
                    };
                    remain -= net.path(node, target, 0).time;
                    let wait = if k + 1 == count {
                        remain
                    } else {
                        self.rng.n((remain / 2 + 1) as usize) as u16
                    };
                    ss.push((target, wait));
                    remain -= wait;
                    node = target;
                }
                if !ss.is_empty() {
                    route.stops = ss;
                }
                return;
            }
            if roll <= 1 || it % 31 == 0 {
                let k = self.rng.n(route.stops.len());
                let from = route.stops[k].0;
                let target = if self.rng.n(3) == 0 {
                    self.rng.n(net.n)
                } else {
                    let mut order = (0..net.n)
                        .map(|s| (net.path(from, s, 0).time, s))
                        .collect::<Vec<_>>();
                    order.sort_unstable();
                    order[self.rng.n(12.min(net.n))].1
                };
                route.stops[k].0 = target;
            } else if roll == 2 && route.stops.len() < 4 {
                let k = self.rng.n(route.stops.len() + 1);
                route
                    .stops
                    .insert(k, (self.rng.n(net.n), (steps / 4).max(2)));
                for stop in &mut route.stops {
                    stop.1 = stop.1.min(steps / 3);
                }
            } else if roll == 3 && route.stops.len() > 1 {
                let k = self.rng.n(route.stops.len());
                route.stops.remove(k);
            } else if roll == 4 {
                let k = self.rng.n(route.stops.len());
                route.stops[k].1 = (self.rng.n(steps as usize + 1)) as u16;
            } else if roll == 5 && route.stops.len() > 1 {
                let k = self.rng.n(route.stops.len());
                let j = self.rng.n(route.stops.len());
                route.stops.swap(k, j);
            } else {
                let agent = agents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.kind == 1)
                    .nth(r)
                    .unwrap()
                    .0;
                route.stops = vec![(
                    if self.rng.n(3) == 0 {
                        spots + agent
                    } else {
                        self.rng.n(net.n)
                    },
                    steps,
                )];
            }
        }
        fn reversed_candidate(
            &mut self,
            previous: &Plan,
            agents: &[Agent],
            net: &Network,
            day: usize,
            steps: u16,
        ) -> Candidate {
            let mut best = None::<Candidate>;
            for preserve_waits in [true, false] {
                let reversed = previous
                    .iter()
                    .map(|route| {
                        route
                            .iter()
                            .rev()
                            .filter_map(|&act| {
                                if act >= 0 {
                                    Some((act + 3) % 6)
                                } else if preserve_waits {
                                    Some(act)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let support = agents
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.kind == 1)
                    .map(|(a, _)| SupportRoute {
                        direct: Some(reversed[a].clone()),
                        stops: Vec::new(),
                    })
                    .collect::<Vec<_>>();
                let supply = Supply::build(&self.p, agents, net, &support, steps);
                let source = Candidate {
                    plan: reversed,
                    score: 0.0,
                    masks: vec![0; agents.len()],
                    end: agents.to_vec(),
                    support: support.clone(),
                };
                let candidate = self
                    .preserve_patrols(&source, agents, net, &supply, support, day, steps)
                    .unwrap();
                if best.as_ref().is_none_or(|c| candidate.score > c.score) {
                    best = Some(candidate);
                }
            }
            best.unwrap()
        }
        fn preserve_patrols(
            &mut self,
            source: &Candidate,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            support: Vec<SupportRoute>,
            day: usize,
            steps: u16,
        ) -> Option<Candidate> {
            let mut c = source.clone();
            c.support = support;
            let mut broken = Vec::new();
            for (a, ag) in agents.iter().enumerate() {
                if ag.kind == 1 {
                    c.plan[a] = supply.plans[a].clone();
                    c.end[a].pos = supply.end[a];
                } else if let Some(route) =
                    window::retime_route(&self.p, net, supply, ag, &c.plan[a], steps)
                {
                    c.plan[a] = route.plan;
                    c.masks[a] = route.mask;
                    c.end[a].pos = route.pos;
                    c.end[a].fuel = route.fuel as u32;
                } else {
                    c.masks[a] = 0;
                    broken.push(a);
                }
            }
            if self.rng.n(2) == 0 {
                broken.reverse();
            }
            let mut partners = broken.iter().fold(0u16, |mask, &a| mask | (1 << a));
            for a in broken {
                partners &= !(1 << a);
                let mut used = vec![0u8; self.p.spots.len()];
                let mut covered = 0u64;
                for (b, &mask) in c.masks.iter().enumerate() {
                    if a == b {
                        continue;
                    }
                    for (s, spot) in self.p.spots.iter().enumerate() {
                        if mask >> s & 1 != 0 {
                            used[s] += 1;
                            covered |= 1 << spot.brand;
                        }
                    }
                }
                let it = self.rng.n(1_000_000);
                let route = if it % 2 == 0 {
                    self.cell_route(
                        a, agents, net, supply, day, steps, &used, covered, 200, it, partners,
                    )
                } else {
                    self.car_route(
                        a, agents, net, supply, day, steps, &used, covered, 45, it, partners,
                    )
                };
                c.plan[a] = route.plan;
                c.masks[a] = route.mask;
                c.end[a].pos = route.pos;
                c.end[a].fuel = route.fuel as u32;
            }
            c.score = self.local_value(&c.masks, &c.end, net, supply, day, steps);
            Some(c)
        }
        fn polish_service(
            &mut self,
            candidate: &mut Candidate,
            source: &Candidate,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            day: usize,
            steps: u16,
        ) {
            let waiting = |plan: &[i64]| {
                // Padding after the last move is free time, not a refill delay.
                let end = plan.iter().rposition(|&a| a >= 0).map_or(0, |i| i + 1);
                plan[..end]
                    .iter()
                    .filter(|&&a| a < 0)
                    .map(|&a| -a)
                    .sum::<i64>()
            };
            let mut priorities = agents
                .iter()
                .enumerate()
                .filter(|(_, ag)| ag.kind == 0)
                .map(|(a, _)| {
                    let fuel = candidate.end[a].fuel as i64 - source.end[a].fuel as i64;
                    let freed = waiting(&source.plan[a]) - waiting(&candidate.plan[a]);
                    (fuel * 2 + freed, a)
                })
                .filter(|&(gain, _)| gain > 3)
                .collect::<Vec<_>>();
            priorities.sort_unstable_by(|a, b| b.cmp(a));
            for &(_, a) in priorities.iter().take(2) {
                let mut used = vec![0u8; self.p.spots.len()];
                let mut covered = 0u64;
                for (b, &mask) in candidate.masks.iter().enumerate() {
                    if a == b {
                        continue;
                    }
                    for (s, spot) in self.p.spots.iter().enumerate() {
                        if mask >> s & 1 != 0 {
                            used[s] += 1;
                            covered |= 1 << spot.brand;
                        }
                    }
                }
                let iteration = self.rng.n(1_000_000);
                let route = self.cell_route(
                    a, agents, net, supply, day, steps, &used, covered, 300, iteration, 0,
                );
                let mut improved = candidate.clone();
                improved.plan[a] = route.plan;
                improved.masks[a] = route.mask;
                improved.end[a].pos = route.pos;
                improved.end[a].fuel = route.fuel as u32;
                improved.score =
                    self.local_value(&improved.masks, &improved.end, net, supply, day, steps);
                if improved.score > candidate.score {
                    *candidate = improved;
                }
            }
        }
        fn build_candidate(
            &mut self,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            support: Vec<SupportRoute>,
            day: usize,
            steps: u16,
            _it: usize,
            width: usize,
        ) -> Candidate {
            // Strategy variation is independent of the caller's operator schedule.
            let it = self.rng.n(1_000_000);
            let mut plan = supply.plans.clone();
            let mut masks = vec![0; agents.len()];
            let mut end = agents.to_vec();
            for a in 0..agents.len() {
                if agents[a].kind == 1 {
                    end[a].pos = supply.end[a];
                }
            }
            let mut order = (0..agents.len())
                .filter(|&a| agents[a].kind == 0)
                .collect::<Vec<_>>();
            if it % 4 == 0 {
                order.sort_by_key(|&a| agents[a].fuel);
            } else {
                for i in 1..order.len() {
                    let j = self.rng.n(i + 1);
                    order.swap(i, j);
                }
            }
            let mut used = vec![0u8; self.p.spots.len()];
            let mut daybrands = 0u64;
            let mut partners = order.iter().fold(0u16, |mask, &a| mask | (1 << a));
            for &a in &order {
                partners &= !(1 << a);
                let route = if it % 2 == 0 {
                    self.car_route(
                        a, agents, net, supply, day, steps, &used, daybrands, width, it, partners,
                    )
                } else {
                    self.cell_route(
                        a,
                        agents,
                        net,
                        supply,
                        day,
                        steps,
                        &used,
                        daybrands,
                        width * 5,
                        it,
                        partners,
                    )
                };
                plan[a] = route.plan;
                masks[a] = route.mask;
                end[a].pos = route.pos;
                end[a].fuel = route.fuel as u32;
                daybrands |= route.brands;
                for (s, u) in used.iter_mut().enumerate() {
                    if route.mask >> s & 1 != 0 {
                        *u += 1;
                    }
                }
            }
            let mut value = self.local_value(&masks, &end, net, supply, day, steps);
            for &a in order.iter().rev().take(agents.len()) {
                let mut uu = vec![0u8; self.p.spots.len()];
                let mut bb = 0u64;
                for b in 0..agents.len() {
                    if b == a {
                        continue;
                    }
                    for (s, spot) in self.p.spots.iter().enumerate() {
                        if masks[b] >> s & 1 != 0 {
                            uu[s] += 1;
                            bb |= 1 << spot.brand;
                        }
                    }
                }
                let route = self.cell_route(
                    a,
                    agents,
                    net,
                    supply,
                    day,
                    steps,
                    &uu,
                    bb,
                    width * 5,
                    it,
                    0,
                );
                let oldmask = masks[a];
                let oldagent = end[a].clone();
                masks[a] = route.mask;
                end[a].pos = route.pos;
                end[a].fuel = route.fuel as u32;
                let nv = self.local_value(&masks, &end, net, supply, day, steps);
                if nv > value {
                    value = nv;
                    plan[a] = route.plan;
                } else {
                    masks[a] = oldmask;
                    end[a] = oldagent;
                }
            }
            Candidate {
                plan,
                score: value,
                masks,
                end,
                support,
            }
        }
        fn rebuild_subset(
            &mut self,
            c: &mut Candidate,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            day: usize,
            steps: u16,
            _it: usize,
            width: usize,
        ) {
            // Strategy variation is independent of the caller's operator schedule.
            let it = self.rng.n(1_000_000);
            let mut cars = (0..agents.len())
                .filter(|&a| agents[a].kind == 0)
                .collect::<Vec<_>>();
            for k in 1..cars.len() {
                let j = self.rng.n(k + 1);
                cars.swap(k, j);
            }
            let count = match self.rng.n(10) {
                0 | 1 => cars.len(),
                2 => cars.len().saturating_sub(1),
                3 | 4 | 5 => 3,
                _ => 2,
            };
            cars.truncate(count.min(cars.len()).max(1));
            let width = if cars.len() >= 4 {
                width * 2 / 3
            } else {
                width
            };
            let mut used = vec![0u8; self.p.spots.len()];
            let mut bb = 0u64;
            for a in 0..agents.len() {
                if cars.contains(&a) {
                    continue;
                }
                for (s, spot) in self.p.spots.iter().enumerate() {
                    if c.masks[a] >> s & 1 != 0 {
                        used[s] += 1;
                        bb |= 1 << spot.brand;
                    }
                }
            }
            let mut partners = cars.iter().fold(0u16, |mask, &a| mask | (1 << a));
            for &a in &cars {
                partners &= !(1 << a);
                let route = if it % 3 == 0 {
                    self.car_route(
                        a, agents, net, supply, day, steps, &used, bb, width, it, partners,
                    )
                } else {
                    self.cell_route(
                        a,
                        agents,
                        net,
                        supply,
                        day,
                        steps,
                        &used,
                        bb,
                        width * 5,
                        it,
                        partners,
                    )
                };
                c.plan[a] = route.plan;
                c.masks[a] = route.mask;
                c.end[a].pos = route.pos;
                c.end[a].fuel = route.fuel as u32;
                bb |= route.brands;
                for (s, u) in used.iter_mut().enumerate() {
                    if route.mask >> s & 1 != 0 {
                        *u += 1;
                    }
                }
            }
            c.score = self.local_value(&c.masks, &c.end, net, supply, day, steps);
        }
        fn local_value(
            &self,
            masks: &[u32],
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            day: usize,
            steps: u16,
        ) -> f64 {
            let (bb, udon) = self.mask_statistics(masks);
            let mut value = 0.0;
            for a in 0..agents.len() {
                if agents[a].kind == 0 {
                    value += self.terminal_pos(
                        agents[a].pos,
                        steps,
                        agents[a].fuel as u16,
                        net,
                        supply,
                        steps,
                        day + 1 == self.p.day_steps.len(),
                    );
                }
            }
            if day + 1 < self.p.day_steps.len() {
                let mut best = [f64::INFINITY; 64];
                let mut closest_time = [f64::INFINITY; 64];
                for ag in agents.iter().filter(|a| a.kind == 0) {
                    let delivery = agents
                        .iter()
                        .filter(|a| a.kind == 1)
                        .map(|sup| {
                            let y1 = sup.pos / self.p.map.width;
                            let y2 = ag.pos / self.p.map.width;
                            let x1 = sup.pos % self.p.map.width;
                            let x2 = ag.pos % self.p.map.width;
                            (x1.abs_diff(x2) + y1.abs_diff(y2)) as f64
                        })
                        .fold(f64::INFINITY, f64::min);
                    for (s, spot) in self.p.spots.iter().enumerate() {
                        let mut dist = net.dist_t[s][ag.pos] as f64;
                        closest_time[spot.brand] = closest_time[spot.brand].min(dist);
                        let shortage = (net.dist_f[s][ag.pos] as u32).saturating_sub(ag.fuel);
                        if shortage > 0 {
                            dist += shortage as f64 * 1.5 + delivery * 0.35;
                        }
                        best[spot.brand] = best[spot.brand].min(dist);
                    }
                }
                for distance in best.into_iter().filter(|d| d.is_finite()) {
                    value -= distance * 0.2;
                }
                if agents.len() <= 4 {
                    let next_steps = self.p.day_steps[day + 1] as f64;
                    for distance in closest_time.into_iter().filter(|d| d.is_finite()) {
                        value -= 400.0 * (distance / next_steps - 0.45).max(0.0).powi(2);
                    }
                }
            }
            value
                + bb.count_ones() as f64 * 1000.0
                + (bb & !self.seen).count_ones() as f64 * self.fresh_bonus(day)
                + udon as f64 * 10.0
        }
        fn cell_route(
            &mut self,
            a: usize,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            day: usize,
            steps: u16,
            used: &[u8],
            daybrands: u64,
            width: usize,
            it: usize,
            partners: u16,
        ) -> CarRoute {
            let width = width * self.beam_scale;
            #[derive(Clone)]
            struct State {
                pos: usize,
                fuel: u16,
                mask: u32,
                brands: u64,
                gain: f64,
                pending: f64,
                rank: f64,
                parent: usize,
                action: i64,
                t: u16,
            }
            let p = &self.p;
            let limit = p.fuel_limits as u16;
            let last = day + 1 == p.day_steps.len();
            let mut weights = [1.0f64; 64];
            if it % 4 != 0 {
                let pending_brands = p
                    .spots
                    .iter()
                    .fold(0u64, |mask, s| mask | (1u64 << s.brand))
                    & !daybrands;
                for brand in 0..64 {
                    if pending_brands >> brand & 1 == 0 {
                        continue;
                    }
                    let own = (0..p.spots.len())
                        .filter(|&s| p.spots[s].brand == brand)
                        .map(|s| {
                            net.dist_t[s][agents[a].pos] as u32
                                + (net.dist_f[s][agents[a].pos] as u32)
                                    .saturating_sub(agents[a].fuel)
                                    * 3
                        })
                        .min()
                        .unwrap_or(0) as f64;
                    let other = agents
                        .iter()
                        .enumerate()
                        .filter(|&(b, ag)| b != a && ag.kind == 0 && partners & (1 << b) != 0)
                        .flat_map(|(_, ag)| {
                            (0..p.spots.len())
                                .filter(move |&s| p.spots[s].brand == brand)
                                .map(move |s| {
                                    net.dist_t[s][ag.pos] as u32
                                        + (net.dist_f[s][ag.pos] as u32).saturating_sub(ag.fuel) * 3
                                })
                        })
                        .min()
                        .unwrap_or(own as u32) as f64;
                    weights[brand] = 0.05 + 1.9 / (1.0 + ((own - other) / 6.0).exp());
                }
            }
            if it % 7 != 0 {
                for weight in &mut weights {
                    *weight *= 0.6 + self.rng.f() * 0.8;
                }
            }
            let novel = p
                .spots
                .iter()
                .enumerate()
                .filter(|(s, spot)| {
                    (used[*s] as u32) < spot.stocks
                        && daybrands & (1u64 << spot.brand) == 0
                        && weights[spot.brand] > 0.0
                })
                .map(|(s, _)| s)
                .collect::<Vec<_>>();
            let primary = if novel.is_empty() {
                Vec::new()
            } else {
                (0..p.map.len())
                    .map(|pos| {
                        let mut order = novel
                            .iter()
                            .map(|&s| {
                                (
                                    weights[p.spots[s].brand] / (net.dist_t[s][pos] as f64 + 3.0),
                                    s,
                                )
                            })
                            .collect::<Vec<_>>();
                        order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
                        order.into_iter().map(|(_, s)| s).collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            let mut rewards = [0.0f64; 64];
            for &s in &novel {
                let brand = p.spots[s].brand;
                rewards[brand] = weights[brand]
                    * (1000.0
                        + if self.seen & (1u64 << brand) == 0 {
                            self.fresh_bonus(day)
                        } else {
                            0.0
                        });
            }
            let remaining_reward = rewards.iter().sum::<f64>();
            let mut buckets = vec![Vec::new(); steps as usize + 1];
            let mut arena = Vec::<State>::new();
            buckets[0].push(State {
                pos: agents[a].pos,
                fuel: agents[a].fuel as u16,
                mask: 0,
                brands: 0,
                gain: 0.0,
                pending: remaining_reward,
                rank: 0.0,
                parent: usize::MAX,
                action: 0,
                t: 0,
            });
            let mut best_id = 0;
            let mut best_value = f64::NEG_INFINITY;
            for t in 0..=steps {
                let states = std::mem::take(&mut buckets[t as usize]);
                // At one time, position and visit set, more fuel dominates less.
                let mut frontier = SearchMap::<(usize, u32), State>::default();
                for b in states {
                    match frontier.entry((b.pos, b.mask)) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(b);
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            if b.fuel > entry.get().fuel {
                                entry.insert(b);
                            }
                        }
                    }
                }
                let mut states = frontier.into_values().collect::<Vec<_>>();
                states.sort_unstable_by(|x, y| {
                    y.rank
                        .total_cmp(&x.rank)
                        .then(y.fuel.cmp(&x.fuel))
                        .then(x.mask.cmp(&y.mask))
                        .then(x.pos.cmp(&y.pos))
                });
                for b in states.into_iter().take(width) {
                    let id = arena.len();
                    arena.push(b.clone());
                    let value =
                        b.gain + self.terminal_pos(b.pos, t, b.fuel, net, supply, steps, last);
                    if value > best_value
                        && t > 0
                        && (net.spot_at[b.pos] >= 0 || supply.at[steps as usize][b.pos] || last)
                    {
                        best_value = value;
                        best_id = id;
                    }
                    if t == steps {
                        continue;
                    }
                    for action in -1i64..6 {
                        let (pos, dt, cost) = if action < 0 {
                            (b.pos, 1u16, 0u16)
                        } else {
                            let Some(pos) = p.map.neighbor(b.pos, action as usize) else {
                                continue;
                            };
                            if p.map.terrain(pos) == 3 {
                                continue;
                            }
                            (pos, net.time[b.pos] as u16, net.cost[b.pos] as u16)
                        };
                        let tt = t + dt;
                        if tt > steps {
                            continue;
                        }
                        let mut fuel = b.fuel;
                        if dt > 1 {
                            fuel = supply.wait_fuel(b.pos, t, tt - 1, fuel, limit);
                        }
                        if fuel < cost {
                            continue;
                        }
                        fuel -= cost;
                        if supply.at[tt as usize][pos] {
                            fuel = limit;
                        }
                        let mut q = State {
                            pos,
                            fuel,
                            mask: b.mask,
                            brands: b.brands,
                            gain: b.gain,
                            pending: b.pending,
                            rank: 0.0,
                            parent: id,
                            action,
                            t: tt,
                        };
                        for at in [if dt > 1 { b.pos } else { pos }, pos] {
                            let s = net.spot_at[at];
                            if s < 0 {
                                continue;
                            }
                            let s = s as usize;
                            if q.mask >> s & 1 != 0 {
                                continue;
                            }
                            q.mask |= 1 << s;
                            let brand = 1 << p.spots[s].brand;
                            if used[s] as u32 >= p.spots[s].stocks {
                                q.brands |= brand;
                                continue;
                            }
                            q.gain += 10.0;
                            if q.brands & brand == 0 && daybrands & brand == 0 {
                                q.gain += rewards[p.spots[s].brand];
                                q.pending = (q.pending - rewards[p.spots[s].brand]).max(0.0);
                            }
                            q.brands |= brand;
                        }
                        let mut potential = 0.0f64;
                        let mut fresh = false;
                        for &s in &net.near[pos] {
                            let d = net.dist_t[s][pos];
                            if d > steps - tt {
                                break;
                            }
                            if q.mask >> s & 1 != 0 || used[s] as u32 >= p.spots[s].stocks {
                                continue;
                            }
                            potential = 30.0 / (d as f64 + 3.0);
                            break;
                        }
                        if !primary.is_empty() {
                            for &s in &primary[pos] {
                                if q.brands & (1u64 << p.spots[s].brand) != 0
                                    || q.mask >> s & 1 != 0
                                {
                                    continue;
                                }
                                let d = net.dist_t[s][pos];
                                if d > steps - tt {
                                    continue;
                                }
                                fresh = true;
                                potential = potential
                                    .max(3000.0 * weights[p.spots[s].brand] / (d as f64 + 3.0));
                                break;
                            }
                        }
                        let mobility = ((steps - tt) as f64).min(fuel as f64 * 1.25);
                        let future = if fresh {
                            (potential + mobility * 30.0).min(q.pending * 0.95)
                        } else {
                            potential + mobility * 0.7
                        };
                        q.rank = q.gain
                            + future
                            + self.terminal_pos(pos, tt, fuel, net, supply, steps, last) * 0.12;
                        buckets[tt as usize].push(q);
                    }
                }
            }
            let rebuild = |best_id: usize| {
                let best = &arena[best_id];
                let mut plan = Vec::new();
                let mut id = best_id;
                while arena[id].parent != usize::MAX {
                    plan.push(arena[id].action);
                    id = arena[id].parent;
                }
                plan.reverse();
                let mut compact = Vec::new();
                for action in plan {
                    if action < 0 && compact.last().is_some_and(|x| *x < 0) {
                        *compact.last_mut().unwrap() -= 1;
                    } else {
                        compact.push(action);
                    }
                }
                if best.t < steps {
                    compact.push(-((steps - best.t) as i64));
                }
                let fuel = supply.wait_fuel(best.pos, best.t, steps, best.fuel, limit);
                CarRoute {
                    plan: compact,
                    mask: best.mask,
                    brands: best.brands,
                    pos: best.pos,
                    fuel,
                }
            };
            if self.captured_routes.is_some() {
                let mut endpoints = SearchMap::<(u32, usize), (f64, usize)>::default();
                for (id, state) in arena.iter().enumerate() {
                    if state.t == 0
                        || !(net.spot_at[state.pos] >= 0
                            || supply.at[steps as usize][state.pos]
                            || last)
                    {
                        continue;
                    }
                    let value = state.mask.count_ones() as f64 * 10.0
                        + self
                            .terminal_pos(state.pos, state.t, state.fuel, net, supply, steps, last);
                    let entry = endpoints
                        .entry((state.mask, state.pos))
                        .or_insert((f64::NEG_INFINITY, id));
                    if value > entry.0 {
                        *entry = (value, id);
                    }
                }
                let mut masks = SearchMap::<u32, Vec<(f64, usize)>>::default();
                for ((mask, _), value) in endpoints {
                    masks.entry(mask).or_default().push(value);
                }
                let mut groups = vec![Vec::<(f64, usize)>::new(); 65];
                for options in masks.values_mut() {
                    options.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                    options.truncate(2);
                    let (value, id) = options[0];
                    groups[arena[id].brands.count_ones() as usize].push((value, id));
                }
                let mut ids = vec![best_id];
                for group in &mut groups {
                    group.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                    let mut chosen = Vec::new();
                    for &(_, id) in group.iter() {
                        if chosen.iter().all(|&other: &usize| {
                            (arena[other].mask ^ arena[id].mask).count_ones() >= 2
                        }) {
                            chosen.push(id);
                            if chosen.len() == 6 {
                                break;
                            }
                        }
                    }
                    // Preserve the option to assign each brand to this car or to
                    // another car, even when its individually best routes agree.
                    let available = group.iter().fold(0u64, |m, &(_, id)| m | arena[id].brands);
                    let unavoidable = group
                        .iter()
                        .fold(u64::MAX, |m, &(_, id)| m & arena[id].brands);
                    let mut present = chosen.iter().fold(0u64, |m, &id| m | arena[id].brands);
                    let mut common = chosen.iter().fold(u64::MAX, |m, &id| m & arena[id].brands);
                    for &(_, id) in group.iter() {
                        if (arena[id].brands & available & !present) != 0
                            || (!arena[id].brands & common & !unavoidable) != 0
                        {
                            chosen.push(id);
                            present |= arena[id].brands;
                            common &= arena[id].brands;
                        }
                    }
                    for id in chosen {
                        ids.extend(masks[&arena[id].mask].iter().map(|&(_, id)| id));
                    }
                }
                self.captured_routes = Some(ids.into_iter().map(rebuild).collect());
            }
            rebuild(best_id)
        }

        fn car_route(
            &mut self,
            a: usize,
            agents: &[Agent],
            net: &Network,
            supply: &Supply,
            day: usize,
            steps: u16,
            used: &[u8],
            daybrands: u64,
            width: usize,
            it: usize,
            partners: u16,
        ) -> CarRoute {
            let width = width * self.beam_scale;
            let p = &self.p;
            let limit = p.fuel_limits as u16;
            let node = p.spots.len() + a;
            let mut weights = vec![1.0; 64];
            if it % 4 != 0 {
                let pending_brands = p
                    .spots
                    .iter()
                    .fold(0u64, |mask, s| mask | (1u64 << s.brand))
                    & !daybrands;
                for brand in 0..64 {
                    if pending_brands >> brand & 1 == 0 {
                        continue;
                    }
                    let own = (0..p.spots.len())
                        .filter(|&s| p.spots[s].brand == brand)
                        .map(|s| {
                            net.dist_t[s][agents[a].pos] as u32
                                + (net.dist_f[s][agents[a].pos] as u32)
                                    .saturating_sub(agents[a].fuel)
                                    * 3
                        })
                        .min()
                        .unwrap_or(0) as f64;
                    let other = (0..agents.len())
                        .filter(|&b| b != a && agents[b].kind == 0 && partners & (1 << b) != 0)
                        .flat_map(|b| {
                            (0..p.spots.len())
                                .filter(move |&s| p.spots[s].brand == brand)
                                .map(move |s| {
                                    net.dist_t[s][agents[b].pos] as u32
                                        + (net.dist_f[s][agents[b].pos] as u32)
                                            .saturating_sub(agents[b].fuel)
                                            * 3
                                })
                        })
                        .min()
                        .unwrap_or(own as u32) as f64;
                    weights[brand] =
                        0.05 + 1.9 / (1.0 + ((own - other) / (3.0 + (it % 4) as f64 * 2.0)).exp());
                }
            }
            if it % 7 != 0 {
                for weight in &mut weights {
                    *weight *= 0.6 + self.rng.f() * 0.8;
                }
            }
            let mut first = Beam {
                node,
                t: 0,
                fuel: agents[a].fuel as u16,
                mask: 0,
                brands: 0,
                gain: 0.0,
                rank: 0.0,
                route: Vec::new(),
            };
            if net.spot_at[agents[a].pos] >= 0 {
                let s = net.spot_at[agents[a].pos] as usize;
                self.pickup(&mut first, s, used, daybrands, &weights, day);
            }
            let mut beam = vec![first.clone()];
            let mut best = first;
            let mut best_value = f64::NEG_INFINITY;
            let last = day + 1 == p.day_steps.len();
            let novelty = if it % 7 == 1 { 1.25 } else { 1.0 };
            for _depth in 0..(p.spots.len() + 6) {
                let mut next = Vec::new();
                for b in &beam {
                    let terminal = self.terminal(b, net, supply, steps, last);
                    let value = b.gain + terminal;
                    if value > best_value {
                        best_value = value;
                        best = b.clone();
                    }
                    if b.t >= steps {
                        continue;
                    }
                    for &dest in &net.targets {
                        let is_spot = dest < p.spots.len();
                        let service = supply.nodes.contains(&dest);
                        if dest == b.node {
                            continue;
                        }
                        if (!is_spot
                            || b.mask >> dest & 1 != 0
                            || used[dest] as u32 >= p.spots[dest].stocks)
                            && !service
                        {
                            continue;
                        }
                        for mode in 0..MODES {
                            let path = net.path(b.node, dest, mode);
                            if b.t + path.time > steps {
                                continue;
                            }
                            if mode > 0
                                && path.time == net.path(b.node, dest, mode - 1).time
                                && path.fuel == net.path(b.node, dest, mode - 1).fuel
                            {
                                continue;
                            }
                            let mut delays = vec![0u16];
                            if service {
                                for &(n, arr, dep) in &supply.windows {
                                    if n == dest
                                        && arr > b.t + path.time
                                        && arr <= steps
                                        && dep >= arr
                                    {
                                        delays.push(arr - b.t - path.time);
                                    }
                                }
                            }
                            for delay in delays {
                                if b.t + delay + path.time > steps {
                                    continue;
                                }
                                let mut q = b.clone();
                                let mut pos = net.pos[b.node];
                                q.fuel = supply.wait_fuel(pos, q.t, q.t + delay, q.fuel, limit);
                                q.t += delay;
                                let mut okay = true;
                                for &(to, _, dt, cost) in &path.moves {
                                    let arrival = q.t + dt as u16;
                                    if dt > 1 {
                                        q.fuel =
                                            supply.wait_fuel(pos, q.t, arrival - 1, q.fuel, limit);
                                    }
                                    if q.fuel < cost as u16 {
                                        okay = false;
                                        break;
                                    }
                                    q.fuel -= cost as u16;
                                    q.t = arrival;
                                    pos = to as usize;
                                    if supply.at[q.t as usize][pos] {
                                        q.fuel = limit;
                                    }
                                    if net.spot_at[pos] >= 0 {
                                        self.pickup(
                                            &mut q,
                                            net.spot_at[pos] as usize,
                                            used,
                                            daybrands,
                                            &weights,
                                            day,
                                        );
                                    }
                                }
                                if !okay {
                                    continue;
                                }
                                let direct = if service
                                    && q.fuel < limit
                                    && (q.mask != b.mask || q.fuel > b.fuel)
                                {
                                    Some(q.clone())
                                } else {
                                    None
                                };
                                let arrival_time = q.t;
                                if service && q.fuel < limit {
                                    let end = supply
                                        .windows
                                        .iter()
                                        .filter(|&&(n, arr, dep)| {
                                            n == dest && dep > q.t && arr <= steps
                                        })
                                        .map(|&(_, arr, _)| arr.max(q.t + 1))
                                        .min();
                                    if let Some(end) = end {
                                        if end <= steps {
                                            q.fuel = supply.wait_fuel(pos, q.t, end, q.fuel, limit);
                                            q.t = end;
                                        }
                                    }
                                }
                                if q.t > arrival_time {
                                    if let Some(mut direct) = direct {
                                        direct.node = dest;
                                        direct.route.push((dest, mode as u8, delay));
                                        next.push(direct);
                                    }
                                }
                                if q.mask == b.mask && q.fuel <= b.fuel {
                                    continue;
                                }
                                // Save time spent before and after the arc as one encoded segment.
                                q.node = dest;
                                let extra = q.t - b.t - delay - path.time;
                                q.route.push((
                                    dest,
                                    mode as u8 | if extra > 0 { 4 } else { 0 },
                                    delay,
                                ));
                                if extra > 0 {
                                    q.route.push((dest, 3, extra));
                                }
                                next.push(q);
                            }
                        }
                    }
                }
                if next.is_empty() {
                    break;
                }
                for b in &mut next {
                    b.rank = b.gain * novelty
                        + self.terminal(b, net, supply, steps, last)
                        + (steps - b.t) as f64 * 1.15;
                }
                let mut frontier = SearchMap::<(u32, usize, u16), Beam>::default();
                for b in next {
                    match frontier.entry((b.mask, b.node, b.t)) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(b);
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            if b.fuel > entry.get().fuel {
                                entry.insert(b);
                            }
                        }
                    }
                }
                let mut next = frontier.into_values().collect::<Vec<_>>();
                next.sort_unstable_by(|x, y| {
                    y.rank
                        .total_cmp(&x.rank)
                        .then(y.fuel.cmp(&x.fuel))
                        .then(x.t.cmp(&y.t))
                        .then(x.mask.cmp(&y.mask))
                        .then(x.node.cmp(&y.node))
                });
                beam = next.into_iter().take(width).collect();
            }
            let mut plan = Vec::new();
            let mut from = node;
            let mut time = 0u16;
            for &(dest, mode, wait) in &best.route {
                if wait > 0 {
                    plan.push(-(wait as i64));
                    time += wait;
                }
                if mode != 3 {
                    let path = net.path(from, dest, (mode & 3) as usize);
                    plan.extend(path.moves.iter().map(|&(_, dir, _, _)| dir as i64));
                    time += path.time;
                }
                from = dest;
            }
            if time < steps {
                plan.push(-((steps - time) as i64));
            }
            let fuel = supply.wait_fuel(net.pos[best.node], best.t, steps, best.fuel, limit);
            CarRoute {
                plan,
                mask: best.mask,
                brands: best.brands,
                pos: net.pos[best.node],
                fuel,
            }
        }
        fn pickup(
            &self,
            b: &mut Beam,
            s: usize,
            used: &[u8],
            daybrands: u64,
            weights: &[f64],
            day: usize,
        ) {
            if b.mask >> s & 1 != 0 {
                return;
            }
            let brand = 1u64 << self.p.spots[s].brand;
            b.mask |= 1u32 << s;
            if used[s] as u32 >= self.p.spots[s].stocks {
                b.brands |= brand;
                return;
            }
            b.gain += 10.0;
            if b.brands & brand == 0 && daybrands & brand == 0 {
                b.gain += 1000.0 * weights[self.p.spots[s].brand];
                if self.seen & brand == 0 {
                    b.gain += self.fresh_bonus(day) * weights[self.p.spots[s].brand];
                }
            }
            b.brands |= brand;
        }
        fn terminal(
            &self,
            b: &Beam,
            net: &Network,
            supply: &Supply,
            steps: u16,
            last: bool,
        ) -> f64 {
            self.terminal_pos(net.pos[b.node], b.t, b.fuel, net, supply, steps, last)
        }
        fn terminal_pos(
            &self,
            pos: usize,
            t: u16,
            fuel: u16,
            net: &Network,
            supply: &Supply,
            steps: u16,
            last: bool,
        ) -> f64 {
            endpoint_value(&self.p, net, pos, t, fuel, supply, steps, last)
        }
    }

    fn endpoint_value(
        p: &Problem,
        net: &Network,
        pos: usize,
        t: u16,
        fuel: u16,
        supply: &Supply,
        steps: u16,
        last: bool,
    ) -> f64 {
        if last {
            return 0.0;
        }
        let fuel = supply.wait_fuel(pos, t, steps, fuel, p.fuel_limits as u16);
        let escape = net.escape_fuel[pos];
        let stranded = if escape > 0 {
            (1.0 - fuel as f64 / escape as f64).max(0.0) * 60.0
        } else {
            0.0
        };
        let first_shop = net.near[pos].first().map_or(0, |&s| net.dist_t[s][pos]);
        let needed = supply.end_f[pos];
        let near = supply.end_t[pos];
        let mobility = (fuel as f64 / (needed as f64 + 1.0)).min(0.5);
        let meeting_time = near as f64 * (1.0 - mobility);
        let urgency = (1.0 - fuel as f64 / (p.fuel_limits as f64 * 0.70)).clamp(0.0, 1.0);
        -stranded
            - first_shop as f64 * 0.6
            - meeting_time * urgency * 1.25
            - if p.agents.len() >= 6 {
                (needed.saturating_sub(fuel) as f64 * 4.0).min(120.0) * urgency
            } else {
                0.0
            }
            + (fuel as f64).min((p.fuel_limits as f64 * 0.5).min(30.0)) * 0.5
            + 0.5 * fuel as f64 / p.fuel_limits as f64
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn shared_stock_layers_match_storewise_capping() {
            let mut p = fixture();
            p.spots = vec![
                tester::model::Spot {
                    brand: 0,
                    pos: 1,
                    stocks: 1,
                },
                tester::model::Spot {
                    brand: 0,
                    pos: 2,
                    stocks: 2,
                },
                tester::model::Spot {
                    brand: 1,
                    pos: 3,
                    stocks: 3,
                },
            ];
            let solver = Solver::new(p, 0);
            for a in 0u32..8 {
                for b in 0u32..8 {
                    for c in 0u32..8 {
                        let masks = [a, b, c];
                        let (brands, food) = solver.mask_statistics(&masks);
                        let mut expected_brands = 0u64;
                        let mut expected_food = 0;
                        for (s, spot) in solver.p.spots.iter().enumerate() {
                            let visits = masks.iter().filter(|&&m| m >> s & 1 != 0).count() as u32;
                            expected_food += visits.min(spot.stocks);
                            if visits > 0 {
                                expected_brands |= 1 << spot.brand;
                            }
                        }
                        assert_eq!((brands, food), (expected_brands, expected_food));
                    }
                }
            }
        }
        fn fixture() -> Problem {
            Problem {
                starts_at: 0,
                day_seconds: vec![60],
                day_steps: vec![8],
                map: tester::model::Map {
                    width: 4,
                    height: 2,
                    cells: vec![vec![1, 0, 0, 0], vec![0, 0, 0, 0]],
                },
                spots: vec![tester::model::Spot {
                    brand: 0,
                    pos: 1,
                    stocks: 1,
                }],
                agents: vec![4, 7],
                fuel_limits: 6,
                players: 1,
                busy_threshold: 2,
                jammed_threshold: 4,
            }
        }
        #[test]
        fn moving_from_road_requires_refill_before_completion() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 0,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let supply = Supply::build(
                &p,
                &agents,
                &net,
                &[SupportRoute {
                    direct: None,
                    stops: vec![(2, 8)],
                }],
                8,
            );
            let mut solver = Solver::new(p.clone(), 0);
            let route = solver.cell_route(0, &agents, &net, &supply, 0, 8, &[0], 0, 100, 0, 0);
            assert!(
                route.plan[0] < 0,
                "a one-step road departure cannot use a refill at its completion"
            );
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            let mut plan = supply.plans;
            plan[0] = route.plan;
            game.play_day(plan).unwrap();
            assert_eq!(game.score.udon, 1);
            assert_eq!(route.mask, 1);
            assert_eq!(game.agents[0].fuel, route.fuel as u32);
        }
        #[test]
        fn visits_remain_recorded_when_another_car_reserved_stock() {
            let solver = Solver::new(fixture(), 0);
            let mut b = Beam {
                node: 0,
                t: 1,
                fuel: 6,
                mask: 0,
                brands: 0,
                gain: 0.0,
                rank: 0.0,
                route: Vec::new(),
            };
            solver.pickup(&mut b, 0, &[1], 1, &[1.0], 0);
            assert_eq!(b.mask, 1);
            assert_eq!(b.brands, 1);
            assert_eq!(b.gain, 0.0);
            let c = Candidate {
                plan: Vec::new(),
                score: 0.0,
                masks: vec![b.mask],
                end: Vec::new(),
                support: Vec::new(),
            };
            assert_eq!(solver.statistics(&c), (1, 1));
        }
        #[test]
        fn macro_search_can_collect_on_the_final_step() {
            let mut p = fixture();
            p.day_steps = vec![1];
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 6,
                },
                Agent {
                    kind: 1,
                    pos: 7,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let supply = Supply::build(
                &p,
                &agents,
                &net,
                &[SupportRoute {
                    direct: None,
                    stops: vec![(2, 1)],
                }],
                1,
            );
            let mut solver = Solver::new(p.clone(), 0);
            let route = solver.car_route(0, &agents, &net, &supply, 0, 1, &[0], 0, 20, 0, 0);
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            let mut plan = supply.plans;
            plan[0] = route.plan;
            game.play_day(plan).unwrap();
            assert_eq!(game.score.udon, 1);
        }
        #[test]
        fn window_replacement_preserves_timing_and_suffix_fuel() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 6,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: None,
                stops: vec![(2, 8)],
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 8);
            let mut c = Candidate {
                plan: vec![vec![-8], vec![-8]],
                score: 0.0,
                masks: vec![0, 0],
                end: agents.clone(),
                support,
            };
            let mut rng = Rng(19);
            for _ in 0..30 {
                window::improve(
                    &p, &net, &supply, &agents, &mut c, &mut rng, 8, 0, 100000.0, true,
                );
                let mut game = Game::new(p.clone(), &[0, 1]);
                game.agents = agents.clone();
                game.play_day(c.plan.clone()).unwrap();
                assert_eq!(game.agents[0], c.end[0]);
                assert_eq!(game.score.udon, c.masks[0].count_ones());
            }
            assert_eq!(c.masks[0], 1);
        }
        #[test]
        fn free_tail_can_refill_without_losing_a_visit() {
            let mut p = fixture();
            p.map = tester::model::Map {
                width: 4,
                height: 1,
                cells: vec![vec![0, 0, 0, 0]],
            };
            p.spots[0].pos = 0;
            p.agents = vec![0, 1];
            p.day_steps = vec![8, 8];
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 1,
                },
                Agent {
                    kind: 1,
                    pos: 1,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: Some(vec![-8]),
                stops: Vec::new(),
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 8);
            let mut c = Candidate {
                plan: vec![vec![-8], vec![-8]],
                score: 0.0,
                masks: vec![1, 0],
                end: agents.clone(),
                support,
            };
            let mut rng = Rng(113);
            for _ in 0..50 {
                let mut trial = c.clone();
                window::improve(
                    &p, &net, &supply, &agents, &mut trial, &mut rng, 8, 1, 300.0, false,
                );
                if trial.masks[0] == 1 && trial.end[0].fuel > c.end[0].fuel {
                    c = trial;
                }
            }
            assert!(c.end[0].fuel > agents[0].fuel);
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            let replay = game.play_day(c.plan).unwrap();
            assert!(
                replay
                    .frames
                    .iter()
                    .flat_map(|frame| &frame.events)
                    .any(|event| matches!(event, tester::model::Event::Refill { agent: 0, .. }))
            );
            assert_eq!(game.agents[0], c.end[0]);
            assert_eq!(game.score.udon, 1);
        }
        #[test]
        fn retiming_removes_obsolete_wait_and_respects_source_refill() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 0,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: Some(vec![-8]),
                stops: Vec::new(),
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 8);
            let mut old = vec![-5];
            old.extend(
                net.path(1, 0, 0)
                    .moves
                    .iter()
                    .map(|&(_, dir, _, _)| dir as i64),
            );
            old.push(-2);
            let route = window::retime_route(&p, &net, &supply, &agents[0], &old, 8).unwrap();
            assert_eq!(route.plan[0], -1);
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            game.play_day(vec![route.plan, vec![-8]]).unwrap();
            assert_eq!(game.agents[0].fuel, route.fuel as u32);
            assert_eq!(game.score.udon, 1);
        }
        #[test]
        fn direct_supplier_mutations_preserve_valid_geographic_moves() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 4,
                    fuel: 6,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let mut solver = Solver::new(p.clone(), 78);
            let mut route = SupportRoute {
                direct: Some(vec![-8]),
                stops: Vec::new(),
            };
            for _ in 0..200 {
                solver.mutate_direct(&mut route, 0, &net, 8);
                let supply = Supply::build(&p, &agents, &net, &[route.clone()], 8);
                let mut game = Game::new(p.clone(), &[0, 1]);
                game.agents = agents.clone();
                let mut plan = supply.plans;
                plan[0] = vec![-8];
                game.play_day(plan).unwrap();
            }
        }
        #[test]
        fn warm_plan_repair_waits_for_available_fuel() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 0,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: None,
                stops: vec![(2, 8)],
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 8);
            let mut old = net
                .path(1, 0, 0)
                .moves
                .iter()
                .map(|&(_, d, _, _)| d as i64)
                .collect::<Vec<_>>();
            old.push(-7);
            let route = window::repair_route(&p, &net, &supply, &agents[0], &old, 8);
            assert!(route.plan[0] < 0);
            let mut plan = supply.plans.clone();
            plan[0] = route.plan;
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            game.play_day(plan).unwrap();
            assert_eq!(game.score.udon, 1);
            assert_eq!(game.agents[0].pos, route.pos);
            assert_eq!(game.agents[0].fuel, route.fuel as u32);
        }
        #[test]
        fn visit_order_keeps_refill_during_pending_move() {
            let mut p = fixture();
            p.day_steps = vec![2];
            p.spots[0].pos = 2;
            p.agents = vec![1, 0];
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 1,
                    fuel: 0,
                },
                Agent {
                    kind: 1,
                    pos: 0,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: None,
                stops: vec![(1, 2)],
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 2);
            let mut plan = supply.plans.clone();
            plan[0] = net
                .path(1, 0, 0)
                .moves
                .iter()
                .map(|&(_, d, _, _)| d as i64)
                .collect();
            let route = window::replay_route(&p, &net, &supply, &agents[0], &plan[0], 2).unwrap();
            assert_eq!(route.mask, 1);
            let mut end = agents.clone();
            end[0].pos = route.pos;
            end[0].fuel = route.fuel as u32;
            end[1].pos = supply.end[1];
            let mut c = Candidate {
                plan,
                score: 0.0,
                masks: vec![route.mask, 0],
                end,
                support,
            };
            let mut solver = Solver::new(p.clone(), 0);
            tour::improve(&mut solver, &agents, &net, &supply, &mut c, 0, 2, 10);
            assert_eq!(
                solver.tour_stats[1], 1,
                "compression must preserve the in-flight refill timing"
            );
            let mut game = Game::new(p, &[0, 1]);
            game.agents = agents;
            game.play_day(c.plan).unwrap();
            assert_eq!(game.score.udon, 1);
            assert_eq!(game.agents, c.end);
        }
        #[test]
        fn visit_order_search_matches_engine_with_moving_supply() {
            let mut p = fixture();
            p.day_steps = vec![20];
            p.spots.push(tester::model::Spot {
                brand: 1,
                pos: 3,
                stocks: 2,
            });
            p.spots.push(tester::model::Spot {
                brand: 2,
                pos: 6,
                stocks: 2,
            });
            p.agents = vec![0, 7];
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 6,
                },
                Agent {
                    kind: 1,
                    pos: 7,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[]);
            let support = vec![SupportRoute {
                direct: None,
                stops: vec![(0, 2), (2, 4), (1, 20)],
            }];
            let supply = Supply::build(&p, &agents, &net, &support, 20);
            let mut solver = Solver::new(p.clone(), 0);
            let mut c = solver.build_candidate(&agents, &net, &supply, support, 0, 20, 0, 30);
            tour::improve(&mut solver, &agents, &net, &supply, &mut c, 0, 20, 1000);
            let mut game = Game::new(p, &[0, 1]);
            game.play_day(c.plan.clone()).unwrap();
            assert_eq!(game.agents, c.end);
            assert_eq!(game.score.udon, c.masks[0].count_ones());
            assert_eq!(game.score.unique_brands, 3);
        }
        #[test]
        fn path_costs_use_the_departure_cell() {
            let p = fixture();
            let agents = vec![
                Agent {
                    kind: 0,
                    pos: 0,
                    fuel: 6,
                },
                Agent {
                    kind: 1,
                    pos: 7,
                    fuel: 6,
                },
            ];
            let net = Network::new(&p, &agents, &[Traffic { pos: 0, status: 2 }]);
            assert_eq!(net.path(1, 0, 0).time, 4);
            assert_eq!(net.path(1, 0, 0).fuel, 2);
            assert_eq!(net.path(0, 1, 0).time, 2);
            assert_eq!(net.path(0, 1, 0).fuel, 1);
            assert_eq!(net.dist_f[0][0], 2);
        }
    }
}
use std::io::{self, BufRead, Write};
use tester::model::{Request, Response};
fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut solver = None;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            continue;
        };
        let answer = if req.phase == "select" {
            let mut s = itinerary::Solver::new(req.problem.unwrap(), seed);
            let kinds = s.select(req.time_limit_ms);
            solver = Some(s);
            serde_json::json!(kinds)
        } else if let (Some(s), Some(state)) = (&mut solver, req.state) {
            serde_json::json!(s.plan(
                state,
                req.previous_accepted.unwrap_or(true),
                req.time_limit_ms
            ))
        } else {
            serde_json::json!([])
        };
        if serde_json::to_writer(
            &mut out,
            &Response {
                request_id: req.request_id,
                answer,
            },
        )
        .is_err()
        {
            break;
        }
        if writeln!(out).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
