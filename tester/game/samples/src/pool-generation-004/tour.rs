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
                        while window_start > 0 && supply.at[window_start as usize - 1][pos] {
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
            let extra = net.path(prev, node, 2).time as i32 + net.path(node, next, 2).time as i32
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
    current.score = solver.local_value(&current.masks, &current.end, &net, supply, day, steps);
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
                        s != old && spot.brand == brand && !proposed.iter().any(|v| v.node == s)
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
