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
                                .clamp(0, u16::MAX as i32) as u16
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
                        for to in p.map.neighbors(pos).filter(|&to| p.map.terrain(to) != 3) {
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
                            q.early +=
                                (limit - q.fuel[a]) as f64 * (steps - k) as f64 / steps as f64;
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
                        let expected = q.fuel[a] - (consumed[future][a] - consumed[tt as usize][a]);
                        let need = (limit as f64 * 0.75 - expected as f64).max(0.0);
                        potential = potential.max(weights[a] * need * 4.0 / (distance + 4.0));
                    }
                }
                let resource = if align_end {
                    let blend = (tt as f64 / steps as f64).powi(2);
                    let mut future_fuel = q.fuel;
                    for &a in &cars {
                        future_fuel[a] -= consumed[steps as usize][a] - consumed[tt as usize][a];
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
