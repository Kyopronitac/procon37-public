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

fn insertion(tour: &[Visit], node: usize, start: usize, net: &Network, rng: &mut Rng) -> usize {
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
    let (nodes, mut tours) = extract(&solver.p, original_net, original_supply, agents, c, steps);
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
            replacement_supply = Some(Supply::build(&solver.p, agents, &net, &routes, steps));
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
                    let at = insertion(tour, node, solver.p.spots.len() + a, &net, &mut solver.rng);
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
        let rebuilt = Supply::build(&p, &agents, &original, &evaluated.candidate.support, 12);
        assert_eq!(rebuilt.at, supply.at);
        let mut game = tester::engine::Game::new(p, &[0, 1]);
        game.play_day(evaluated.candidate.plan).unwrap();
        assert_eq!(game.agents, evaluated.candidate.end);
    }
}
