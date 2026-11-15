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
                candidate.score =
                    solver.local_value(&candidate.masks, &candidate.end, net, supply, day, steps);
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
                        + (brands & !solver.seen).count_ones() as f64 * solver.fresh_bonus(day)
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
                candidate.score =
                    solver.local_value(&candidate.masks, &candidate.end, net, supply, day, steps);
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
                    reward(r.route.brands) + r.route.mask.count_ones() as f64 * 10.0 + r.endpoint
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
                let food = (union & remaining[0]).count_ones() + (both & remaining[1]).count_ones();
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
