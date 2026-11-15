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
                        if let Some(c) = self.evaluate(p, metric, ready, order, steps, seen, last) {
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
            if let Some(c) = self.evaluate(p, metric, ready, warm.to_vec(), steps, seen, last) {
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
            if c.value >= current.value || rng.f() < ((c.value - current.value) / temp).exp() {
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
