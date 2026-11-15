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
        .filter(|&j| before.bounds[j].0 > start + 3 && before.bounds[j].0 <= start + horizon)
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
                            super::endpoint_value(p, net, b.pos, t, b.fuel, supply, steps, last)
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
