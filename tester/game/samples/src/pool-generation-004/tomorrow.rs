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
        let mut future =
            probe.build_candidate(&current.end, &net, &supply, support, next_day, steps, 0, 65);
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
                    .preserve_patrols(&c, &current.end, &net, &supply, support, next_day, steps)
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
